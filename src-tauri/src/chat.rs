use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::Value;
use tauri::{path::BaseDirectory, AppHandle, Manager, State};

use crate::{
    commands::OperationGuard,
    download::{
        spawn_output_reader, spawn_process_monitor_with_cleanup, ChildProcessGuard,
        PendingHistoryGuard,
    },
    state::AppState,
    tdl::resolve_tdl,
    tdl_config::prepend_tdl_global_args,
    types::{
        AppConfig, ChatDownloadRequest, ChatInfo, ChatMediaPreview, ChatMediaPreviewFile,
        DownloadRecord, DownloadStarted, DownloadStatus, MediaKind, MessageInfo, SourceMode,
    },
    util::{
        apply_hidden_process_flags, lock, preview_command, run_with_timeout, tdl_database_guard,
        tdl_database_guard_for_quick_task, validate_download_dir,
    },
};

const CHAT_LIST_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_EXPORT_TIMEOUT: Duration = Duration::from_secs(90);
const MEDIA_PREVIEW_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone)]
struct PreviewContext {
    app_dir: PathBuf,
    config: Arc<Mutex<AppConfig>>,
    cache_generation: Arc<AtomicU64>,
}

#[tauri::command]
pub fn clear_chat_cache(state: State<'_, AppState>) -> Result<(), String> {
    state.cache_generation.fetch_add(1, Ordering::SeqCst);
    for (path, label) in [
        (state.app_dir.join("previews"), "缩略图缓存"),
        (state.app_dir.join("chat"), "对话缓存"),
    ] {
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("清理{label}失败 ({}): {error}", path.display()))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_chats(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ChatInfo>, String> {
    crate::commands::ensure_tdl_update_not_running(
        &state,
        "tdl 正在更新，请等待更新完成后再读取对话列表。",
    )?;
    let tdl_path = resolve_tdl_path(&app, &state)?;
    let config = lock(&state.config)?.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let args = prepend_tdl_global_args(
            &config,
            vec![
                "chat".to_string(),
                "ls".to_string(),
                "-o".to_string(),
                "json".to_string(),
            ],
        );
        let _guard = tdl_database_guard_for_quick_task()?;
        let output = run_tdl_with_timeout(&tdl_path, &args, CHAT_LIST_TIMEOUT)?;
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("解析对话列表失败: {error}"))?;
        let items = value
            .as_array()
            .ok_or_else(|| "tdl 返回的对话列表格式不正确。".to_string())?;

        Ok(items.iter().filter_map(parse_chat_info).collect())
    })
    .await
    .map_err(|error| format!("读取对话列表失败: {error}"))?
}

#[tauri::command]
pub fn export_chat_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    count: i64,
) -> Result<Vec<MessageInfo>, String> {
    crate::commands::ensure_tdl_update_not_running(
        &state,
        "tdl 正在更新，请等待更新完成后再读取消息。",
    )?;
    let tdl_path = resolve_tdl_path(&app, &state)?;
    let output_path = temp_chat_json(&state, "messages")?;
    let config = lock(&state.config)?.clone();
    let count = count.clamp(1, 500);
    let args = prepend_tdl_global_args(
        &config,
        vec![
            "chat".to_string(),
            "export".to_string(),
            "--with-content".to_string(),
            "--all".to_string(),
            "-T".to_string(),
            "last".to_string(),
            "-c".to_string(),
            chat_id,
            "-i".to_string(),
            count.to_string(),
            "-o".to_string(),
            output_path.to_string_lossy().to_string(),
        ],
    );

    let result = (|| {
        let _guard = tdl_database_guard_for_quick_task()?;
        run_tdl_with_timeout(&tdl_path, &args, CHAT_EXPORT_TIMEOUT)?;
        parse_messages_file(&output_path)
    })();
    let _ = fs::remove_file(&output_path);
    result
}

