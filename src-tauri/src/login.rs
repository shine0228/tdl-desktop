use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::{ensure_tdl_update_not_running, OperationGuard},
    state::AppState,
    tdl::resolve_tdl,
    tdl_config::{prepend_tdl_global_args, session_targets},
    types::{
        AppConfig, LoginEvent, LoginEventKind, LoginMethod, LoginRequest, LoginResultStatus,
        LoginStarted, LoginStatus, LoginStatusRequest,
    },
    util::{
        apply_hidden_process_flags, lock, run_with_timeout, strip_ansi, tdl_database_guard,
        tdl_database_guard_for_quick_task, TdlDatabaseGuard,
    },
};

const LOGIN_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[tauri::command]
pub async fn check_login_status(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Option<LoginStatusRequest>,
) -> Result<LoginStatus, String> {
    ensure_tdl_update_not_running(&state, "tdl 正在更新，请等待更新完成后再检查登录状态。")?;
    let tdl = resolve_tdl(&app, &state)?;
    if !tdl.available {
        return Ok(login_status(
            false,
            "tdl 不可用，无法检查登录状态。",
            None,
            None,
            None,
        ));
    }

    let config = lock(&state.config)?.clone();
    if !tdl_session_exists(&config)? {
        return Ok(login_status(
            false,
            "tdl 尚未登录 Telegram。",
            None,
            None,
            None,
        ));
    }

    if !request.is_some_and(|request| request.verify_online) {
        return Ok(login_status(
            true,
            "本机存在 tdl Telegram 登录态。",
            None,
            None,
            None,
        ));
    }

    let tdl_path = PathBuf::from(
        tdl.path
            .ok_or_else(|| "tdl 路径不可用，无法检查登录状态。".to_string())?,
    );

    tauri::async_runtime::spawn_blocking(move || check_login_status_with_tdl(tdl_path, config))
        .await
        .map_err(|error| format!("检查 Telegram 登录状态失败: {error}"))?
}

fn check_login_status_with_tdl(
    tdl_path: PathBuf,
    config: AppConfig,
) -> Result<LoginStatus, String> {
    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    let args = prepend_tdl_global_args(
        &config,
        vec![
            "chat".to_string(),
            "ls".to_string(),
            "-o".to_string(),
            "json".to_string(),
            "-f".to_string(),
            "false".to_string(),
        ],
    );
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let _database_guard = tdl_database_guard_for_quick_task()?;
    let output = match run_with_timeout(command, LOGIN_CHECK_TIMEOUT) {
        Ok(output) => output,
        Err(error) => {
            let lower_error = error.to_ascii_lowercase();
            let message = if lower_error.contains("超时") || lower_error.contains("timeout") {
                "连接 Telegram 超时，请检查网络或代理设置。"
            } else {
                "无法确认 Telegram 登录状态。"
            };
            return Ok(login_status(false, message, Some(error), None, None));
        }
    };

    if output.status.success() {
        let account = detect_account_from_chats(&output.stdout);
        return Ok(login_status(
            true,
            "tdl 已登录 Telegram。",
            None,
            account.as_ref().and_then(|item| item.username.clone()),
            account.and_then(|item| item.display_name),
        ));
    }

    let detail = output_detail(&output.stdout, &output.stderr);
    let lower_detail = detail.to_ascii_lowercase();
    let message = if lower_detail.contains("not authorized") {
        "tdl 尚未登录 Telegram。"
    } else if lower_detail.contains("context deadline exceeded")
        || lower_detail.contains("dial failed")
    {
        "无法连接 Telegram，请检查网络或代理设置。"
    } else {
        "无法确认 Telegram 登录状态。"
    };

    Ok(login_status(
        false,
        message,
        (!detail.is_empty()).then_some(detail),
        None,
        None,
    ))
}

