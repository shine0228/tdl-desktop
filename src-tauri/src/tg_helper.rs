use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use serde::Deserialize;
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::{
    types::{MessageInfo, TgLiteChat, TgLiteStatus},
    util::{apply_hidden_process_flags, run_with_timeout},
};

const STATUS_TIMEOUT: Duration = Duration::from_secs(35);
const LIST_TIMEOUT: Duration = Duration::from_secs(90);
const HELPER_LOCK_RETRIES: usize = 3;
const HELPER_LOCK_RETRY_DELAY: Duration = Duration::from_millis(450);

static HELPER_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct HelperResponse<T> {
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

#[tauri::command]
pub fn tg_helper_status(app: AppHandle) -> Result<TgLiteStatus, String> {
    helper_status(&app)
}

pub fn helper_status(app: &AppHandle) -> Result<TgLiteStatus, String> {
    run_helper_json(app, &["status".to_string()], STATUS_TIMEOUT)
}

#[tauri::command]
pub fn tg_helper_load_chats(app: AppHandle, limit: i64) -> Result<Vec<TgLiteChat>, String> {
    run_helper_json(
        &app,
        &[
            "chats".to_string(),
            "--limit".to_string(),
            limit.clamp(1, 500).to_string(),
        ],
        LIST_TIMEOUT,
    )
}

#[tauri::command]
pub fn tg_helper_load_messages(
    app: AppHandle,
    chat_id: i64,
    limit: i64,
) -> Result<Vec<MessageInfo>, String> {
    run_helper_json(
        &app,
        &[
            "messages".to_string(),
            "--chat-id".to_string(),
            chat_id.to_string(),
            "--limit".to_string(),
            limit.clamp(1, 500).to_string(),
        ],
        LIST_TIMEOUT,
    )
}

fn run_helper_json<T>(app: &AppHandle, args: &[String], timeout: Duration) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = HELPER_PROCESS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "tdl-helper 内部锁已损坏。".to_string())?;

    let mut last_error = None;
    for attempt in 0..HELPER_LOCK_RETRIES {
        match run_helper_json_once(app, args, timeout) {
            Ok(value) => return Ok(value),
            Err(error) if is_database_lock_error(&error) && attempt + 1 < HELPER_LOCK_RETRIES => {
                last_error = Some(error);
                thread::sleep(HELPER_LOCK_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| "tdl-helper 执行失败。".to_string()))
}

fn run_helper_json_once<T>(app: &AppHandle, args: &[String], timeout: Duration) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let helper_path = resolve_helper_path(app)?;
    let mut command = Command::new(&helper_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let output = run_with_timeout(command, timeout)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<HelperResponse<T>>(stdout.trim())
        .map_err(|error| helper_error(&output, &format!("解析 tdl-helper 输出失败: {error}")))?;

    if !parsed.ok || !output.status.success() {
        return Err(parsed
            .error
            .unwrap_or_else(|| helper_error(&output, "tdl-helper 执行失败")));
    }

    parsed
        .result
        .ok_or_else(|| "tdl-helper 没有返回结果。".to_string())
}

pub fn is_database_lock_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("open db: timeout")
        || error.contains("database is used")
        || error.contains("database is locked")
}

fn resolve_helper_path(app: &AppHandle) -> Result<PathBuf, String> {
    let resource = app
        .path()
        .resolve("resources/tdl-helper.exe", BaseDirectory::Resource)
        .ok()
        .filter(|path| path.is_file());
    if let Some(path) = resource {
        return Ok(path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir.join("resources").join("tdl-helper.exe");
    if dev_path.is_file() {
        return Ok(dev_path);
    }

    let local_build = manifest_dir
        .parent()
        .map(|root| {
            root.join("helper")
                .join("tdl-helper")
                .join("tdl-helper.exe")
        })
        .filter(|path| path.is_file());
    if let Some(path) = local_build {
        return Ok(path);
    }

    Err("未找到 tdl-helper.exe，请先运行 npm run build:helper。".into())
}

fn helper_error(output: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = [stderr.trim(), stdout.trim(), fallback]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string();
    message
}
