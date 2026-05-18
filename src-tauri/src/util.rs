use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use serde::{de::DeserializeOwned, Serialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn apply_hidden_process_flags(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

pub fn read_json<T>(path: &Path) -> Option<T>
where
    T: DeserializeOwned,
{
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn write_json<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: Serialize + ?Sized,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建数据目录: {error}"))?;
    }
    let content =
        serde_json::to_string_pretty(value).map_err(|error| format!("序列化数据失败: {error}"))?;
    fs::write(path, content).map_err(|error| format!("写入数据失败: {error}"))
}

pub fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "内部状态锁已损坏".to_string())
}

pub fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }

    output
}

/// 在一行 tdl 输出中找进度百分比,取最后一个 `<num>%`,
/// 这样同时含"总进度/单文件进度"时更接近整体进度。
pub fn parse_progress(line: &str) -> Option<f64> {
    let bytes = line.as_bytes();
    let mut last: Option<f64> = None;
    for (idx, ch) in line.char_indices() {
        if ch != '%' {
            continue;
        }
        let mut start = idx;
        while start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_digit() || prev == b'.' {
                start -= 1;
            } else {
                break;
            }
        }
        if start == idx {
            continue;
        }
        if let Ok(value) = line[start..idx].parse::<f64>() {
            last = Some(value);
        }
    }
    last
}

pub fn quote_for_preview(value: &str) -> String {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

pub fn preview_command(program: &str, args: &[String]) -> String {
    std::iter::once(quote_for_preview(program))
        .chain(args.iter().map(|arg| quote_for_preview(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动进程失败: {error}"))?;
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("读取进程输出失败: {error}"))
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(120));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_percent() {
        assert_eq!(parse_progress("downloading 42% done"), Some(42.0));
        assert_eq!(parse_progress("[12.5%] foo"), Some(12.5));
    }

    #[test]
    fn picks_last_percent_when_multiple() {
        assert_eq!(parse_progress("file 50% total 73%"), Some(73.0));
    }

    #[test]
    fn returns_none_when_no_percent() {
        assert!(parse_progress("hello world").is_none());
    }

    #[test]
    fn strips_ansi_sequences() {
        let raw = "\u{1b}[32mok\u{1b}[0m";
        assert_eq!(strip_ansi(raw), "ok");
    }
}
