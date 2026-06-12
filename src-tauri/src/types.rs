use serde::{Deserialize, Serialize};

fn default_language() -> String {
    "zh".into()
}

fn default_tdl_namespace() -> String {
    "default".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub last_directory: String,
    pub limit: u8,
    pub threads: u8,
    pub pool: u8,
    pub tdl_override_path: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub log_directory: String,
    #[serde(default)]
    pub desktop_update_url: String,
    #[serde(default = "default_tdl_namespace")]
    pub tdl_namespace: String,
    #[serde(default)]
    pub tdl_storage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub config: AppConfig,
    pub history: Vec<DownloadRecord>,
    pub tdl: TdlInfo,
    pub desktop_version: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<ErrorCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCategory {
    MissingTdl,
    TdlNotRunnable,
    NotLoggedIn,
    NetworkTimeout,
    PermissionDenied,
    InvalidInput,
    DirectoryNotWritable,
    DatabaseBusy,
    Interrupted,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifiedError {
    pub category: ErrorCategory,
    pub title: String,
    pub message: String,
    pub redacted_detail: Option<String>,
}

impl ClassifiedError {
    pub fn from_message(message: &str) -> Self {
        let lower = message.to_ascii_lowercase();
        let (category, title, hint) = if contains_any(
            &lower,
            &[
                "用户取消",
                "cancelled",
                "canceled",
                "operation cancelled",
                "context canceled",
            ],
        ) {
            (
                ErrorCategory::Cancelled,
                "任务已取消",
                "任务已被用户取消。如需继续，请从历史记录重新开始或继续下载。",
            )
        } else if contains_any(
            &lower,
            &[
                "未找到可用的 tdl",
                "tdl 路径不可用",
                "tdl not found",
                "tdl executable not found",
                "missing tdl",
                "cannot find tdl",
            ],
        ) || ((lower.contains("tdl.exe") || lower.contains("tdl "))
            && contains_any(
                &lower,
                &["no such file", "not found", "系统找不到指定的文件"],
            ))
        {
            (
                ErrorCategory::MissingTdl,
                "tdl 不可用",
                "请刷新 tdl 信息，或在设置中重新下载/更新 tdl。",
            )
        } else if contains_any(
            &lower,
            &[
                "not executable",
                "无法启动 tdl",
                "启动 tdl",
                "spawn",
                "不是有效的 win32",
                "exec format",
            ],
        ) {
            (
                ErrorCategory::TdlNotRunnable,
                "tdl 无法运行",
                "请确认 tdl 文件完整且可执行，必要时重新更新 tdl。",
            )
        } else if contains_any(
            &lower,
            &[
                "not logged in",
                "login required",
                "unauthorized",
                "forbidden",
                "401",
                "403",
                "auth",
                "登录",
                "未登录",
            ],
        ) {
            (
                ErrorCategory::NotLoggedIn,
                "登录状态不可用",
                "请在登录区域重新检查 Telegram 登录状态后再试。",
            )
        } else if contains_any(
            &lower,
            &[
                "database is locked",
                "database locked",
                "database busy",
                "resource busy",
                "另一个 tdl",
                "数据库",
                "被占用",
            ],
        ) {
            (
                ErrorCategory::DatabaseBusy,
                "tdl 数据库忙碌",
                "请等待当前 tdl 操作结束，或稍后重试。",
            )
        } else if contains_any(
            &lower,
            &[
                "timeout",
                "timed out",
                "deadline exceeded",
                "connection reset",
                "connection refused",
                "network",
                "dns",
                "proxy",
                "tls",
                "超时",
                "网络",
                "连接",
            ],
        ) {
            (
                ErrorCategory::NetworkTimeout,
                "网络或超时问题",
                "请检查网络、代理和 Telegram 连接状态后重试。",
            )
        } else if contains_any(
            &lower,
            &["permission denied", "access denied", "拒绝访问", "权限"],
        ) {
            (
                ErrorCategory::PermissionDenied,
                "权限不足",
                "请确认应用对下载目录和 tdl 文件有读取/写入权限。",
            )
        } else if contains_any(
            &lower,
            &[
                "无法创建下载目录",
                "directory",
                "path",
                "disk",
                "no space",
                "not writable",
                "目录",
                "路径",
                "磁盘",
            ],
        ) {
            (
                ErrorCategory::DirectoryNotWritable,
                "下载目录不可写",
                "请选择一个可写的本地普通目录后重试。",
            )
        } else if contains_any(
            &lower,
            &["invalid", "请输入", "请选择", "参数", "格式", "parse"],
        ) {
            (
                ErrorCategory::InvalidInput,
                "输入参数需要检查",
                "请检查下载链接、JSON 文件或原始参数后重试。",
            )
        } else if contains_any(&lower, &["interrupted", "broken pipe", "中断"]) {
            (
                ErrorCategory::Interrupted,
                "任务被中断",
                "请确认 tdl 进程和系统资源状态后重试。",
            )
        } else {
            (
                ErrorCategory::Unknown,
                "未知错误",
                "请查看错误详情；如需反馈，请生成并检查脱敏日志包。",
            )
        };

        let redacted_detail =
            (!message.trim().is_empty()).then(|| crate::redaction::redact_support_text(message));

        Self {
            category,
            title: title.into(),
            message: hint.into(),
            redacted_detail,
        }
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticStatus {
    Ok,
    Warning,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticActionKind {
    RefreshTdlInfo,
    UpdateTdl,
    CheckLogin,
    ChooseDirectory,
    OpenDiagnostics,
    CollectLogs,
    RetryDownload,
    ContinueDownload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticAction {
    pub kind: DiagnosticActionKind,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub scope: String,
    pub label: String,
    pub severity: DiagnosticSeverity,
    pub status: DiagnosticStatus,
    pub summary: String,
    pub detail: Option<String>,
    pub action: Option<DiagnosticAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticOverall {
    Ready,
    NeedsAttention,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHealth {
    pub status: DiagnosticStatus,
    pub total_records: usize,
    pub stale_downloading_count: usize,
    pub missing_request_count: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSnapshot {
    pub generated_at: String,
    pub overall: DiagnosticOverall,
    pub checks: Vec<DiagnosticCheck>,
    pub history_health: HistoryHealth,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<ErrorCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_hint: Option<String>,
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
pub struct LoginStatusRequest {
    #[serde(default)]
    pub verify_online: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPackageInfo {
    pub path: String,
    pub file_name: String,
    pub size: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopUpdateStatus {
    pub configured: bool,
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ClassifiedError, ErrorCategory};

    #[test]
    fn classifies_network_errors() {
        let error = ClassifiedError::from_message("request timeout while connecting to Telegram");
        assert_eq!(error.category, ErrorCategory::NetworkTimeout);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn classifies_auth_errors() {
        let error = ClassifiedError::from_message("401 unauthorized: login required");
        assert_eq!(error.category, ErrorCategory::NotLoggedIn);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn classifies_directory_errors() {
        let error = ClassifiedError::from_message("无法创建下载目录: disk is read-only");
        assert_eq!(error.category, ErrorCategory::DirectoryNotWritable);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn classifies_unknown_errors() {
        let error = ClassifiedError::from_message("unexpected tdl output");
        assert_eq!(error.category, ErrorCategory::Unknown);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn generic_missing_file_is_not_missing_tdl() {
        let error =
            ClassifiedError::from_message("open C:\\tmp\\input.json: no such file or directory");
        assert_eq!(error.category, ErrorCategory::DirectoryNotWritable);
    }

    #[test]
    fn missing_tdl_binary_is_missing_tdl() {
        let error = ClassifiedError::from_message("tdl.exe: no such file or directory");
        assert_eq!(error.category, ErrorCategory::MissingTdl);
    }

    #[test]
    fn redacts_classified_error_detail() {
        let error = ClassifiedError::from_message("password=super-secret-value");
        assert_eq!(error.category, ErrorCategory::Unknown);
        let detail = error.redacted_detail.expect("detail should be present");
        assert!(detail.contains("password=<SECRET>"));
        assert!(!detail.contains("super-secret-value"));
    }
}