#[tauri::command]
pub fn start_login(
    app: AppHandle,
    state: State<'_, AppState>,
    request: LoginRequest,
) -> Result<LoginStarted, String> {
    let operation_guard = OperationGuard::login(&state)?;
    if !lock(&state.running)?.is_empty() {
        return Err("当前还有下载任务在执行，请等任务结束或取消后再登录。".into());
    }
    if lock(&state.login)?.is_some() {
        return Err("已有登录流程在运行。".into());
    }

    let tdl = resolve_tdl(&app, &state)?;
    if !tdl.available {
        return Err("未找到可用的 tdl.exe，无法登录 Telegram。".into());
    }
    let tdl_path = PathBuf::from(tdl.path.ok_or_else(|| "tdl 路径不可用".to_string())?);
    let login_id = state.next_id("login");

    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    let config = lock(&state.config)?.clone();
    let login_args = prepend_tdl_global_args(&config, build_login_args(&request));
    command
        .args(&login_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let database_guard = tdl_database_guard()?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 登录失败: {error}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    {
        *lock(&state.login)? = Some(Arc::clone(&child));
        *lock(&state.login_cancelled)? = false;
        drop(operation_guard);
    }

    let qr_lines = Arc::new(Mutex::new(Vec::new()));
    if let Some(stream) = stdout {
        spawn_login_reader(app.clone(), login_id.clone(), stream, Arc::clone(&qr_lines));
    }
    if let Some(stream) = stderr {
        spawn_login_reader(app.clone(), login_id.clone(), stream, qr_lines);
    }
    spawn_login_monitor(app, state.inner(), login_id.clone(), child, database_guard);

    Ok(LoginStarted {
        login_id,
        method: request.method,
    })
}

#[tauri::command]
pub fn cancel_login(state: State<'_, AppState>) -> Result<(), String> {
    let child = lock(&state.login)?.clone();
    if let Some(child) = child {
        *lock(&state.login_cancelled)? = true;
        let mut child = lock(&child)?;
        let _ = child.kill();
    }
    Ok(())
}

#[tauri::command]
pub fn logout(state: State<'_, AppState>) -> Result<LoginStatus, String> {
    ensure_tdl_update_not_running(&state, "tdl 正在更新，请等待更新完成后再退出登录。")?;
    if !lock(&state.running)?.is_empty() {
        return Err("当前还有下载任务在执行，请等任务结束或取消后再退出登录。".into());
    }
    if lock(&state.login)?.is_some() {
        return Err("当前有登录流程在运行，请先取消或等待完成。".into());
    }

    let config = lock(&state.config)?.clone();
    let _database_guard = tdl_database_guard_for_quick_task()?;
    let targets = session_targets(&config)?;
    remove_if_exists(&targets.namespace_dir)?;
    for legacy_file in targets.legacy_files {
        remove_if_exists(&legacy_file)?;
    }

    Ok(login_status(
        false,
        "已退出 Telegram 登录。",
        None,
        None,
        None,
    ))
}

#[derive(Debug)]
struct AccountInfo {
    username: Option<String>,
    display_name: Option<String>,
}

fn tdl_session_exists(config: &AppConfig) -> Result<bool, String> {
    let targets = session_targets(config)?;
    Ok(targets.namespace_dir.exists() || targets.legacy_files.iter().any(|path| path.exists()))
}

fn login_status(
    logged_in: bool,
    message: &str,
    detail: Option<String>,
    username: Option<String>,
    display_name: Option<String>,
) -> LoginStatus {
    LoginStatus {
        logged_in,
        message: message.into(),
        detail,
        username,
        display_name,
    }
}

fn detect_account_from_chats(stdout: &[u8]) -> Option<AccountInfo> {
    let value: Value = serde_json::from_slice(stdout).ok()?;
    let chats = value.as_array()?;
    chats.iter().find_map(|item| {
        let chat_type = get_string(item, "type")?;
        let name = get_string(item, "visible_name").or_else(|| get_string(item, "name"));
        let username = get_string(item, "username");
        let is_self = chat_type.eq_ignore_ascii_case("self")
            || name.as_deref().is_some_and(|value| {
                matches!(value, "Saved Messages" | "Saved messages" | "收藏夹")
            });
        is_self.then_some(AccountInfo {
            username,
            display_name: name,
        })
    })
}

fn get_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    })
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| format!("清理 tdl 登录态失败 ({}): {error}", path.display()))
}

fn build_login_args(request: &LoginRequest) -> Vec<String> {
    let mut args = vec![
        "login".to_string(),
        "-T".to_string(),
        match request.method {
            LoginMethod::Desktop => "desktop",
            LoginMethod::Qr => "qr",
        }
        .to_string(),
    ];

    if request.method == LoginMethod::Desktop {
        if let Some(path) = request
            .desktop_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            args.extend(["-d".to_string(), path.to_string()]);
        }
        if let Some(passcode) = request
            .passcode
            .as_deref()
            .filter(|passcode| !passcode.is_empty())
        {
            args.extend(["-p".to_string(), passcode.to_string()]);
        }
    }

    args
}

fn spawn_login_reader<R>(
    app: AppHandle,
    login_id: String,
    stream: R,
    qr_lines: Arc<Mutex<Vec<String>>>,
) where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut pending = Vec::with_capacity(512);

        loop {
            let buf = match reader.fill_buf() {
                Ok([]) => break,
                Ok(slice) => slice,
                Err(error) => {
                    emit_login_output(&app, &login_id, format!("读取 tdl 登录输出失败: {error}"));
                    break;
                }
            };

            let consumed = buf.len();
            for &byte in buf {
                match byte {
                    b'\n' | b'\r' => flush_login_line(&app, &login_id, &mut pending, &qr_lines),
                    _ => pending.push(byte),
                }
            }
            reader.consume(consumed);
        }

        if !pending.is_empty() {
            flush_login_line(&app, &login_id, &mut pending, &qr_lines);
        }
    });
}