#[tauri::command]
pub fn download_from_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ChatDownloadRequest,
) -> Result<DownloadStarted, String> {
    let operation_guard = OperationGuard::download(&state)?;
    if request.message_ids.is_empty() {
        return Err("请至少选择一条消息。".into());
    }
    let directory = validate_download_dir(&request.directory, &state.app_dir)?;
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建下载目录: {error}"))?;

    let tdl_path = resolve_tdl_path(&app, &state)?;
    let config = lock(&state.config)?.clone();
    let mut selected_ids = request.message_ids.clone();
    selected_ids.sort_unstable();
    selected_ids.dedup();
    let export_path = export_exact_messages(
        &tdl_path,
        &state,
        &request.chat_id,
        &selected_ids,
        "selected",
        &config,
    )?;

    let download_args = prepend_tdl_global_args(
        &config,
        build_chat_download_args(&request, &directory, &export_path),
    );
    let task_id = state.next_id("task");
    let created_at = Utc::now().to_rfc3339();
    let records = vec![DownloadRecord {
        id: state.next_id("record"),
        task_id: task_id.clone(),
        source: format!("{} · {} 条消息", request.chat_name, selected_ids.len()),
        mode: SourceMode::Chat,
        directory: directory.to_string_lossy().to_string(),
        status: DownloadStatus::Downloading,
        created_at,
        completed_at: None,
        error: None,
    }];
    let record_ids: Vec<String> = records.iter().map(|record| record.id.clone()).collect();

    {
        let mut history = lock(&state.history)?;
        history.splice(0..0, records.clone());
        state.persist_history(&history)?;
    }
    let history_guard = PendingHistoryGuard::new(state.refs(), record_ids.clone());

    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(&download_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let database_guard = tdl_database_guard()?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 下载失败: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    let child_guard = ChildProcessGuard::new(Arc::clone(&child));
    lock(&state.running)?.insert(task_id.clone(), Arc::clone(&child));
    drop(operation_guard);

    if let Some(stream) = stdout {
        spawn_output_reader(app.clone(), task_id.clone(), stream);
    }
    if let Some(stream) = stderr {
        spawn_output_reader(app.clone(), task_id.clone(), stream);
    }

    spawn_process_monitor_with_cleanup(
        app,
        state.refs(),
        task_id.clone(),
        record_ids,
        child,
        Some(export_path),
        database_guard,
    );
    child_guard.disarm();
    history_guard.disarm();

    Ok(DownloadStarted {
        task_id,
        command_preview: preview_command(&tdl_path.to_string_lossy(), &download_args),
        records,
    })
}

#[tauri::command]
pub async fn preview_chat_media(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    message_id: i64,
) -> Result<ChatMediaPreview, String> {
    crate::commands::ensure_tdl_update_not_running(
        &state,
        "tdl 正在更新，请等待更新完成后再加载预览。",
    )?;
    let context = PreviewContext {
        app_dir: state.app_dir.clone(),
        config: Arc::clone(&state.config),
        cache_generation: Arc::clone(&state.cache_generation),
    };
    tauri::async_runtime::spawn_blocking(move || {
        preview_chat_media_blocking(app, context, chat_id, message_id)
    })
    .await
    .map_err(|error| format!("预览任务执行失败: {error}"))?
}

fn preview_chat_media_blocking(
    app: AppHandle,
    context: PreviewContext,
    chat_id: String,
    message_id: i64,
) -> Result<ChatMediaPreview, String> {
    let generation = context.cache_generation.load(Ordering::SeqCst);
    let tdl_path = resolve_tdl_path_for_preview(&app, &context)?;
    ensure_preview_generation(&context, generation)?;
    let preview_dir = media_preview_dir_for(&context.app_dir, &chat_id, message_id);
    fs::create_dir_all(&preview_dir).map_err(|error| format!("无法创建媒体预览目录: {error}"))?;

    let mut files = collect_preview_files(&preview_dir)?;
    if files.is_empty() {
        let config = lock(&context.config)?.clone();
        let export_path = export_exact_messages_in_dir(
            &tdl_path,
            &context.app_dir,
            &chat_id,
            &[message_id],
            "preview",
            &config,
            Some((&context.cache_generation, generation)),
        )?;
        let args = prepend_tdl_global_args(
            &config,
            vec![
                "download".to_string(),
                "-d".to_string(),
                preview_dir.to_string_lossy().to_string(),
                "-l".to_string(),
                "1".to_string(),
                "-t".to_string(),
                "1".to_string(),
                "-f".to_string(),
                export_path.to_string_lossy().to_string(),
                "--skip-same".to_string(),
                "--pool".to_string(),
                "1".to_string(),
                "--template".to_string(),
                "{{ .MessageID }}_{{ filenamify .FileName }}".to_string(),
            ],
        );

        let result = (|| {
            let _guard = tdl_database_guard_for_quick_task()?;
            run_tdl_with_timeout_for_generation(
                &tdl_path,
                &args,
                MEDIA_PREVIEW_TIMEOUT,
                &context.cache_generation,
                generation,
            )
        })();
        let _ = fs::remove_file(&export_path);
        match result {
            Ok(_) => ensure_preview_generation(&context, generation)?,
            Err(error) => {
                if context.cache_generation.load(Ordering::SeqCst) != generation {
                    let _ = fs::remove_dir_all(&preview_dir);
                }
                return Err(error);
            }
        }
        files = collect_preview_files(&preview_dir)?;
    }

    if files.is_empty() {
        return Err("这条消息没有可预览的媒体文件。".into());
    }

    Ok(ChatMediaPreview {
        chat_id,
        message_id,
        files,
    })
}

#[tauri::command]
pub async fn cached_chat_media_preview(
    state: State<'_, AppState>,
    chat_id: String,
    message_id: i64,
) -> Result<Option<ChatMediaPreview>, String> {
    let app_dir = state.app_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        cached_chat_media_preview_blocking(app_dir, chat_id, message_id)
    })
    .await
    .map_err(|error| format!("读取预览缓存失败: {error}"))?
}

