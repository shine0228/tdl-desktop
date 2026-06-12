use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use chrono::Utc;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    diagnostics::diagnostics_snapshot,
    download::{
        build_download_args, build_records, shared_output_tail, spawn_output_reader,
        spawn_process_monitor, ChildProcessGuard, PendingHistoryGuard, ProcessMonitorArgs,
    },
    redaction::redact_support_text,
    state::AppState,
    tdl::{resolve_tdl, update_tdl as update_tdl_impl},
    tdl_config::{add_missing_raw_global_args, prepend_tdl_global_args},
    types::{
        AppConfig, AppSnapshot, DesktopUpdateStatus, DiagnosticsSnapshot, DownloadRequest,
        DownloadStarted, LogPackageInfo, SourceMode, TdlInfo, TdlUpdateEvent, TdlUpdateStatus,
    },
    util::{
        apply_hidden_process_flags, lock, preview_command, tdl_database_guard,
        validate_download_dir,
    },
};

#[tauri::command]
pub fn get_app_state(app: AppHandle, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let config = lock(&state.config)?.clone();
    let history = lock(&state.history)?.clone();
    let tdl = if *lock(&state.tdl_update_running)? {
        TdlInfo {
            available: false,
            version: None,
            path: None,
            source: crate::types::TdlSource::Missing,
        }
    } else {
        resolve_tdl(&app, &state)?
    };

    Ok(AppSnapshot {
        config,
        history,
        tdl,
        desktop_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
pub fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    {
        let mut current = lock(&state.config)?;
        *current = config.clone();
        state.persist_config(&current)?;
    }
    Ok(config)
}

#[tauri::command]
pub fn pick_directory() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn pick_log_directory() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_directory(path: String) -> Result<(), String> {
    let path_obj = std::path::Path::new(&path);
    if !path_obj.exists() {
        return Err(format!("目录不存在: {}", path));
    }
    if !path_obj.is_dir() {
        return Err(format!("路径不是目录: {}", path));
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|error| format!("打开目录失败: {error}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| format!("打开目录失败: {error}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| format!("打开目录失败: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
pub fn preview_download_command(
    state: State<'_, AppState>,
    request: DownloadRequest,
) -> Result<String, String> {
    let config = lock(&state.config)?.clone();
    let args = build_download_args(&request)?;
    let args = if request.mode == SourceMode::Raw {
        add_missing_raw_global_args(&config, args)
    } else {
        prepend_tdl_global_args(&config, args)
    };
    Ok(preview_command("tdl", &args))
}

#[tauri::command]
pub fn refresh_tdl_info(app: AppHandle, state: State<'_, AppState>) -> Result<TdlInfo, String> {
    ensure_tdl_update_not_running(&state, "tdl 正在更新，请等待更新完成后再刷新状态。")?;
    resolve_tdl(&app, &state)
}

#[tauri::command]
pub fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    request: DownloadRequest,
) -> Result<DownloadStarted, String> {
    let operation_guard = OperationGuard::download(&state)?;
    let tdl = resolve_tdl(&app, &state)?;
    if !tdl.available {
        return Err("未找到可用的 tdl.exe。发布包应内置 tdl.exe,也可以手动更新 tdl。".into());
    }
    if request.mode == SourceMode::Chat {
        return Err("对话模式请使用对话下载命令。".into());
    }

    let tdl_path = PathBuf::from(
        tdl.path
            .clone()
            .ok_or_else(|| "tdl 路径不可用".to_string())?,
    );
    let config = lock(&state.config)?.clone();
    let args = build_download_args(&request)?;
    let args = if request.mode == SourceMode::Raw {
        add_missing_raw_global_args(&config, args)
    } else {
        prepend_tdl_global_args(&config, args)
    };

    if request.mode != SourceMode::Raw {
        let directory = validate_download_dir(&request.directory, &state.app_dir)?;
        fs::create_dir_all(&directory).map_err(|error| format!("无法创建下载目录: {error}"))?;
    }

    let task_id = state.next_id("task");
    let created_at = Utc::now().to_rfc3339();
    let records = build_records(&state, &request, &task_id, &created_at)?;
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
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let database_guard = tdl_database_guard()?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 失败: {error}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    let child_guard = ChildProcessGuard::new(Arc::clone(&child));
    lock(&state.running)?.insert(task_id.clone(), Arc::clone(&child));
    drop(operation_guard);

    let output_tail = shared_output_tail();
    let mut output_join_handles = Vec::new();
    if let Some(stream) = stdout {
        output_join_handles.push(spawn_output_reader(
            app.clone(),
            task_id.clone(),
            stream,
            Some(Arc::clone(&output_tail)),
        ));
    }
    if let Some(stream) = stderr {
        output_join_handles.push(spawn_output_reader(
            app.clone(),
            task_id.clone(),
            stream,
            Some(Arc::clone(&output_tail)),
        ));
    }

    spawn_process_monitor(ProcessMonitorArgs {
        app,
        state: state.refs(),
        task_id: task_id.clone(),
        record_ids,
        child,
        cleanup_file: None,
        output_tail,
        output_join_handles,
        database_guard,
    });
    child_guard.disarm();
    history_guard.disarm();

    Ok(DownloadStarted {
        task_id,
        command_preview: preview_command(&tdl_path.to_string_lossy(), &args),
        records,
    })
}

#[tauri::command]
pub fn cancel_download(task_id: String, state: State<'_, AppState>) -> Result<(), String> {
    lock(&state.cancelled)?.insert(task_id.clone());

    let child = {
        let running = lock(&state.running)?;
        running.get(&task_id).cloned()
    };

    if let Some(child) = child {
        let mut child = lock(&child)?;
        child
            .kill()
            .map_err(|error| format!("取消下载失败: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    let mut history = lock(&state.history)?;
    history.clear();
    state.persist_history(&history)
}

#[tauri::command]
pub fn get_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticsSnapshot, String> {
    diagnostics_snapshot(&app, &state)
}

#[tauri::command]
pub fn collect_logs(state: State<'_, AppState>) -> Result<LogPackageInfo, String> {
    let config = lock(&state.config)?.clone();
    let log_dir = configured_log_dir(&state, &config);
    fs::create_dir_all(&log_dir).map_err(|error| format!("无法创建日志目录: {error}"))?;

    let file_name = format!(
        "tdl-desktop-logs-{}.txt",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let output_path = log_dir.join(&file_name);
    let content = build_sanitized_log_report(&state, &config)?;
    fs::write(&output_path, content).map_err(|error| format!("写入日志包失败: {error}"))?;
    let size = fs::metadata(&output_path)
        .map(|item| item.len())
        .unwrap_or(0);

    Ok(LogPackageInfo {
        path: output_path.to_string_lossy().to_string(),
        file_name,
        size,
        message: "日志包已生成，可手动提交给开发者分析。".into(),
    })
}

#[tauri::command]
pub fn check_desktop_update(state: State<'_, AppState>) -> Result<DesktopUpdateStatus, String> {
    let config = lock(&state.config)?.clone();
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    if config.desktop_update_url.trim().is_empty() {
        return Ok(DesktopUpdateStatus {
            configured: false,
            update_available: false,
            current_version,
            latest_version: None,
            message: "TDL Desktop 更新地址未配置。".into(),
        });
    }

    Ok(DesktopUpdateStatus {
        configured: true,
        update_available: false,
        current_version,
        latest_version: None,
        message: "TDL Desktop 更新检查暂未启用。".into(),
    })
}

#[tauri::command]
pub fn update_tdl(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let guard = UpdateRunningGuard::claim(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.try_state::<AppState>()
                .ok_or_else(|| "应用状态不可用。".to_string())
                .and_then(|app_state| update_tdl_impl(&app, app_state.inner()))
        }));

        let event = match result {
            Ok(Ok(info)) => TdlUpdateEvent {
                status: TdlUpdateStatus::Completed,
                tdl: Some(info),
                message: "tdl 已更新。".into(),
            },
            Ok(Err(error)) => TdlUpdateEvent {
                status: TdlUpdateStatus::Failed,
                tdl: None,
                message: error,
            },
            Err(_) => TdlUpdateEvent {
                status: TdlUpdateStatus::Failed,
                tdl: None,
                message: "tdl 更新过程发生未预期错误。".into(),
            },
        };
        let _ = app.emit("tdl-update-event", event);
    });

    Ok(())
}

pub(crate) fn ensure_tdl_update_not_running(state: &AppState, message: &str) -> Result<(), String> {
    if *lock(&state.tdl_update_running)? {
        return Err(message.to_string());
    }
    Ok(())
}

pub(crate) struct OperationGuard<'a> {
    _update_running: std::sync::MutexGuard<'a, bool>,
}

impl<'a> OperationGuard<'a> {
    pub(crate) fn download(state: &'a AppState) -> Result<Self, String> {
        Self::new(state, "tdl 正在更新，请等待更新完成后再开始下载。")
    }

    pub(crate) fn login(state: &'a AppState) -> Result<Self, String> {
        Self::new(state, "tdl 正在更新，请等待更新完成后再登录。")
    }

    fn new(state: &'a AppState, message: &str) -> Result<Self, String> {
        let update_running = lock(&state.tdl_update_running)?;
        if *update_running {
            return Err(message.to_string());
        }
        Ok(Self {
            _update_running: update_running,
        })
    }
}

struct UpdateRunningGuard {
    running: Arc<Mutex<bool>>,
}

impl UpdateRunningGuard {
    fn claim(state: &AppState) -> Result<Self, String> {
        {
            let mut running = lock(&state.tdl_update_running)?;
            if *running {
                return Err("正在检查 tdl 更新，请稍候。".into());
            }
            if !lock(&state.running)?.is_empty() {
                return Err("当前还有下载任务在执行，请等任务结束或取消后再更新 tdl。".into());
            }
            if lock(&state.login)?.is_some() {
                return Err("当前有登录流程在运行，请先取消或等待完成。".into());
            }
            *running = true;
        }
        Ok(Self {
            running: Arc::clone(&state.tdl_update_running),
        })
    }
}

impl Drop for UpdateRunningGuard {
    fn drop(&mut self) {
        match self.running.lock() {
            Ok(mut running) => *running = false,
            Err(_) => eprintln!("[tdl] tdl update state lock poisoned while clearing update flag"),
        }
    }
}

fn configured_log_dir(state: &AppState, config: &AppConfig) -> PathBuf {
    if config.log_directory.trim().is_empty() {
        state.default_log_dir()
    } else {
        PathBuf::from(config.log_directory.trim())
    }
}

fn build_sanitized_log_report(state: &AppState, config: &AppConfig) -> Result<String, String> {
    let history = lock(&state.history)?.clone();
    let mut lines = vec![
        "TDL Desktop Diagnostic Log".to_string(),
        format!("generated_at={}", Utc::now().to_rfc3339()),
        format!("app_version={}", env!("CARGO_PKG_VERSION")),
        format!("language={}", redact_support_text(&config.language)),
        format!("download_limit={}", config.limit),
        format!("download_threads={}", config.threads),
        format!("download_pool={}", config.pool),
        format!("history_count={}", history.len()),
        String::new(),
        "Recent history".to_string(),
    ];

    for record in history.iter().take(20) {
        lines.push(format!(
            "- status={:?} mode={:?} source={} directory={} error={}",
            record.status,
            record.mode,
            redact_support_text(&record.source),
            redact_support_text(&record.directory),
            redact_support_text(record.error.as_deref().unwrap_or("")),
        ));
    }

    Ok(lines.join("\n"))
}
