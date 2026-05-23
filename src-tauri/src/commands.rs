use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use chrono::Utc;
use tauri::{AppHandle, State};

use crate::{
    download::{build_download_args, build_records, spawn_output_reader, spawn_process_monitor},
    state::AppState,
    tdl::{resolve_tdl, update_tdl as update_tdl_impl},
    types::{AppConfig, AppSnapshot, DownloadRequest, DownloadStarted, SourceMode, TdlInfo},
    util::{apply_hidden_process_flags, lock, preview_command},
};

#[tauri::command]
pub fn get_app_state(app: AppHandle, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let config = lock(&state.config)?.clone();
    let history = lock(&state.history)?.clone();
    let tdl = resolve_tdl(&app, &state)?;

    Ok(AppSnapshot {
        config,
        history,
        tdl,
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
pub fn preview_download_command(request: DownloadRequest) -> Result<String, String> {
    let args = build_download_args(&request)?;
    Ok(preview_command("tdl", &args))
}

#[tauri::command]
pub fn refresh_tdl_info(app: AppHandle, state: State<'_, AppState>) -> Result<TdlInfo, String> {
    resolve_tdl(&app, &state)
}

#[tauri::command]
pub fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    request: DownloadRequest,
) -> Result<DownloadStarted, String> {
    let tdl = resolve_tdl(&app, &state)?;
    if !tdl.available {
        return Err("未找到可用的 tdl.exe。发布包应内置 tdl.exe,也可以手动更新 tdl。".into());
    }
    if request.mode == SourceMode::TgLite {
        return Err("TG Lite 模式请在 TG Lite 页面选择消息下载。".into());
    }

    let tdl_path = PathBuf::from(
        tdl.path
            .clone()
            .ok_or_else(|| "tdl 路径不可用".to_string())?,
    );
    let args = build_download_args(&request)?;

    if request.mode != SourceMode::Raw {
        fs::create_dir_all(request.directory.trim())
            .map_err(|error| format!("无法创建下载目录: {error}"))?;
    }

    let mut command = Command::new(&tdl_path);
    apply_hidden_process_flags(&mut command);
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 tdl 失败: {error}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let task_id = state.next_id("task");
    let created_at = Utc::now().to_rfc3339();
    let records = build_records(&state, &request, &task_id, &created_at)?;
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

    spawn_process_monitor(app, state.refs(), task_id.clone(), record_ids, child);

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
pub fn update_tdl(app: AppHandle, state: State<'_, AppState>) -> Result<TdlInfo, String> {
    update_tdl_impl(&app, &state)
}