fn cached_chat_media_preview_blocking(
    app_dir: PathBuf,
    chat_id: String,
    message_id: i64,
) -> Result<Option<ChatMediaPreview>, String> {
    let preview_dir = media_preview_dir_for(&app_dir, &chat_id, message_id);
    let files = collect_preview_files(&preview_dir)?;
    if files.is_empty() {
        return Ok(None);
    }

    Ok(Some(ChatMediaPreview {
        chat_id,
        message_id,
        files,
    }))
}

fn resolve_tdl_path(app: &AppHandle, state: &AppState) -> Result<PathBuf, String> {
    let tdl = resolve_tdl(app, state)?;
    if !tdl.available {
        return Err("未找到可用的 tdl.exe。".into());
    }
    tdl.path
        .map(PathBuf::from)
        .ok_or_else(|| "tdl 路径不可用".to_string())
}

fn resolve_tdl_path_for_preview(
    app: &AppHandle,
    context: &PreviewContext,
) -> Result<PathBuf, String> {
    if let Some(path) = lock(&context.config)?.tdl_override_path.clone() {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(path) = bundled_tdl_path(app) {
        return Ok(path);
    }

    let local_path = context.app_dir.join("bin").join("tdl.exe");
    if local_path.is_file() {
        return Ok(local_path);
    }

    find_tdl_in_path().ok_or_else(|| "未找到可用的 tdl.exe。".to_string())
}

fn bundled_tdl_path(app: &AppHandle) -> Option<PathBuf> {
    let resource = app
        .path()
        .resolve("resources/tdl.exe", BaseDirectory::Resource)
        .ok();

    resource.filter(|path| path.is_file()).or_else(|| {
        let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("tdl.exe");
        dev_path.is_file().then_some(dev_path)
    })
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

fn run_tdl_with_timeout(
    tdl_path: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut command = Command::new(tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let output = run_with_timeout(command, timeout)?;
    ensure_tdl_success(output)
}

fn run_tdl_with_timeout_for_generation(
    tdl_path: &Path,
    args: &[String],
    timeout: Duration,
    generation_source: &AtomicU64,
    generation: u64,
) -> Result<std::process::Output, String> {
    let mut command = Command::new(tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let output = run_with_timeout_until(command, timeout, || {
        generation_source.load(Ordering::SeqCst) != generation
    })?;
    ensure_tdl_success(output)
}

fn ensure_tdl_success(output: std::process::Output) -> Result<std::process::Output, String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = first_non_empty(&[stderr.trim(), stdout.trim()]).unwrap_or("tdl 执行失败");
        return Err(compact_message(detail));
    }
    Ok(output)
}

fn run_with_timeout_until(
    mut command: Command,
    timeout: Duration,
    should_cancel: impl Fn() -> bool,
) -> Result<std::process::Output, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动进程失败: {error}"))?;
    let started = Instant::now();

    loop {
        if should_cancel() {
            let _ = child.kill();
            let _ = child.wait_with_output();
            return Err("预览缓存已清理，已取消旧预览任务。".into());
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("读取进程输出失败: {error}"))
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(120));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait_with_output();
                return Err("操作超时，请确认已登录 tdl 且网络可用。".into());
            }
            Err(error) => {
                let _ = child.kill();
                return Err(format!("检查进程状态失败: {error}"));
            }
        }
    }
}

