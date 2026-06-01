use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock},
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

pub fn tdl_database_guard() -> Result<TdlDatabaseGuard, String> {
    acquire_tdl_database_guard(None, "tdl 正在执行其他任务，请稍后再试。")
}

pub fn tdl_database_guard_with_timeout(
    timeout: Duration,
    busy_message: &str,
) -> Result<TdlDatabaseGuard, String> {
    acquire_tdl_database_guard(Some(timeout), busy_message)
}

pub fn tdl_database_guard_for_quick_task() -> Result<TdlDatabaseGuard, String> {
    tdl_database_guard_with_timeout(
        Duration::from_secs(2),
        "tdl 正在执行下载、登录或预览任务，请稍后再试。",
    )
}

fn acquire_tdl_database_guard(
    timeout: Option<Duration>,
    busy_message: &str,
) -> Result<TdlDatabaseGuard, String> {
    let gate = TDL_DATABASE_LOCK
        .get_or_init(|| Arc::new(TdlDatabaseGate::default()))
        .clone();
    {
        let mut locked = gate
            .locked
            .lock()
            .map_err(|_| "tdl 数据库访问锁已损坏。".to_string())?;
        match timeout {
            Some(timeout) => {
                let started = Instant::now();
                let mut remaining = timeout;
                while *locked {
                    let (next_locked, wait_result) = gate
                        .released
                        .wait_timeout(locked, remaining)
                        .map_err(|_| "tdl 数据库访问锁已损坏。".to_string())?;
                    locked = next_locked;
                    if !*locked {
                        break;
                    }
                    if wait_result.timed_out() || started.elapsed() >= timeout {
                        return Err(busy_message.to_string());
                    }
                    remaining = timeout.saturating_sub(started.elapsed());
                }
            }
            None => {
                while *locked {
                    locked = gate
                        .released
                        .wait(locked)
                        .map_err(|_| "tdl 数据库访问锁已损坏。".to_string())?;
                }
            }
        }
        *locked = true;
    }
    Ok(TdlDatabaseGuard { gate })
}

static TDL_DATABASE_LOCK: OnceLock<Arc<TdlDatabaseGate>> = OnceLock::new();

#[derive(Default)]
struct TdlDatabaseGate {
    locked: Mutex<bool>,
    released: Condvar,
}

pub struct TdlDatabaseGuard {
    gate: Arc<TdlDatabaseGate>,
}

impl Drop for TdlDatabaseGuard {
    fn drop(&mut self) {
        match self.gate.locked.lock() {
            Ok(mut locked) => {
                *locked = false;
                self.gate.released.notify_one();
            }
            Err(_) => eprintln!("[tdl] database lock poisoned while releasing guard"),
        }
    }
}

pub fn default_download_dir() -> Result<PathBuf, String> {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "无法定位下载目录，请手动选择一个下载目录。".to_string())
}

pub fn validate_download_dir(value: &str, app_dir: &Path) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("请选择下载目录。".into());
    }

    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("下载目录必须是绝对路径。".into());
    }
    if is_path_root(&path) {
        return Err("不能直接下载到磁盘根目录。".into());
    }
    if path == app_dir {
        return Err("不能把下载目录设为应用数据目录。".into());
    }

    Ok(path)
}

fn is_path_root(path: &Path) -> bool {
    path.parent().is_none() || path.parent().is_some_and(|parent| parent == path)
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
