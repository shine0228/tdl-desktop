mod commands;
mod download;
mod login;
mod preview;
mod state;
mod tdl;
mod types;
mod util;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::save_config,
            commands::pick_directory,
            commands::preview_download_command,
            commands::start_download,
            commands::cancel_download,
            commands::clear_history,
            commands::refresh_tdl_info,
            commands::update_tdl,
            login::check_login_status,
            login::start_login,
            login::cancel_login,
            preview::preview_link,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TDL Desktop");
}