fn ensure_preview_generation(context: &PreviewContext, generation: u64) -> Result<(), String> {
    if context.cache_generation.load(Ordering::SeqCst) != generation {
        return Err("预览缓存已清理，已取消旧预览任务。".into());
    }
    Ok(())
}

fn temp_chat_json(state: &AppState, prefix: &str) -> Result<PathBuf, String> {
    temp_chat_json_in_dir(&state.app_dir, &state.next_id(prefix))
}

fn temp_chat_json_in_dir(app_dir: &Path, prefix: &str) -> Result<PathBuf, String> {
    let dir = app_dir.join("chat");
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建对话缓存目录: {error}"))?;
    Ok(dir.join(format!("{}-{}.json", prefix, Utc::now().timestamp_millis())))
}

fn media_preview_dir_for(app_dir: &Path, chat_id: &str, message_id: i64) -> PathBuf {
    app_dir
        .join("previews")
        .join(sanitize_file_name(chat_id))
        .join(message_id.to_string())
}

fn collect_preview_files(directory: &Path) -> Result<Vec<ChatMediaPreviewFile>, String> {
    let mut files = Vec::new();
    collect_preview_files_inner(directory, &mut files)?;
    files.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(files)
}

fn collect_preview_files_inner(
    directory: &Path,
    files: &mut Vec<ChatMediaPreviewFile>,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(directory).map_err(|error| format!("读取预览目录失败: {error}"))?
    {
        let entry = entry.map_err(|error| format!("读取预览文件失败: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_preview_files_inner(&path, files)?;
            continue;
        }
        if !path.is_file() || is_transient_file(&path) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("media")
            .to_string();
        let mime_type = mime_from_extension(&file_name);
        let media_kind =
            detect_media_kind(&Value::Null, Some(&file_name), mime_type.as_deref(), None);
        if matches!(media_kind, MediaKind::None | MediaKind::Unknown) {
            continue;
        }
        let size = path
            .metadata()
            .ok()
            .and_then(|metadata| i64::try_from(metadata.len()).ok());
        files.push(ChatMediaPreviewFile {
            path: path.to_string_lossy().to_string(),
            file_name,
            media_kind,
            mime_type,
            size,
        });
    }

    Ok(())
}

fn is_transient_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".json")
        || name.ends_with(".tmp")
        || name.ends_with(".part")
        || name.ends_with(".downloading")
}

fn sanitize_file_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn export_exact_messages(
    tdl_path: &Path,
    state: &AppState,
    chat_id: &str,
    message_ids: &[i64],
    prefix: &str,
    config: &AppConfig,
) -> Result<PathBuf, String> {
    export_exact_messages_with_output(
        tdl_path,
        temp_chat_json(state, prefix)?,
        chat_id,
        message_ids,
        config,
        None,
    )
}

