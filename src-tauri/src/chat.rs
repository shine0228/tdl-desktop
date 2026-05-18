use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::{
    download::{spawn_output_reader, spawn_process_monitor_with_cleanup},
    state::AppState,
    tdl::resolve_tdl,
    types::{
        ChatDownloadRequest, ChatInfo, DownloadRecord, DownloadStarted, DownloadStatus,
        MessageInfo, SourceMode,
    },
    util::{apply_hidden_process_flags, lock, preview_command, run_with_timeout},
};

const CHAT_LIST_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_EXPORT_TIMEOUT: Duration = Duration::from_secs(90);

#[tauri::command]
pub fn list_chats(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<ChatInfo>, String> {
    let tdl_path = resolve_tdl_path(&app, &state)?;
    let args = vec![
        "chat".to_string(),
        "ls".to_string(),
        "-o".to_string(),
        "json".to_string(),
    ];
    let output = run_tdl_with_timeout(&tdl_path, &args, CHAT_LIST_TIMEOUT)?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("解析对话列表失败: {error}"))?;
    let items = value
        .as_array()
        .ok_or_else(|| "tdl 返回的对话列表格式不正确。".to_string())?;

    Ok(items.iter().filter_map(parse_chat_info).collect())
}

#[tauri::command]
pub fn export_chat_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    count: i64,
) -> Result<Vec<MessageInfo>, String> {
    let tdl_path = resolve_tdl_path(&app, &state)?;
    let output_path = temp_chat_json(&state, "messages")?;
    let count = count.clamp(1, 500);
    let args = vec![
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
    ];

    let result = run_tdl_with_timeout(&tdl_path, &args, CHAT_EXPORT_TIMEOUT)
        .and_then(|_| parse_messages_file(&output_path));
    let _ = fs::remove_file(&output_path);
    result
}

#[tauri::command]
pub fn download_from_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ChatDownloadRequest,
) -> Result<DownloadStarted, String> {
    if request.message_ids.is_empty() {
        return Err("请至少选择一条消息。".into());
    }
    let directory = request.directory.trim();
    if directory.is_empty() {
        return Err("请选择下载目录。".into());
    }
    fs::create_dir_all(directory).map_err(|error| format!("无法创建下载目录: {error}"))?;

    let tdl_path = resolve_tdl_path(&app, &state)?;
    let export_path = temp_chat_json(&state, "selected")?;
    let ids = request
        .message_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let export_args = vec![
        "chat".to_string(),
        "export".to_string(),
        "--with-content".to_string(),
        "--all".to_string(),
        "-T".to_string(),
        "id".to_string(),
        "-c".to_string(),
        request.chat_id.clone(),
        "-i".to_string(),
        ids,
        "-o".to_string(),
        export_path.to_string_lossy().to_string(),
    ];
    run_tdl_with_timeout(&tdl_path, &export_args, CHAT_EXPORT_TIMEOUT)?;

    let download_args = build_chat_download_args(&request, &export_path);
    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(&download_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 下载失败: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let task_id = state.next_id("task");
    let created_at = Utc::now().to_rfc3339();
    let records = vec![DownloadRecord {
        id: state.next_id("record"),
        task_id: task_id.clone(),
        source: format!("{} · {} 条消息", request.chat_name, request.message_ids.len()),
        mode: SourceMode::Chat,
        directory: directory.to_string(),
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

    let child = Arc::new(Mutex::new(child));
    lock(&state.running)?.insert(task_id.clone(), Arc::clone(&child));

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
    );

    Ok(DownloadStarted {
        task_id,
        command_preview: preview_command(&tdl_path.to_string_lossy(), &download_args),
        records,
    })
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
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = first_non_empty(&[stderr.trim(), stdout.trim()]).unwrap_or("tdl 执行失败");
        return Err(compact_message(detail));
    }
    Ok(output)
}

fn temp_chat_json(state: &AppState, prefix: &str) -> Result<PathBuf, String> {
    let dir = state.app_dir.join("chat");
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建对话缓存目录: {error}"))?;
    Ok(dir.join(format!("{}-{}.json", state.next_id(prefix), Utc::now().timestamp_millis())))
}

fn build_chat_download_args(request: &ChatDownloadRequest, file: &Path) -> Vec<String> {
    let mut args = vec![
        "download".to_string(),
        "-d".to_string(),
        request.directory.trim().to_string(),
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
        args.extend(["--template".to_string(), request.template.trim().to_string()]);
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
    let name = get_string(value, &["visible_name", "visibleName", "name", "title"])
        .unwrap_or_else(|| id.to_string());
    Some(ChatInfo {
        id,
        name,
        chat_type: get_string(value, &["type", "chat_type", "chatType"]).unwrap_or_default(),
        username: get_string(value, &["username", "user_name", "userName"]),
    })
}

fn parse_messages_file(path: &Path) -> Result<Vec<MessageInfo>, String> {
    let content = fs::read_to_string(path).map_err(|error| format!("读取消息列表失败: {error}"))?;
    let value: Value = serde_json::from_str(&content).map_err(|error| format!("解析消息列表失败: {error}"))?;
    let messages = find_messages(&value).ok_or_else(|| "tdl 导出结果里没有 messages 数组。".to_string())?;
    Ok(messages.iter().filter_map(parse_message_info).collect())
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
    let id = get_i64(value, &["id", "ID", "message_id", "messageId"])?;
    Some(MessageInfo {
        id,
        date: get_string(value, &["date", "Date", "created_at", "createdAt"]),
        text: find_text(value).map(|text| compact_message(&text)),
        media_type: find_media_type(value),
        file_name: find_file_name(value),
        file_size: find_file_size(value),
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
    const KEYS: &[&str] = &["media", "mime_type", "mimeType", "type", "class_name", "className"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    if !text.trim().is_empty() {
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

fn find_file_name(value: &Value) -> Option<String> {
    const KEYS: &[&str] = &["file_name", "fileName", "filename", "name"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    if !text.trim().is_empty() {
                        return Some(text.trim().to_string());
                    }
                }
            }
            map.values().find_map(find_file_name)
        }
        Value::Array(items) => items.iter().find_map(find_file_name),
        _ => None,
    }
}

fn find_file_size(value: &Value) -> Option<i64> {
    const KEYS: &[&str] = &["file_size", "fileSize", "size"];
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
    keys.iter().find_map(|key| map.get(*key).and_then(value_to_i64))
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| number.as_u64().and_then(|v| i64::try_from(v).ok())),
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
