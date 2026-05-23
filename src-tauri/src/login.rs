use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::{
    state::AppState,
    tdl::resolve_tdl,
    types::{LoginEvent, LoginEventKind, LoginMethod, LoginRequest, LoginResultStatus, LoginStarted, LoginStatus},
    util::{apply_hidden_process_flags, lock, strip_ansi},
};

const LOGIN_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

#[tauri::command]
pub async fn check_login_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LoginStatus, String> {
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

    let tdl_path = PathBuf::from(
        tdl.path
            .ok_or_else(|| "tdl 路径不可用，无法检查登录状态。".to_string())?,
    );

    tauri::async_runtime::spawn_blocking(move || {
        check_login_status_with_helper_or_tdl(app, tdl_path)
    })
    .await
    .map_err(|error| format!("检查 Telegram 登录状态失败: {error}"))?
}

fn check_login_status_with_helper_or_tdl(
    _app: AppHandle,
    tdl_path: PathBuf,
) -> Result<LoginStatus, String> {
    check_login_status_with_tdl(tdl_path)
}

fn check_login_status_with_tdl(tdl_path: PathBuf) -> Result<LoginStatus, String> {
    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(["chat", "ls", "-o", "json", "-f", "false"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let output = match run_with_timeout(command, LOGIN_CHECK_TIMEOUT) {
        Ok(output) => output,
        Err(error) => {
            return Ok(login_status(
                false,
                "无法确认 Telegram 登录状态。",
                Some(error),
                None,
                None,
            ));
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
    let message = if detail.to_ascii_lowercase().contains("not authorized") {
        "tdl 尚未登录 Telegram。"
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
    command
        .args(build_login_args(&request))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 登录失败: {error}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    {
        *lock(&state.login)? = Some(Arc::clone(&child));
        *lock(&state.login_cancelled)? = false;
    }

    let qr_lines = Arc::new(Mutex::new(Vec::new()));
    if let Some(stream) = stdout {
        spawn_login_reader(app.clone(), login_id.clone(), stream, Arc::clone(&qr_lines));
    }
    if let Some(stream) = stderr {
        spawn_login_reader(app.clone(), login_id.clone(), stream, qr_lines);
    }
    spawn_login_monitor(app, state.inner(), login_id.clone(), child);

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
    if !lock(&state.running)?.is_empty() {
        return Err("当前还有下载任务在执行，请等任务结束或取消后再退出登录。".into());
    }
    if lock(&state.login)?.is_some() {
        return Err("当前有登录流程在运行，请先取消或等待完成。".into());
    }

    let home =
        dirs::home_dir().ok_or_else(|| "无法定位用户目录，不能清理 tdl 登录态。".to_string())?;
    let tdl_dir = home.join(".tdl");
    remove_if_exists(&tdl_dir.join("data").join("default"))?;
    remove_if_exists(&tdl_dir.join("data.kv"))?;

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
    fs::remove_file(path)
        .map_err(|error| format!("清理 tdl 登录态失败 ({}): {error}", path.display()))
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
                Ok(slice) if slice.is_empty() => break,
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
        if let Ok(mut lines) = qr_lines.lock() {
            lines.push(line);
            emit_login_qr(app, login_id, lines.join("\n"));
        }
    } else {
        emit_login_output(app, login_id, line.trim().to_string());
    }
}

fn is_qr_line(line: &str) -> bool {
    line.contains('█') || line.contains('▀') || line.contains('▄')
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
) {
    let login = Arc::clone(&state.login);
    let login_cancelled = Arc::clone(&state.login_cancelled);
    thread::spawn(move || {
        let exit_code = loop {
            let status = {
                let mut child = match child.lock() {
                    Ok(child) => child,
                    Err(_) => break None,
                };
                child.try_wait()
            };

            match status {
                Ok(Some(status)) => break status.code(),
                Ok(None) => thread::sleep(Duration::from_millis(180)),
                Err(_) => break None,
            }
        };

        if let Ok(mut login) = login.lock() {
            *login = None;
        }
        let cancelled = login_cancelled
            .lock()
            .map(|mut value| {
                let cancelled = *value;
                *value = false;
                cancelled
            })
            .unwrap_or(false);

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

fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 状态检查失败: {error}"))?;
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("读取 tdl 状态检查输出失败: {error}"))
            }
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(120)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait_with_output();
                return Err("检查 Telegram 登录状态超时。".into());
            }
            Err(error) => {
                let _ = child.kill();
                return Err(format!("检查 tdl 状态失败: {error}"));
            }
        }
    }
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