fn export_exact_messages_in_dir(
    tdl_path: &Path,
    app_dir: &Path,
    chat_id: &str,
    message_ids: &[i64],
    prefix: &str,
    config: &AppConfig,
    generation: Option<(&AtomicU64, u64)>,
) -> Result<PathBuf, String> {
    let prefix = format!(
        "{prefix}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    export_exact_messages_with_output(
        tdl_path,
        temp_chat_json_in_dir(app_dir, &prefix)?,
        chat_id,
        message_ids,
        config,
        generation,
    )
}

fn export_exact_messages_with_output(
    tdl_path: &Path,
    output_path: PathBuf,
    chat_id: &str,
    message_ids: &[i64],
    config: &AppConfig,
    generation: Option<(&AtomicU64, u64)>,
) -> Result<PathBuf, String> {
    if message_ids.is_empty() {
        return Err("请至少选择一条消息。".into());
    }
    let mut selected_ids = message_ids.to_vec();
    selected_ids.sort_unstable();
    selected_ids.dedup();
    let selected_set: HashSet<i64> = selected_ids.iter().copied().collect();
    let first_id = selected_ids[0];
    let last_id = selected_ids[selected_ids.len() - 1];
    let export_args = prepend_tdl_global_args(
        config,
        vec![
            "chat".to_string(),
            "export".to_string(),
            "--with-content".to_string(),
            "--all".to_string(),
            "-T".to_string(),
            "id".to_string(),
            "-c".to_string(),
            chat_id.to_string(),
            "-i".to_string(),
            format!("{first_id},{last_id}"),
            "-o".to_string(),
            output_path.to_string_lossy().to_string(),
        ],
    );

    {
        let _guard = tdl_database_guard_for_quick_task()?;
        if let Some((generation_source, expected_generation)) = generation {
            run_tdl_with_timeout_for_generation(
                tdl_path,
                &export_args,
                CHAT_EXPORT_TIMEOUT,
                generation_source,
                expected_generation,
            )?;
        } else {
            run_tdl_with_timeout(tdl_path, &export_args, CHAT_EXPORT_TIMEOUT)?;
        }
    }
    filter_exported_messages(&output_path, &selected_set)?;
    Ok(output_path)
}

fn build_chat_download_args(
    request: &ChatDownloadRequest,
    directory: &Path,
    file: &Path,
) -> Vec<String> {
    let mut args = vec![
        "download".to_string(),
        "-d".to_string(),
        directory.to_string_lossy().to_string(),
        "-l".to_string(),
        request.limit.max(1).to_string(),
        "-t".to_string(),
        request.threads.max(1).to_string(),
    ];
    if request.pool > 0 {
        args.extend(["--pool".to_string(), request.pool.to_string()]);
    }
    args.extend(["-f".to_string(), file.to_string_lossy().to_string()]);
    if request.group {
        args.push("--group".to_string());
    }
    push_csv_arg(&mut args, "-i", &request.include);
    push_csv_arg(&mut args, "-e", &request.exclude);
    if !request.template.trim().is_empty() {
        args.extend([
            "--template".to_string(),
            request.template.trim().to_string(),
        ]);
    }
    if request.skip_same {
        args.push("--skip-same".to_string());
    }
    if request.continue_last {
        args.push("--continue".to_string());
    }
    if request.restart {
        args.push("--restart".to_string());
    }
    if request.desc {
        args.push("--desc".to_string());
    }
    if request.takeout {
        args.push("--takeout".to_string());
    }
    if request.rewrite_ext {
        args.push("--rewrite-ext".to_string());
    }
    args
}

fn push_csv_arg(args: &mut Vec<String>, flag: &str, value: &str) {
    let value = value.trim().trim_matches(',');
    if !value.is_empty() {
        args.extend([flag.to_string(), value.to_string()]);
    }
}

fn parse_chat_info(value: &Value) -> Option<ChatInfo> {
    let id = get_i64(value, &["id", "ID"])?;
    let chat_type = get_string(value, &["type", "chat_type", "chatType"]).unwrap_or_default();
    let name = get_string(value, &["visible_name", "visibleName", "name", "title"])
        .unwrap_or_else(|| format!("{} {id}", chat_type_label(&chat_type)));
    Some(ChatInfo {
        id,
        name,
        chat_type,
        username: get_string(value, &["username", "user_name", "userName"]),
    })
}

