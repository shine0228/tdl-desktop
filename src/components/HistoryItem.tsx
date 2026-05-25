import { AlertCircle, CheckCircle2, Download, XCircle } from "lucide-react";

import type { AppLanguage } from "../appTypes";
import { formatDate, modeLabel, statusLabel } from "../appUtils";
import type { TranslationKey } from "../i18n";
import type { DownloadRecord } from "../types";

export function HistoryItem({
  record,
  language,
  t,
}: {
  record: DownloadRecord;
  language: AppLanguage;
  t: (key: TranslationKey) => string;
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
      </div>
    </article>
  );
}
