use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub last_directory: String,
    pub limit: u8,
    pub threads: u8,
    pub pool: u8,
    pub tdl_override_path: Option<String>,
    #[serde(default)]
    pub tg_lite_api_id: String,
    #[serde(default)]
    pub tg_lite_api_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub config: AppConfig,
    pub history: Vec<DownloadRecord>,
    pub tdl: TdlInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TdlInfo {
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub source: TdlSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TdlSource {
    Bundled,
    Updated,
    Path,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub mode: SourceMode,
    pub directory: String,
    pub links: Vec<String>,
    pub files: Vec<String>,
    pub raw_args: String,
    pub limit: u8,
    pub threads: u8,
    pub pool: u8,
    pub group: bool,
    pub include: String,
    pub exclude: String,
    pub template: String,
    pub skip_same: bool,
    pub continue_last: bool,
    pub restart: bool,
    pub desc: bool,
    pub takeout: bool,
    pub rewrite_ext: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDownloadRequest {
    pub chat_id: String,
    pub chat_name: String,
    pub message_ids: Vec<i64>,
    pub directory: String,
    pub limit: u8,
    pub threads: u8,
    pub pool: u8,
    pub group: bool,
    pub include: String,
    pub exclude: String,
    pub template: String,
    pub skip_same: bool,
    pub continue_last: bool,
    pub restart: bool,
    pub desc: bool,
    pub takeout: bool,
    pub rewrite_ext: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceMode {
    Links,
    Json,
    Raw,
    Chat,
    TgLite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadStatus {
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    pub id: String,
    pub task_id: String,
    pub source: String,
    pub mode: SourceMode,
    pub directory: String,
    pub status: DownloadStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStarted {
    pub task_id: String,
    pub command_preview: String,
    pub records: Vec<DownloadRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFileProgress {
    pub key: String,
    pub name: String,
    pub progress: f64,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvent {
    pub task_id: String,
    pub kind: DownloadEventKind,
    pub line: Option<String>,
    pub progress: Option<f64>,
    pub file_progress: Option<DownloadFileProgress>,
    pub status: Option<DownloadStatus>,
    pub message: Option<String>,
    pub record_ids: Vec<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadEventKind {
    Output,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub link: String,
    pub chat: String,
    pub message_id: u64,
    pub text: Option<String>,
    pub media_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatInfo {
    pub id: i64,
    pub name: String,
    pub chat_type: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageInfo {
    pub id: i64,
    pub date: Option<String>,
    pub text: Option<String>,
    pub media_kind: MediaKind,
    pub media_type: Option<String>,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub previewable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    None,
    Photo,
    Video,
    Audio,
    Document,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMediaPreview {
    pub chat_id: String,
    pub message_id: i64,
    pub files: Vec<ChatMediaPreviewFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMediaPreviewFile {
    pub path: String,
    pub file_name: String,
    pub media_kind: MediaKind,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TgLiteStatus {
    pub configured: bool,
    pub initialized: bool,
    pub authorized: bool,
    pub state: String,
    pub message: String,
    pub qr_link: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TgLiteChat {
    pub id: i64,
    pub title: String,
    pub chat_type: String,
    pub unread_count: i32,
    pub last_message_id: Option<i64>,
    pub last_message_text: Option<String>,
    pub order: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TgLiteEvent {
    Status { status: TgLiteStatus },
    Connection { state: String },
    ChatUpsert { chat: TgLiteChat },
    ChatDelete { chat_id: i64 },
    MessageNew { chat_id: i64, message: MessageInfo },
    MessageUpdate {
        chat_id: i64,
        message_id: i64,
        message: Option<MessageInfo>,
    },
    MessageDelete { chat_id: i64, message_ids: Vec<i64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    pub logged_in: bool,
    pub message: String,
    pub detail: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub method: LoginMethod,
    pub desktop_path: Option<String>,
    pub passcode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginMethod {
    Desktop,
    Qr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStarted {
    pub login_id: String,
    pub method: LoginMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginEvent {
    pub login_id: String,
    pub kind: LoginEventKind,
    pub line: Option<String>,
    pub qr: Option<String>,
    pub status: Option<LoginResultStatus>,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginEventKind {
    Output,
    Qr,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginResultStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TdlUpdateEvent {
    pub status: TdlUpdateStatus,
    pub tdl: Option<TdlInfo>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TdlUpdateStatus {
    Completed,
    Failed,
}

#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
}