fn chat_type_label(chat_type: &str) -> &'static str {
    match chat_type.to_ascii_lowercase().as_str() {
        "channel" => "Channel",
        "group" | "supergroup" => "Group",
        "private" | "user" => "User",
        "bot" => "Bot",
        _ => "Chat",
    }
}

fn parse_messages_file(path: &Path) -> Result<Vec<MessageInfo>, String> {
    let content = fs::read_to_string(path).map_err(|error| format!("读取消息列表失败: {error}"))?;
    let value: Value =
        serde_json::from_str(&content).map_err(|error| format!("解析消息列表失败: {error}"))?;
    let messages =
        find_messages(&value).ok_or_else(|| "tdl 导出结果里没有 messages 数组。".to_string())?;
    Ok(messages.iter().filter_map(parse_message_info).collect())
}

fn filter_exported_messages(path: &Path, selected_ids: &HashSet<i64>) -> Result<(), String> {
    let content =
        fs::read_to_string(path).map_err(|error| format!("读取已导出的消息失败: {error}"))?;
    let mut value: Value =
        serde_json::from_str(&content).map_err(|error| format!("解析已导出的消息失败: {error}"))?;
    let kept = filter_messages_in_value(&mut value, selected_ids)?;
    if kept == 0 {
        return Err("tdl 已导出消息范围，但没有找到已勾选的消息。".into());
    }
    let content = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("序列化已筛选消息失败: {error}"))?;
    fs::write(path, content).map_err(|error| format!("写入已筛选消息失败: {error}"))
}

fn filter_messages_in_value(
    value: &mut Value,
    selected_ids: &HashSet<i64>,
) -> Result<usize, String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(messages)) = map.get_mut("messages") {
                messages.retain(|message| {
                    message_id(message).is_some_and(|id| selected_ids.contains(&id))
                });
                return Ok(messages.len());
            }
            for child in map.values_mut() {
                let kept = filter_messages_in_value(child, selected_ids)?;
                if kept > 0 {
                    return Ok(kept);
                }
            }
            Ok(0)
        }
        Value::Array(items) => {
            items
                .retain(|message| message_id(message).is_some_and(|id| selected_ids.contains(&id)));
            Ok(items.len())
        }
        _ => Err("tdl 导出结果里没有可筛选的 messages 数组。".into()),
    }
}

fn message_id(value: &Value) -> Option<i64> {
    get_i64(
        value,
        &[
            "id",
            "ID",
            "message_id",
            "messageId",
            "MessageID",
            "MessageId",
        ],
    )
}

fn find_messages(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(messages)) = map.get("messages") {
                return Some(messages);
            }
            map.values().find_map(find_messages)
        }
        Value::Array(items) => Some(items),
        _ => None,
    }
}

fn parse_message_info(value: &Value) -> Option<MessageInfo> {
    let id = message_id(value)?;
    let file_name = find_file_name(value);
    let mime_type =
        find_mime_type(value).or_else(|| file_name.as_deref().and_then(mime_from_extension));
    let media_type = find_media_type(value);
    let media_kind = detect_media_kind(
        value,
        file_name.as_deref(),
        mime_type.as_deref(),
        media_type.as_deref(),
    );
    Some(MessageInfo {
        id,
        date: get_date_string(value, &["date", "Date", "created_at", "createdAt"]),
        text: find_text(value).map(|text| compact_message(&text)),
        media_kind,
        media_type,
        mime_type,
        file_name,
        file_size: find_file_size(value),
        width: find_dimension(value, &["width", "Width", "w"]),
        height: find_dimension(value, &["height", "Height", "h"]),
        duration: find_dimension(
            value,
            &[
                "duration",
                "Duration",
                "duration_seconds",
                "durationSeconds",
            ],
        ),
        previewable: matches!(media_kind, MediaKind::Photo | MediaKind::Video),
    })
}