fn flush_login_line(
    app: &AppHandle,
    login_id: &str,
    pending: &mut Vec<u8>,
    qr_lines: &Arc<Mutex<Vec<String>>>,
) {
    if pending.is_empty() {
        return;
    }
    let raw = String::from_utf8_lossy(pending).to_string();
    pending.clear();

    let line = strip_ansi(&raw).trim_end().to_string();
    if line.trim().is_empty() {
        return;
    }

    if is_qr_line(&line) {
        match qr_lines.lock() {
            Ok(mut lines) => {
                lines.push(line);
                emit_login_qr(app, login_id, lines.join("\n"));
            }
            Err(_) => eprintln!("[login] QR buffer lock poisoned for login {login_id}"),
        }
    } else {
        emit_login_output(app, login_id, line.trim().to_string());
    }
}

fn is_qr_line(line: &str) -> bool {
    line.chars().any(|ch| {
        matches!(
            ch,
            '█' | '▀'
                | '▄'
                | '▌'
                | '▐'
                | '▖'
                | '▗'
                | '▘'
                | '▙'
                | '▚'
                | '▛'
                | '▜'
                | '▝'
                | '▞'
                | '▟'
        )
    })
}

fn emit_login_output(app: &AppHandle, login_id: &str, line: String) {
    let _ = app.emit(
        "login-event",
        LoginEvent {
            login_id: login_id.to_string(),
            kind: LoginEventKind::Output,
            line: Some(line),
            qr: None,
            status: None,
            message: None,
            error: None,
        },
    );
}

fn emit_login_qr(app: &AppHandle, login_id: &str, qr: String) {
    let _ = app.emit(
        "login-event",
        LoginEvent {
            login_id: login_id.to_string(),
            kind: LoginEventKind::Qr,
            line: None,
            qr: Some(qr),
            status: None,
            message: None,
            error: None,
        },
    );
}

fn spawn_login_monitor(
    app: AppHandle,
    state: &AppState,
    login_id: String,
    child: Arc<Mutex<std::process::Child>>,
    database_guard: TdlDatabaseGuard,
) {
    let login = Arc::clone(&state.login);
    let login_cancelled = Arc::clone(&state.login_cancelled);
    thread::spawn(move || {
        let _database_guard = database_guard;
        let exit_code = loop {
            let status = {
                let mut child = match child.lock() {
                    Ok(child) => child,
                    Err(_) => {
                        eprintln!("[login] child process lock poisoned for login {login_id}");
                        break None;
                    }
                };
                child.try_wait()
            };

            match status {
                Ok(Some(status)) => break status.code(),
                Ok(None) => thread::sleep(Duration::from_millis(180)),
                Err(_) => break None,
            }
        };

        match login.lock() {
            Ok(mut login) => *login = None,
            Err(_) => eprintln!("[login] login-state lock poisoned for login {login_id}"),
        }
        let cancelled = match login_cancelled.lock() {
            Ok(mut value) => {
                let cancelled = *value;
                *value = false;
                cancelled
            }
            Err(_) => {
                eprintln!("[login] cancellation-state lock poisoned for login {login_id}");
                false
            }
        };

        let (status, message, error) = if cancelled {
            (
                LoginResultStatus::Cancelled,
                "登录已取消".to_string(),
                Some("用户取消登录".to_string()),
            )
        } else if exit_code == Some(0) {
            (LoginResultStatus::Completed, "登录完成".to_string(), None)
        } else {
            let detail = exit_code
                .map(|code| format!("登录失败 (退出码: {code})"))
                .unwrap_or_else(|| "登录失败，未能获取退出码".to_string());
            (LoginResultStatus::Failed, detail.clone(), Some(detail))
        };

        let _ = app.emit(
            "login-event",
            LoginEvent {
                login_id,
                kind: LoginEventKind::Complete,
                line: None,
                qr: None,
                status: Some(status),
                message: Some(message),
                error,
            },
        );
    });
}

fn output_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_desktop_login_args() {
        let args = build_login_args(&LoginRequest {
            method: LoginMethod::Desktop,
            desktop_path: Some("D:/Telegram Desktop".into()),
            passcode: Some("1234".into()),
        });

        assert_eq!(
            args,
            vec![
                "login",
                "-T",
                "desktop",
                "-d",
                "D:/Telegram Desktop",
                "-p",
                "1234"
            ]
        );
    }

    #[test]
    fn builds_qr_login_args() {
        let args = build_login_args(&LoginRequest {
            method: LoginMethod::Qr,
            desktop_path: Some("ignored".into()),
            passcode: Some("ignored".into()),
        });

        assert_eq!(args, vec!["login", "-T", "qr"]);
    }

    #[test]
    fn detects_qr_lines() {
        assert!(is_qr_line("████ ▄▄▄▄▄ ████"));
        assert!(!is_qr_line("Scan QR code with your Telegram app..."));
    }
}
