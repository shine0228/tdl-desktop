import { AlertCircle, CheckCircle2, Download, XCircle, FolderOpen, Copy, CornerDownLeft, RotateCcw, PlayCircle } from "lucide-react";

import type { AppLanguage } from "../appTypes";
import { formatDate, modeLabel, statusLabel } from "../appUtils";
import type { TranslationKey } from "../i18n";
import type { DownloadRecord } from "../types";

export function HistoryItem({
  record,
  language,
  t,
  onOpenDirectory,
  onCopyError,
  onCopySource,
  onUseAsInput,
  onRetry,
  onContinue,
}: {
  record: DownloadRecord;
  language: AppLanguage;
  t: (key: TranslationKey) => string;
  onOpenDirectory?: (record: DownloadRecord) => void;
  onCopyError?: (record: DownloadRecord) => void;
  onCopySource?: (record: DownloadRecord) => void;
  onUseAsInput?: (record: DownloadRecord) => void;
  onRetry?: (record: DownloadRecord) => void;
  onContinue?: (record: DownloadRecord) => void;
}) {
  const Icon =
    record.status === "completed"
      ? CheckCircle2
      : record.status === "failed"
        ? XCircle
        : record.status === "cancelled"
          ? AlertCircle
          : Download;

  return (
    <article className="history-item">
      <div className={`history-icon ${record.status}`}>
        <Icon size={18} />
      </div>
      <div className="history-main">
        <div className="history-top">
          <strong>{record.source}</strong>
          <span className={`pill ${record.status}`}>{t(statusLabel[record.status])}</span>
        </div>
        <div className="history-meta">
          <span>{t(modeLabel[record.mode])}</span>
          <span>{record.directory}</span>
          <span>{formatDate(record.completedAt ?? record.createdAt, language)}</span>
        </div>
        {record.error ? <p>{record.error}</p> : null}
        {record.errorHint ? <p><strong>{t("errorHint")}:</strong> {record.errorHint}</p> : null}
        <div className="history-actions">
          {onRetry && record.request && (
            <button
              type="button"
              className="text-button"
              onClick={() => onRetry(record)}
              title={t("retryDownload")}
            >
              <RotateCcw size={14} />
              {t("retryDownload")}
            </button>
          )}
          {onContinue && record.request && (record.status === "failed" || record.status === "cancelled") && (
            <button
              type="button"
              className="text-button"
              onClick={() => onContinue(record)}
              title={t("continueDownload")}
            >
              <PlayCircle size={14} />
              {t("continueDownload")}
            </button>
          )}
          {onOpenDirectory && record.directory && record.mode !== "raw" && (
            <button
              type="button"
              className="text-button"
              onClick={() => onOpenDirectory(record)}
              title={t("openDirectory")}
            >
              <FolderOpen size={14} />
              {t("openDirectory")}
            </button>
          )}
          {onCopyError && (record.error || record.errorHint) && (
            <button
              type="button"
              className="text-button"
              onClick={() => onCopyError(record)}
              title={t("copyError")}
            >
              <Copy size={14} />
              {t("copyError")}
            </button>
          )}
          {onCopySource && (
            <button
              type="button"
              className="text-button"
              onClick={() => onCopySource(record)}
              title={t("copySource")}
            >
              <Copy size={14} />
              {t("copySource")}
            </button>
          )}
          {onUseAsInput && (record.mode === "links" || record.mode === "json") && (
            <button
              type="button"
              className="text-button"
              onClick={() => onUseAsInput(record)}
              title={t("useAsInput")}
            >
              <CornerDownLeft size={14} />
              {t("useAsInput")}
            </button>
          )}
        </div>
      </div>
    </article>
  );
}
