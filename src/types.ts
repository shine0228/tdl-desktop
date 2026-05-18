export type SourceMode = "links" | "json" | "raw";

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
}

export interface AppState {
  config: AppConfig;
  history: DownloadRecord[];
  tdl: TdlInfo;
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
}

export type LoginMethod = "desktop" | "qr";

export interface LoginStatus {
  loggedIn: boolean;
  message: string;
  detail?: string | null;
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
