const APP_COMMANDS: &[&str] = &[
    "get_app_state",
    "save_config",
    "pick_directory",
    "pick_log_directory",
    "open_directory",
    "preview_download_command",
    "start_download",
    "cancel_download",
    "clear_history",
    "get_diagnostics",
    "collect_logs",
    "check_desktop_update",
    "refresh_tdl_info",
    "update_tdl",
    "clear_chat_cache",
    "list_chats",
    "export_chat_messages",
    "download_from_chat",
    "preview_chat_media",
    "cached_chat_media_preview",
    "check_login_status",
    "start_login",
    "cancel_login",
    "logout",
    "preview_link",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}
