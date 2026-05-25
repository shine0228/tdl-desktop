mod chat;
mod commands;
mod download;
mod login;
mod preview;
mod state;
mod tdl;
mod tdl_config;
mod types;
mod util;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().unwrap_or_else(|error| {
        eprintln!("[tdl-desktop] 初始化应用状态失败: {error}");
        std::process::exit(1);
    });
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::save_config,
            commands::pick_directory,
            commands::pick_log_directory,
            commands::preview_download_command,
            commands::start_download,
            commands::cancel_download,
            commands::clear_history,
            commands::collect_logs,
            commands::check_desktop_update,
            commands::refresh_tdl_info,
            commands::update_tdl,
            chat::clear_chat_cache,
            chat::list_chats,
            chat::export_chat_messages,
            chat::download_from_chat,
            chat::preview_chat_media,
            chat::cached_chat_media_preview,
            login::check_login_status,
            login::start_login,
            login::cancel_login,
            login::logout,
            preview::preview_link,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TDL Desktop");
}
