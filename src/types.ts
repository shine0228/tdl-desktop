export type SourceMode = "links" | "json" | "raw" | "chat";
export type AppLanguage = "zh" | "en";

export type DownloadStatus =
  | "downloading"
  | "completed"
  | "failed"
  | "cancelled";

export interface AppConfig {
  lastDirectory: string;
  limit: number;
  threads: number;
  pool: number;
  tdlOverridePath?: string | null;
  language?: AppLanguage;
  logDirectory?: string;
  desktopUpdateUrl?: string;
  tdlNamespace?: string;
  tdlStorage?: string;
}

export interface TdlInfo {
  available: boolean;
  version?: string | null;
  path?: string | null;
  source: "bundled" | "updated" | "path" | "missing";
}

export interface LinkPreview {
  link: string;
  chat: string;
  messageId: number;
  text?: string | null;
  mediaCount: number;
}

export type ErrorCategory =
  | "missingTdl"
  | "tdlNotRunnable"
  | "notLoggedIn"
  | "networkTimeout"
  | "permissionDenied"
  | "invalidInput"
  | "directoryNotWritable"
  | "databaseBusy"
  | "interrupted"
  | "cancelled"
  | "unknown";

export interface ClassifiedError {
  category: ErrorCategory;
  title: string;
  message: string;
  redactedDetail?: string | null;
}

export type DiagnosticSeverity = "info" | "warning" | "blocker";
export type DiagnosticStatus = "ok" | "warning" | "error" | "skipped";
export type DiagnosticOverall = "ready" | "needsAttention" | "blocked";
export type DiagnosticActionKind =
  | "refreshTdlInfo"
  | "updateTdl"
  | "checkLogin"
  | "chooseDirectory"
  | "openDiagnostics"
  | "collectLogs"
  | "retryDownload"
  | "continueDownload";

export interface DiagnosticAction {
  kind: DiagnosticActionKind;
  label: string;
  detail?: string | null;
}

export interface DiagnosticCheck {
  id: string;
  scope: string;
  label: string;
  severity: DiagnosticSeverity;
  status: DiagnosticStatus;
  summary: string;
  detail?: string | null;
  action?: DiagnosticAction | null;
}

export interface HistoryHealth {
  status: DiagnosticStatus;
  totalRecords: number;
  staleDownloadingCount: number;
  missingRequestCount: number;
  warning?: string | null;
}

export interface DiagnosticsSnapshot {
  generatedAt: string;
  overall: DiagnosticOverall;
  checks: DiagnosticCheck[];
  historyHealth: HistoryHealth;
}

export interface DownloadRecord {
  id: string;
  taskId: string;
  source: string;
  mode: SourceMode;
  directory: string;
  status: DownloadStatus;
  createdAt: string;
  completedAt?: string | null;
  error?: string | null;
  errorCategory?: ErrorCategory | null;
  errorHint?: string | null;
  request?: DownloadRequest | ChatDownloadRequest;
}

export interface AppState {
  config: AppConfig;
  history: DownloadRecord[];
  tdl: TdlInfo;
  desktopVersion: string;
}

export interface DownloadRequest {
  mode: SourceMode;
  directory: string;
  links: string[];
  files: string[];
  rawArgs: string;
  limit: number;
  threads: number;
  pool: number;
  group: boolean;
  include: string;
  exclude: string;
  template: string;
  skipSame: boolean;
  continueLast: boolean;
  restart: boolean;
  desc: boolean;
  takeout: boolean;
  rewriteExt: boolean;
}

export interface ChatDownloadRequest {
  chatId: string;
  chatName: string;
  messageIds: number[];
  directory: string;
  limit: number;
  threads: number;
  pool: number;
  group: boolean;
  include: string;
  exclude: string;
  template: string;
  skipSame: boolean;
  continueLast: boolean;
  restart: boolean;
  desc: boolean;
  takeout: boolean;
  rewriteExt: boolean;
}

export interface ChatInfo {
  id: number;
  name: string;
  chatType: string;
  username?: string | null;
}

export interface MessageInfo {
  id: number;
  date?: string | null;
  text?: string | null;
  mediaKind: MediaKind;
  mediaType?: string | null;
  mimeType?: string | null;
  fileName?: string | null;
  fileSize?: number | null;
  width?: number | null;
  height?: number | null;
  duration?: number | null;
  previewable: boolean;
}

export type MediaKind = "none" | "photo" | "video" | "audio" | "document" | "unknown";

export interface ChatMediaPreview {
  chatId: string;
  messageId: number;
  files: ChatMediaPreviewFile[];
}

export interface ChatMediaPreviewFile {
  path: string;
  fileName: string;
  mediaKind: MediaKind;
  mimeType?: string | null;
  size?: number | null;
}

export interface DownloadStarted {
  taskId: string;
  commandPreview: string;
  records: DownloadRecord[];
}

export interface DownloadFileProgress {
  key: string;
  name: string;
  progress: number;
  done: boolean;
}

export interface DownloadEvent {
  taskId: string;
  kind: "output" | "complete";
  line?: string | null;
  progress?: number | null;
  fileProgress?: DownloadFileProgress | null;
  status?: DownloadStatus | null;
  message?: string | null;
  recordIds: string[];
  completedAt?: string | null;
  error?: string | null;
  errorCategory?: ErrorCategory | null;
  errorHint?: string | null;
}

export type LoginMethod = "desktop" | "qr";

export interface LoginStatus {
  loggedIn: boolean;
  message: string;
  detail?: string | null;
  username?: string | null;
  displayName?: string | null;
}

export interface LoginRequest {
  method: LoginMethod;
  desktopPath?: string | null;
  passcode?: string | null;
}

export interface LoginStarted {
  loginId: string;
  method: LoginMethod;
}

export type LoginResultStatus = "completed" | "failed" | "cancelled";

export interface LoginEvent {
  loginId: string;
  kind: "output" | "qr" | "complete";
  line?: string | null;
  qr?: string | null;
  status?: LoginResultStatus | null;
  message?: string | null;
  error?: string | null;
}

export type TdlUpdateStatus = "completed" | "failed";

export interface TdlUpdateEvent {
  status: TdlUpdateStatus;
  tdl?: TdlInfo | null;
  message: string;
}

export interface LogPackageInfo {
  path: string;
  fileName: string;
  size: number;
  message: string;
}

export interface DesktopUpdateStatus {
  configured: boolean;
  updateAvailable: boolean;
  currentVersion: string;
  latestVersion?: string | null;
  message: string;
}
