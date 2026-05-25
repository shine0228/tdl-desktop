use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::{
    commands::ensure_tdl_update_not_running,
    state::AppState,
    tdl::resolve_tdl,
    tdl_config::prepend_tdl_global_args,
    types::LinkPreview,
    util::{apply_hidden_process_flags, lock, run_with_timeout},
};

const PREVIEW_TIMEOUT: Duration = Duration::from_secs(25);

#[tauri::command]
pub fn preview_link(
    app: AppHandle,
    state: State<'_, AppState>,
    link: String,
) -> Result<LinkPreview, String> {
    ensure_tdl_update_not_running(&state, "tdl 正在更新，请等待更新完成后再读取消息预览。")?;
    let parsed = parse_telegram_link(&link)?;
    let tdl = resolve_tdl(&app, &state)?;
    if !tdl.available {
        return Err("tdl 不可用，无法读取消息预览。".into());
    }
    let tdl_path = PathBuf::from(
        tdl.path
            .ok_or_else(|| "tdl 路径不可用，无法读取消息预览。".to_string())?,
    );

    let preview_dir = state.app_dir.join("preview");
    fs::create_dir_all(&preview_dir).map_err(|error| format!("无法创建预览目录: {error}"))?;
    let output_path = preview_dir.join(format!(
        "{}-{}-{}.json",
        state.next_id("preview"),
        sanitize_file_name(&parsed.chat),
        parsed.message_id
    ));

    let config = lock(&state.config)?.clone();
    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    let args = prepend_tdl_global_args(
        &config,
        vec![
            "chat".to_string(),
            "export".to_string(),
            "--with-content".to_string(),
            "--all".to_string(),
            "-T".to_string(),
            "id".to_string(),
            "-c".to_string(),
            parsed.chat.clone(),
            "-i".to_string(),
            format!("{},{}", parsed.message_id, parsed.message_id),
            "-o".to_string(),
            output_path.to_string_lossy().to_string(),
        ],
    );
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let output = run_with_timeout(command, PREVIEW_TIMEOUT)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail =
            first_non_empty(&[stderr.trim(), stdout.trim()]).unwrap_or("tdl chat export 执行失败");
        return Err(format!("读取消息预览失败: {}", compact_message(detail)));
    }

    let content = fs::read_to_string(&output_path)
        .map_err(|error| format!("读取消息预览结果失败: {error}"))?;
    let value: Value =
        serde_json::from_str(&content).map_err(|error| format!("解析消息预览结果失败: {error}"))?;

    let message = find_message_by_id(&value, parsed.message_id)
        .ok_or_else(|| format!("tdl 已导出 JSON，但未找到消息 ID {}。", parsed.message_id))?;
    let text = find_message_text(message).as_deref().map(compact_message);
    let media_count = count_media(message);
    let _ = fs::remove_file(&output_path);

    Ok(LinkPreview {
        link,
        chat: parsed.chat,
        message_id: parsed.message_id,
        text,
        media_count,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedTelegramLink {
    chat: String,
    message_id: u64,
}

fn parse_telegram_link(link: &str) -> Result<ParsedTelegramLink, String> {
    let mut value = link.trim();
    value = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .unwrap_or(value);
    value = value.strip_prefix("www.").unwrap_or(value);
    let value = value
        .strip_prefix("t.me/")
        .or_else(|| value.strip_prefix("telegram.me/"))
        .ok_or_else(|| "当前仅支持普通 t.me/<频道>/<消息ID> 链接预览。".to_string())?;
    let value = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_matches('/');
    let parts: Vec<&str> = value.split('/').filter(|part| !part.is_empty()).collect();

    let (chat, message_id) = match parts.as_slice() {
        ["s", chat, message_id] => (*chat, *message_id),
        ["c", ..] => return Err("当前预览暂不支持 t.me/c/... 私有链接，但仍可尝试下载。".into()),
        [chat, message_id] => (*chat, *message_id),
        _ => return Err("当前仅支持普通 t.me/<频道>/<消息ID> 链接预览。".into()),
    };

    let message_id = message_id
        .parse::<u64>()
        .map_err(|_| "消息链接里没有可识别的消息 ID。".to_string())?;
    if chat.is_empty() {
        return Err("消息链接里没有可识别的频道名。".into());
    }

    Ok(ParsedTelegramLink {
        chat: chat.trim_start_matches('@').to_string(),
        message_id,
    })
}

fn find_message_text(value: &Value) -> Option<String> {
    const TEXT_KEYS: &[&str] = &[
        "text",
        "message",
        "caption",
        "content",
        "messageText",
        "rawText",
    ];

    match value {
        Value::Object(map) => {
            for key in TEXT_KEYS {
                if let Some(Value::String(text)) = map.get(*key) {
                    let text = text.trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            for child in map.values() {
                if let Some(text) = find_message_text(child) {
                    return Some(text);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_message_text),
        Value::String(_) => None,
        _ => None,
    }
}

fn find_message_by_id(value: &Value, message_id: u64) -> Option<&Value> {
    match value {
        Value::Object(map) => {
            if value_has_message_id(value, message_id) {
                return Some(value);
            }

            if let Some(Value::Array(messages)) = map.get("messages") {
                if let Some(found) = messages
                    .iter()
                    .find(|message| value_has_message_id(message, message_id))
                {
                    return Some(found);
                }
            }

            map.values()
                .find_map(|child| find_message_by_id(child, message_id))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_message_by_id(child, message_id)),
        _ => None,
    }
}

fn value_has_message_id(value: &Value, message_id: u64) -> bool {
    let Value::Object(map) = value else {
        return false;
    };

    [
        "id",
        "ID",
        "message_id",
        "MessageID",
        "messageId",
        "MessageId",
    ]
    .iter()
    .any(|key| {
        map.get(*key).is_some_and(|value| match value {
            Value::Number(number) => number.as_u64() == Some(message_id),
            Value::String(text) => text.parse::<u64>().ok() == Some(message_id),
            _ => false,
        })
    })
}

fn count_media(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            let direct = ["media", "file", "files", "document", "photo", "video"]
                .iter()
                .filter(|key| map.get(**key).is_some_and(|value| !value.is_null()))
                .count();
            direct + map.values().map(count_media).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_media).sum(),
        _ => 0,
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
    if compact.chars().count() > 500 {
        compact = compact.chars().take(500).collect::<String>();
        compact.push_str("...");
    }
    compact
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_tme_link() {
        assert_eq!(
            parse_telegram_link("https://t.me/channel_name/123?single").unwrap(),
            ParsedTelegramLink {
                chat: "channel_name".into(),
                message_id: 123,
            }
        );
    }

    #[test]
    fn parses_public_preview_link() {
        assert_eq!(
            parse_telegram_link("t.me/s/channel_name/456").unwrap(),
            ParsedTelegramLink {
                chat: "channel_name".into(),
                message_id: 456,
            }
        );
    }

    #[test]
    fn rejects_private_link_preview() {
        assert!(parse_telegram_link("https://t.me/c/123/456").is_err());
    }

    #[test]
    fn extracts_text_from_nested_json() {
        let value = serde_json::json!([{ "meta": { "message": "hello" } }]);
        assert_eq!(find_message_text(&value), Some("hello".into()));
    }

    #[test]
    fn finds_exact_message_by_id() {
        let value = serde_json::json!({
            "id": 3857071525u64,
            "messages": [
                { "id": 4852, "text": "wrong" },
                { "id": 4853, "text": "right" }
            ]
        });
        let message = find_message_by_id(&value, 4853).unwrap();
        assert_eq!(find_message_text(message), Some("right".into()));
    }
}
