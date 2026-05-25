import type { TranslationKey } from "./i18n";
import type { AppLanguage, MediaPreviewState } from "./appTypes";
import type { DownloadStatus, LoginStatus, MediaKind, MessageInfo, SourceMode, TdlInfo } from "./types";

export const statusLabel: Record<DownloadStatus, TranslationKey> = {
  downloading: "downloading",
  completed: "completed",
  failed: "failed",
  cancelled: "cancelled",
};

export const modeLabel: Record<SourceMode | "history" | "settings" | "docs", TranslationKey> = {
  links: "links",
  json: "json",
  raw: "raw",
  chat: "chat",
  history: "history",
  settings: "settings",
  docs: "operationDoc",
};

export function formatDate(value: string | null | undefined, language: AppLanguage) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(language === "en" ? "en-US" : "zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatFileSize(value: number | null | undefined, t: (key: TranslationKey) => string) {
  if (!value || value <= 0) return t("unknownSize");
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

export function mediaKindLabel(kind: MediaKind, t: (key: TranslationKey) => string) {
  const labels: Record<MediaKind, TranslationKey> = {
    none: "message",
    photo: "photo",
    video: "video",
    audio: "audio",
    document: "document",
    unknown: "media",
  };
  return t(labels[kind] ?? "media");
}

export function canPreviewMedia(kind: MediaKind) {
  return kind === "photo" || kind === "video";
}

export function shouldAutoDownloadPreview(message: MessageInfo) {
  if (!message.previewable || !canPreviewMedia(message.mediaKind)) return false;
  if (message.mediaKind === "photo") {
    return !message.fileSize || message.fileSize <= 20 * 1024 * 1024;
  }
  if (message.mediaKind === "video") {
    return Boolean(message.fileSize && message.fileSize <= 30 * 1024 * 1024);
  }
  return false;
}

export function skippedPreviewState(message: MessageInfo): MediaPreviewState {
  if (message.mediaKind === "video") return { status: "skipped", reason: "videoTooLarge" };
  if (message.mediaKind === "photo") return { status: "skipped", reason: "tooLarge" };
  return { status: "skipped", reason: "unsupported" };
}

export function previewButtonLabel(message: MessageInfo, state: MediaPreviewState, t: (key: TranslationKey) => string) {
  if (state.status === "loading") return t("loading");
  if (message.mediaKind === "video") {
    return state.status === "ready" ? t("refreshPreview") : t("loadPreview");
  }
  return state.status === "ready" ? t("refreshThumbnail") : t("loadThumbnail");
}

export function loggedInLabel(status: LoginStatus, t: (key: TranslationKey) => string) {
  if (status.username) return `${t("loggedInPrefix")}@${status.username}`;
  if (status.displayName) return `${t("loggedInPrefix")}${status.displayName}`;
  return status.message;
}

export function tdlSourceLabel(info: TdlInfo | null, t: (key: TranslationKey) => string) {
  if (!info || !info.available) return t("tdlUnavailable");
  const source =
    info.source === "bundled"
      ? t("tdlSourceBundled")
      : info.source === "updated"
        ? t("tdlSourceUpdated")
        : "PATH";
  return `${source} ${info.version ?? ""}`.trim();
}