fn find_text(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &["text", "message", "caption", "content", "rawText"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    let text = text.trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            map.values().find_map(find_text)
        }
        Value::Array(items) => items.iter().find_map(find_text),
        _ => None,
    }
}

fn find_media_type(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &["media", "class_name", "className", "_"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    if is_specific_media_type(text) {
                        return Some(text.trim().to_string());
                    }
                }
            }
            map.values().find_map(find_media_type)
        }
        Value::Array(items) => items.iter().find_map(find_media_type),
        _ => None,
    }
}

fn find_mime_type(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "mime_type",
        "mimeType",
        "mime",
        "content_type",
        "contentType",
    ];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    if !text.trim().is_empty() {
                        return Some(text.trim().to_ascii_lowercase());
                    }
                }
            }
            map.values().find_map(find_mime_type)
        }
        Value::Array(items) => items.iter().find_map(find_mime_type),
        _ => None,
    }
}

fn find_file_name(value: &Value) -> Option<String> {
    const DIRECT_KEYS: &[&str] = &["file_name", "fileName", "filename", "FileName", "file"];
    const NESTED_KEYS: &[&str] = &["file_name", "fileName", "filename", "FileName"];
    find_file_name_inner(value, DIRECT_KEYS, true)
        .or_else(|| find_file_name_inner(value, NESTED_KEYS, false))
}

fn find_file_name_inner(
    value: &Value,
    keys: &[&str],
    allow_current_file_key: bool,
) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if !allow_current_file_key && *key == "file" {
                    continue;
                }
                if let Some(Value::String(text)) = map.get(*key) {
                    let text = text.trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            map.values()
                .find_map(|child| find_file_name_inner(child, keys, false))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_file_name_inner(child, keys, false)),
        _ => None,
    }
}

fn find_file_size(value: &Value) -> Option<i64> {
    const KEYS: &[&str] = &["file_size", "fileSize", "FileSize", "size", "Size", "bytes"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(size) = map.get(*key).and_then(value_to_i64) {
                    return Some(size);
                }
            }
            map.values().find_map(find_file_size)
        }
        Value::Array(items) => items.iter().find_map(find_file_size),
        _ => None,
    }
}

fn find_dimension(value: &Value, keys: &[&str]) -> Option<i64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(size) = map.get(*key).and_then(value_to_i64) {
                    return Some(size);
                }
            }
            map.values().find_map(|child| find_dimension(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| find_dimension(child, keys)),
        _ => None,
    }
}

fn detect_media_kind(
    value: &Value,
    file_name: Option<&str>,
    mime_type: Option<&str>,
    media_type: Option<&str>,
) -> MediaKind {
    if let Some(mime) = mime_type {
        if mime.starts_with("image/") {
            return MediaKind::Photo;
        }
        if mime.starts_with("video/") {
            return MediaKind::Video;
        }
        if mime.starts_with("audio/") {
            return MediaKind::Audio;
        }
    }

    if let Some(kind) = media_type.and_then(kind_from_text) {
        return kind;
    }

    if let Some(name) = file_name {
        if let Some(kind) = kind_from_extension(name) {
            return kind;
        }
        return MediaKind::Document;
    }

    if has_key_recursive(value, &["photo", "Photo"]) {
        return MediaKind::Photo;
    }
    if has_key_recursive(value, &["video", "Video"]) {
        return MediaKind::Video;
    }
    if has_key_recursive(value, &["audio", "Audio", "voice", "Voice"]) {
        return MediaKind::Audio;
    }
    if has_key_recursive(value, &["document", "Document"]) {
        return MediaKind::Document;
    }

    MediaKind::None
}

fn kind_from_text(text: &str) -> Option<MediaKind> {
    let text = text.to_ascii_lowercase();
    if text.contains("photo") || text.contains("image") {
        Some(MediaKind::Photo)
    } else if text.contains("video") {
        Some(MediaKind::Video)
    } else if text.contains("audio") || text.contains("voice") {
        Some(MediaKind::Audio)
    } else if text.contains("document") || text.contains("file") {
        Some(MediaKind::Document)
    } else {
        None
    }
}

