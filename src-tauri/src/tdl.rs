use std::{
    env, fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::{
    state::AppState,
    types::{GitHubRelease, TdlInfo, TdlSource},
    util::{apply_hidden_process_flags, lock},
};

const TDL_RELEASE_SOURCES: &[&str] = &[
    "https://api.github.com/repos/iyear/tdl/releases/latest",
];
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

pub fn resolve_tdl(app: &AppHandle, state: &AppState) -> Result<TdlInfo, String> {
    let override_path = lock(&state.config)?.tdl_override_path.clone();
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        if let Some(version) = get_tdl_version(&path) {
            return Ok(tdl_info(
                true,
                Some(version),
                Some(path),
                TdlSource::Updated,
            ));
        }
    }

    if let Some(path) = bundled_tdl_path(app) {
        if let Some(version) = get_tdl_version(&path) {
            return Ok(tdl_info(
                true,
                Some(version),
                Some(path),
                TdlSource::Bundled,
            ));
        }
    }

    let local_path = state.local_tdl_path();
    if let Some(version) = get_tdl_version(&local_path) {
        return Ok(tdl_info(
            true,
            Some(version),
            Some(local_path),
            TdlSource::Updated,
        ));
    }

    if let Some(path) = find_tdl_in_path() {
        if let Some(version) = get_tdl_version(&path) {
            return Ok(tdl_info(true, Some(version), Some(path), TdlSource::Path));
        }
    }

    Ok(tdl_info(false, None, None, TdlSource::Missing))
}

pub fn update_tdl(app: &AppHandle, state: &AppState) -> Result<TdlInfo, String> {
    let asset_name = windows_asset_name();
    let client = reqwest::blocking::Client::builder()
        .user_agent("TDL-Desktop")
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_TOTAL_TIMEOUT)
        .build()
        .map_err(|error| format!("无法创建下载客户端: {error}"))?;

    let release = fetch_latest_release(&client)?;

    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("未找到适配当前架构的 tdl 包: {asset_name}"))?;

    let expected_digest = expected_sha256(&asset)?;
    let bytes = client
        .get(asset.browser_download_url)
        .send()
        .map_err(|error| format!("下载 tdl 失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("下载 tdl 失败: {error}"))?
        .bytes()
        .map_err(|error| format!("读取 tdl 下载内容失败: {error}"))?;
    verify_sha256(bytes.as_ref(), &expected_digest, &asset.name)?;

    // 持有 running 锁直到替换完成，避免在替换 tdl.exe 时被新启动的下载任务占用。
    let running = lock(&state.running)?;
    if !running.is_empty() {
        return Err("当前还有下载任务在执行，请等任务结束或取消后再更新 tdl。".into());
    }

    let tdl_path = state.local_tdl_path();
    if let Some(parent) = tdl_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建 tdl 目录: {error}"))?;
    }

    // 先写到临时文件再原子替换，即便写入过程中目标文件被占用也不会留下半成品。
    let tmp_path = tdl_path.with_extension("exe.tmp");
    extract_tdl_exe(bytes.as_ref(), &tmp_path)?;
    if get_tdl_version(&tmp_path).is_none() {
        let _ = fs::remove_file(&tmp_path);
        return Err("tdl 已下载，但无法运行，可能是架构不匹配。".into());
    }
    fs::rename(&tmp_path, &tdl_path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!("替换 tdl.exe 失败 (可能正被占用): {error}")
    })?;
    drop(running);

    {
        let mut config = lock(&state.config)?;
        config.tdl_override_path = Some(tdl_path.to_string_lossy().to_string());
        state.persist_config(&config)?;
    }

    resolve_tdl(app, state)
}

