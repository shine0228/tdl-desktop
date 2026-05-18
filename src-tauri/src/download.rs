use std::{
    io::{BufRead, BufReader, Read},
    process::Child,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use chrono::Utc;
use tauri::{AppHandle, Emitter};

use crate::{
    state::{AppState, StateRefs},
    types::{
        DownloadEvent, DownloadEventKind, DownloadFileProgress, DownloadRecord, DownloadRequest,
        DownloadStatus, SourceMode,
    },
    util::{parse_progress, strip_ansi, write_json},
};

pub fn build_records(
    state: &AppState,
    request: &DownloadRequest,
    task_id: &str,
    created_at: &str,
) -> Result<Vec<DownloadRecord>, String> {
    let sources = match request.mode {
        SourceMode::Links => {
            if request.links.is_empty() {
                return Err("请至少输入一个消息链接。".into());
            }
            request.links.clone()
        }
        SourceMode::Json => {
            if request.files.is_empty() {
                return Err("请至少输入一个 JSON 文件路径。".into());
            }
            request.files.clone()
        }
        SourceMode::Raw => {
            if request.raw_args.trim().is_empty() {
                return Err("请输入 tdl 原始参数。".into());
            }
            vec![request.raw_args.trim().to_string()]
        }
    };

    Ok(sources
        .into_iter()
        .map(|source| DownloadRecord {
            id: state.next_id("record"),
            task_id: task_id.to_string(),
            source,
            mode: request.mode,
            directory: request.directory.clone(),
            status: DownloadStatus::Downloading,
            created_at: created_at.to_string(),
            completed_at: None,
            error: None,
        })
        .collect())
}

pub fn build_download_args(request: &DownloadRequest) -> Result<Vec<String>, String> {
    if request.mode == SourceMode::Raw {
        let mut args = split_args(&request.raw_args)?;
        if args
            .first()
            .map(|arg| arg.eq_ignore_ascii_case("tdl") || arg.eq_ignore_ascii_case("tdl.exe"))
            .unwrap_or(false)
        {
            args.remove(0);
        }
        if args.is_empty() {
            return Err("请输入 tdl 原始参数。".into());
        }
        return Ok(args);
    }

    let directory = request.directory.trim();
    if directory.is_empty() {
        return Err("请选择下载目录。".into());
    }

    let mut args = vec![
        "download".to_string(),
        "-d".to_string(),
        directory.to_string(),
        "-l".to_string(),
        request.limit.max(1).to_string(),
        "-t".to_string(),
        request.threads.max(1).to_string(),
    ];

    if request.pool > 0 {
        args.extend(["--pool".to_string(), request.pool.to_string()]);
    }

    match request.mode {
        SourceMode::Links => {
            if request.links.is_empty() {
                return Err("请至少输入一个消息链接。".into());
            }
            for link in &request.links {
                args.extend(["-u".to_string(), link.trim().to_string()]);
            }
        }
        SourceMode::Json => {
            if request.files.is_empty() {
                return Err("请至少输入一个 JSON 文件路径。".into());
            }
            for file in &request.files {
                args.extend(["-f".to_string(), file.trim().to_string()]);
            }
        }
        SourceMode::Raw => unreachable!(),
    }

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

    Ok(args)
}

fn push_csv_arg(args: &mut Vec<String>, flag: &str, value: &str) {
    let value = value.trim().trim_matches(',');
    if !value.is_empty() {
        args.extend([flag.to_string(), value.to_string()]);
    }
}

fn split_args(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in input.chars() {
        match ch {
            '"' | '\'' if quote == Some(ch) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(ch),
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if quote.is_some() {
        return Err("原始参数中的引号没有闭合。".into());
    }

    if !current.is_empty() {
        args.push(current);
    }

    Ok(args)
}

/// 把子进程输出按 `\n` 或 `\r` 切成行,实时往前端推。
/// 单独处理是因为 tdl 的进度条只刷 `\r` 不换行,
/// 标准的 `BufRead::lines()` 会把整段进度全卡在缓冲区里。
pub fn spawn_output_reader<R>(app: AppHandle, task_id: String, stream: R)
where
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
                    emit_line(&app, &task_id, format!("读取 tdl 输出失败: {error}"), None);
                    break;
                }
            };

            let consumed = buf.len();
            for &byte in buf {
                match byte {
                    b'\n' | b'\r' => flush_line(&app, &task_id, &mut pending),
                    _ => pending.push(byte),
                }
            }
            reader.consume(consumed);
        }

        // 进程结束时把残留输出也吐出去,避免最后一行被吞。
        if !pending.is_empty() {
            flush_line(&app, &task_id, &mut pending);
        }
    });
}

fn flush_line(app: &AppHandle, task_id: &str, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    let raw = String::from_utf8_lossy(pending).to_string();
    pending.clear();

    let line = strip_ansi(&raw).trim().to_string();
    if line.is_empty() {
        return;
    }

    let progress = parse_progress(&line);
    emit_line(app, task_id, line, progress);
}

fn emit_line(app: &AppHandle, task_id: &str, line: String, progress: Option<f64>) {
    let file_progress = parse_file_progress(&line);
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            task_id: task_id.to_string(),
            kind: DownloadEventKind::Output,
            line: Some(line),
            progress,
            file_progress,
            status: None,
            message: None,
            record_ids: Vec::new(),
            completed_at: None,
            error: None,
        },
    );
}