fn kind_from_extension(file_name: &str) -> Option<MediaKind> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "heic" | "heif" => Some(MediaKind::Photo),
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "wmv" => Some(MediaKind::Video),
        "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "flac" => Some(MediaKind::Audio),
        "" => None,
        _ => Some(MediaKind::Document),
    }
}

fn mime_from_extension(file_name: &str) -> Option<String> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "heic" | "heif" => "image/heif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/aac",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => return None,
    };
    Some(mime.to_string())
}

fn has_key_recursive(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Object(map) => {
            keys.iter().any(|key| map.contains_key(*key))
                || map.values().any(|child| has_key_recursive(child, keys))
        }
        Value::Array(items) => items.iter().any(|child| has_key_recursive(child, keys)),
        _ => false,
    }
}

fn is_specific_media_type(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && !matches!(text, "message" | "Message")
}

fn get_date_string(value: &Value, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Value::Number(number) => number.as_i64().map(|timestamp| {
                if timestamp > 10_000_000_000 {
                    chrono::DateTime::from_timestamp_millis(timestamp)
                } else {
                    chrono::DateTime::from_timestamp(timestamp, 0)
                }
                .map(|date| date.to_rfc3339())
                .unwrap_or_else(|| timestamp.to_string())
            }),
            _ => None,
        })
    })
}

fn get_string(value: &Value, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            _ => None,
        })
    })
}

fn get_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    let Value::Object(map) = value else {
        return None;
    };
    keys.iter()
        .find_map(|key| map.get(*key).and_then(value_to_i64))
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|v| i64::try_from(v).ok())),
        Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn first_non_empty<'a>(values: &[&'a str]) -> Option<&'a str> {
    values
        .iter()
        .copied()
        .find(|value| !value.trim().is_empty())
}

fn compact_message(message: &str) -> String {
    let mut compact = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() > 160 {
        compact = compact.chars().take(160).collect::<String>();
        compact.push_str("...");
    }
    compact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_exported_messages_to_selected_ids() {
        let mut value = serde_json::json!({
            "id": 100,
            "messages": [
                { "id": 4853, "file": "a.mp4" },
                { "id": 4854, "file": "middle.mp4" },
                { "id": 4855, "file": "b.jpg" }
            ]
        });
        let selected = HashSet::from([4853, 4855]);

        let kept = filter_messages_in_value(&mut value, &selected).unwrap();
        let ids = value["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message_id(message).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(kept, 2);
        assert_eq!(ids, vec![4853, 4855]);
    }

    #[test]
    fn parses_file_field_as_video_media() {
        let value = serde_json::json!({
            "id": 4853,
            "type": "message",
            "file": "6069126377972440177.mp4",
            "date": 1778955294,
            "text": "caption"
        });

        let message = parse_message_info(&value).unwrap();

        assert_eq!(message.media_kind, MediaKind::Video);
        assert_eq!(
            message.file_name.as_deref(),
            Some("6069126377972440177.mp4")
        );
        assert_eq!(message.mime_type.as_deref(), Some("video/mp4"));
        assert!(message.previewable);
        assert!(message.date.is_some());
    }

    #[test]
    fn ignores_generic_message_type_for_text_messages() {
        let value = serde_json::json!({
            "id": 1,
            "type": "message",
            "text": "hello"
        });

        let message = parse_message_info(&value).unwrap();

        assert_eq!(message.media_kind, MediaKind::None);
        assert_eq!(message.media_type, None);
        assert!(!message.previewable);
    }

    #[test]
    fn ignores_sender_names_and_titles_for_text_messages() {
        let value = serde_json::json!({
            "id": 2,
            "type": "message",
            "from": "Alice",
            "from_id": "user123",
            "text": "just text",
            "reply_to_message": {
                "id": 1,
                "name": "Bob",
                "title": "General chat",
                "file": null
            }
        });

        let message = parse_message_info(&value).unwrap();

        assert_eq!(message.media_kind, MediaKind::None);
        assert_eq!(message.file_name, None);
        assert!(!message.previewable);
    }
}