fn fetch_latest_release(client: &reqwest::blocking::Client) -> Result<GitHubRelease, String> {
    let mut sources: Vec<&str> = TDL_RELEASE_SOURCES.to_vec();
    if let Ok(mirror) = env::var("TDL_MIRROR") {
        if !mirror.is_empty() {
            eprintln!("[tdl] TDL_MIRROR 环境变量已设置，前置镜像源: {mirror}");
            // Leak to get &'static str — only runs once at startup, acceptable cost.
            let mirror_boxed: &'static str = Box::leak(mirror.into_boxed_str());
            sources.insert(0, mirror_boxed);
        }
    }

    let mut last_error = String::new();
    for source in &sources {
        eprintln!("[tdl] 尝试获取 tdl 最新版本: {source}");
        match client.get(*source).send() {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => match resp.json::<GitHubRelease>() {
                    Ok(release) => {
                        eprintln!("[tdl] 成功从 {source} 获取版本信息");
                        return Ok(release);
                    }
                    Err(error) => {
                        last_error = format!("无法解析版本信息 ({source}): {error}");
                        eprintln!("[tdl] {last_error}，尝试下一个源...");
                    }
                },
                Err(error) => {
                    last_error = format!("GitHub 返回错误 ({source}): {error}");
                    eprintln!("[tdl] {last_error}，尝试下一个源...");
                }
            },
            Err(error) => {
                last_error = format!("无法连接 ({source}): {error}");
                eprintln!("[tdl] {last_error}，尝试下一个源...");
            }
        }
    }

    Err(format!("所有 tdl 下载源均失败。最后一个错误: {last_error}"))
}

pub fn get_tdl_version(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }

    let mut command = Command::new(path);
    apply_hidden_process_flags(&mut command);
    let output = command.arg("version").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(version) = line.trim().strip_prefix("Version:") {
            return Some(version.trim().to_string());
        }
    }

    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn bundled_tdl_path(app: &AppHandle) -> Option<PathBuf> {
    let resource = app
        .path()
        .resolve("resources/tdl.exe", BaseDirectory::Resource)
        .ok();

    resource
        .filter(|path| path.exists())
        .or_else(portable_tdl_path)
        .or_else(|| {
            let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join("tdl.exe");
            dev_path.exists().then_some(dev_path)
        })
}

fn portable_tdl_path() -> Option<PathBuf> {
    let path = env::current_exe()
        .ok()?
        .parent()?
        .join("resources")
        .join("tdl.exe");
    path.exists().then_some(path)
}

fn find_tdl_in_path() -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let candidates = if cfg!(windows) {
        vec!["tdl.exe"]
    } else {
        vec!["tdl"]
    };

    for dir in env::split_paths(&path_var) {
        for candidate in &candidates {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn tdl_info(
    available: bool,
    version: Option<String>,
    path: Option<PathBuf>,
    source: TdlSource,
) -> TdlInfo {
    TdlInfo {
        available,
        version,
        path: path.map(|path| path.to_string_lossy().to_string()),
        source,
    }
}

fn windows_asset_name() -> String {
    let suffix = match env::consts::ARCH {
        "x86_64" => "64bit",
        "x86" => "32bit",
        "aarch64" => "arm64",
        "arm" => "armv7",
        _ => "64bit",
    };
    format!("tdl_Windows_{suffix}.zip")
}

fn expected_sha256(asset: &crate::types::GitHubAsset) -> Result<String, String> {
    let digest = asset
        .digest
        .as_deref()
        .ok_or_else(|| format!("tdl 发布资产 {} 未提供 SHA-256 摘要。", asset.name))?;
    let value = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("tdl 发布资产 {} 的摘要不是 SHA-256。", asset.name))?
        .trim()
        .to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "tdl 发布资产 {} 的 SHA-256 摘要格式无效。",
            asset.name
        ));
    }
    Ok(value)
}

fn verify_sha256(data: &[u8], expected: &str, label: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(data));
    if actual != expected {
        return Err(format!(
            "tdl 下载包 SHA-256 校验失败 ({label})。期望 {expected}，实际 {actual}。"
        ));
    }
    Ok(())
}

fn extract_tdl_exe(data: &[u8], destination: &Path) -> Result<(), String> {
    let cursor = Cursor::new(data.to_vec());
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|error| format!("解压 tdl 失败: {error}"))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| format!("读取 tdl 压缩包失败: {error}"))?;
        let name = file.name().replace('\\', "/").to_lowercase();
        if !name.ends_with("tdl.exe") {
            continue;
        }

        let mut output =
            fs::File::create(destination).map_err(|error| format!("写入 tdl.exe 失败: {error}"))?;
        std::io::copy(&mut file, &mut output)
            .map_err(|error| format!("写入 tdl.exe 失败: {error}"))?;
        return Ok(());
    }

    Err("tdl 压缩包中未找到 tdl.exe。".into())
}