pub fn spawn_process_monitor(
    app: AppHandle,
    state: StateRefs,
    task_id: String,
    record_ids: Vec<String>,
    child: Arc<Mutex<Child>>,
) {
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

        let cancelled = state
            .cancelled
            .lock()
            .map(|mut cancelled| cancelled.remove(&task_id))
            .unwrap_or(false);

        if let Ok(mut running) = state.running.lock() {
            running.remove(&task_id);
        }

        let (status, message, error) = if cancelled {
            (
                DownloadStatus::Cancelled,
                "下载已取消".to_string(),
                Some("用户取消下载".to_string()),
            )
        } else if exit_code == Some(0) {
            (DownloadStatus::Completed, "下载完成".to_string(), None)
        } else {
            let detail = exit_code
                .map(|code| format!("下载失败 (退出码: {code})"))
                .unwrap_or_else(|| "下载失败,未能获取退出码".to_string());
            (DownloadStatus::Failed, detail.clone(), Some(detail))
        };

        let completed_at = Utc::now().to_rfc3339();
        if let Ok(mut history) = state.history.lock() {
            for record in history.iter_mut() {
                if record_ids.contains(&record.id) {
                    record.status = status;
                    record.completed_at = Some(completed_at.clone());
                    record.error = error.clone();
                }
            }
            let _ = write_json(&state.history_path, &*history);
        }

        let _ = app.emit(
            "download-event",
            DownloadEvent {
                task_id,
                kind: DownloadEventKind::Complete,
                line: None,
                progress: None,
                file_progress: None,
                status: Some(status),
                message: Some(message),
                record_ids,
                completed_at: Some(completed_at),
                error,
            },
        );
    });
}

fn parse_file_progress(line: &str) -> Option<DownloadFileProgress> {
    let (span, progress) = last_percent_span(line)?;
    let name = clean_progress_label(&line[..span.start]);
    if name.is_empty() || is_generic_progress_label(&name) {
        return None;
    }

    let key = progress_key(&name);
    if key.is_empty() {
        return None;
    }

    Some(DownloadFileProgress {
        key,
        name,
        progress: progress.clamp(0.0, 100.0),
        done: progress >= 99.9 || line.to_ascii_lowercase().contains("done"),
    })
}

fn last_percent_span(line: &str) -> Option<(std::ops::Range<usize>, f64)> {
    let bytes = line.as_bytes();
    let mut last: Option<(std::ops::Range<usize>, f64)> = None;

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
            last = Some((start..idx, value));
        }
    }

    last
}

fn clean_progress_label(prefix: &str) -> String {
    let mut label = prefix.trim();
    if let Some(index) = label.rfind('[') {
        label = &label[..index];
    }
    if let Some(index) = label.rfind('|') {
        label = &label[..index];
    }

    label
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '[' | ']'
                        | '|'
                        | '.'
                        | '#'
                        | '<'
                        | '>'
                        | '='
                        | '-'
                        | '_'
                        | '*'
                        | '░'
                        | '▒'
                        | '▓'
                        | '█'
                        | '▏'
                        | '▎'
                        | '▍'
                        | '▌'
                        | '▋'
                        | '▊'
                        | '▉'
                )
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_generic_progress_label(label: &str) -> bool {
    matches!(
        label.to_ascii_lowercase().as_str(),
        "download" | "downloading" | "progress" | "total" | "all"
    )
}

fn progress_key(label: &str) -> String {
    label
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request() -> DownloadRequest {
        DownloadRequest {
            mode: SourceMode::Links,
            directory: "D:/Downloads".into(),
            links: vec!["https://t.me/foo/1".into()],
            files: vec![],
            raw_args: String::new(),
            limit: 4,
            threads: 4,
            pool: 8,
            group: true,
            include: String::new(),
            exclude: String::new(),
            template: String::new(),
            skip_same: true,
            continue_last: false,
            restart: false,
            desc: false,
            takeout: false,
            rewrite_ext: false,
        }
    }

    #[test]
    fn builds_links_args() {
        let args = build_download_args(&base_request()).unwrap();
        assert_eq!(args[0], "download");
        assert!(args.contains(&"-u".to_string()));
        assert!(args.contains(&"--group".to_string()));
        assert!(args.contains(&"--skip-same".to_string()));
    }

    #[test]
    fn rejects_empty_directory_for_links() {
        let mut req = base_request();
        req.directory = "  ".into();
        assert!(build_download_args(&req).is_err());
    }

    #[test]
    fn raw_strips_leading_tdl() {
        let mut req = base_request();
        req.mode = SourceMode::Raw;
        req.raw_args = "tdl download -u https://t.me/foo/1".into();
        let args = build_download_args(&req).unwrap();
        assert_eq!(args[0], "download");
    }

    #[test]
    fn split_args_handles_quotes() {
        let args = split_args("--template \"a b c\" --foo bar").unwrap();
        assert_eq!(args, vec!["--template", "a b c", "--foo", "bar"]);
    }

    #[test]
    fn split_args_rejects_unclosed_quote() {
        assert!(split_args("--template \"oops").is_err());
    }

    #[test]
    fn parses_file_progress_label() {
        let progress = parse_file_progress("video file.mp4 [=====>] 42.5% 1.2 MiB/s").unwrap();
        assert_eq!(progress.name, "video file.mp4");
        assert_eq!(progress.progress, 42.5);
        assert!(!progress.done);
    }

    #[test]
    fn marks_file_progress_done() {
        let progress = parse_file_progress("archive.zip |████| 100% done").unwrap();
        assert_eq!(progress.name, "archive.zip");
        assert!(progress.done);
    }

    #[test]
    fn ignores_generic_progress_label() {
        assert!(parse_file_progress("downloading 12%").is_none());
    }
}
