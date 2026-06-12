import { Activity, File, FolderOpen, RotateCw, Trash2 } from "lucide-react";

import { type TranslationKey } from "../i18n";
import type { AppConfig, DesktopUpdateStatus, DiagnosticsSnapshot, DiagnosticStatus, LogPackageInfo, TdlInfo } from "../types";

export function SettingsWorkspace({
  config,
  tdl,
  t,
  logPackage,
  diagnostics,
  diagnosticsLoading,
  desktopUpdateStatus,
  desktopVersion,
  desktopUpdateChecking,
  tdlUpdateChecking,
  tdlUpdating,
  onSaveConfig,
  onPickLogDirectory,
  onCollectLogs,
  onRefreshDiagnostics,
  onCheckDesktopUpdate,
  onCheckTdlUpdate,
  onUpdateTdl,
  onClearCache,
}: {
  config: AppConfig;
  tdl: TdlInfo | null;
  t: (key: TranslationKey) => string;
  logPackage: LogPackageInfo | null;
  diagnostics: DiagnosticsSnapshot | null;
  diagnosticsLoading: boolean;
  desktopUpdateStatus: DesktopUpdateStatus | null;
  desktopVersion: string;
  desktopUpdateChecking: boolean;
  tdlUpdateChecking: boolean;
  tdlUpdating: boolean;
  onSaveConfig: (next: AppConfig) => Promise<void>;
  onPickLogDirectory: () => void;
  onCollectLogs: () => void;
  onRefreshDiagnostics: () => void;
  onCheckDesktopUpdate: () => void;
  onCheckTdlUpdate: () => void;
  onUpdateTdl: () => void;
  onClearCache: () => void;
}) {
  const language = config.language === "en" ? "en" : "zh";
  const currentDesktopVersion = desktopUpdateStatus?.currentVersion || desktopVersion || t("notConfigured");
  const diagnosticsOverallLabel = diagnostics
    ? diagnostics.overall === "ready"
      ? t("diagnosticsOverallReady")
      : diagnostics.overall === "blocked"
        ? t("diagnosticsOverallBlocked")
        : t("diagnosticsOverallNeedsAttention")
    : t("notConfigured");
  const diagnosticCounts = diagnostics?.checks.reduce(
    (counts, check) => {
      counts[check.status] += 1;
      return counts;
    },
    { ok: 0, warning: 0, error: 0, skipped: 0 } satisfies Record<DiagnosticStatus, number>,
  );
  return (
    <section className="settings-page">
      <div className="section-header">
        <h2>{t("settingsTitle")}</h2>
      </div>

      <div className="settings-grid">
        <section className="settings-card diagnostics-card">
          <div className="section-header">
            <h3>{t("diagnostics")}</h3>
            <span className={`diagnostic-overall ${diagnostics?.overall ?? "unknown"}`}>{diagnosticsOverallLabel}</span>
          </div>
          <div className="diagnostic-summary">
            <div><strong>{diagnosticCounts?.ok ?? 0}</strong><span>{t("diagnosticsOk")}</span></div>
            <div><strong>{diagnosticCounts?.warning ?? 0}</strong><span>{t("diagnosticsWarnings")}</span></div>
            <div><strong>{diagnosticCounts?.error ?? 0}</strong><span>{t("diagnosticsBlockers")}</span></div>
          </div>
          {diagnostics?.historyHealth ? (
            <p className="settings-help">
              {t("historyHealth")}: {diagnostics.historyHealth.totalRecords} · {t("staleDownloading")}: {diagnostics.historyHealth.staleDownloadingCount}
              {diagnostics.historyHealth.warning ? ` · ${diagnostics.historyHealth.warning}` : ""}
            </p>
          ) : null}
          <div className="diagnostic-check-list">
            {diagnostics?.checks.slice(0, 8).map((check) => (
              <article key={check.id} className={`diagnostic-check ${check.status}`}>
                <div>
                  <strong>{check.label}</strong>
                  <p>{check.summary}</p>
                  {check.detail ? <code>{check.detail}</code> : null}
                </div>
                <span>{check.status}</span>
              </article>
            )) ?? <p className="settings-help">{t("diagnosticsNotRun")}</p>}
          </div>
          <div className="action-row">
            <button className="ghost-button" type="button" onClick={onRefreshDiagnostics} disabled={diagnosticsLoading}>
              <Activity size={16} />
              {diagnosticsLoading ? "..." : t("refreshDiagnostics")}
            </button>
            <button className="primary-button" type="button" onClick={onCollectLogs}>
              <File size={16} />
              {t("collectLogs")}
            </button>
          </div>
          <p className="settings-help">{t("diagnosticsPrivacy")}</p>
        </section>

        <section className="settings-card">
          <h3>{t("general")}</h3>
          <label className="field compact">
            <span>{t("language")}</span>
            <select value={language} onChange={(event) => void onSaveConfig({ ...config, language: event.target.value as "zh" | "en" })}>
              <option value="zh">{t("chinese")}</option>
              <option value="en">{t("english")}</option>
            </select>
          </label>
        </section>

        <section className="settings-card">
          <h3>{t("logs")}</h3>
          <label className="field compact">
            <span>{t("logDirectory")}</span>
            <input value={config.logDirectory || ""} onChange={(event) => void onSaveConfig({ ...config, logDirectory: event.target.value })} />
          </label>
          <div className="action-row">
            <button className="ghost-button" type="button" onClick={onPickLogDirectory}>
              <FolderOpen size={16} />
              {t("chooseLogDirectory")}
            </button>
            <button className="primary-button" type="button" onClick={onCollectLogs}>
              <File size={16} />
              {t("collectLogs")}
            </button>
          </div>
          <p className="settings-help">{t("logPrivacy")}</p>
          {logPackage ? <code className="settings-path">{logPackage.path}</code> : null}
        </section>

        <section className="settings-card">
          <h3>{t("tdlInfo")}</h3>
          <div className="settings-kv"><span>{t("version")}</span><strong>{tdl?.version ?? t("notConfigured")}</strong></div>
          <div className="settings-kv"><span>{t("path")}</span><code>{tdl?.path ?? t("notConfigured")}</code></div>
          <div className="action-row">
            <button className="ghost-button" type="button" onClick={onCheckTdlUpdate} disabled={tdlUpdateChecking || tdlUpdating}>
              <RotateCw size={16} />
              {tdlUpdateChecking ? "..." : t("checkTdlUpdate")}
            </button>
            <button className="primary-button" type="button" onClick={onUpdateTdl} disabled={tdlUpdateChecking || tdlUpdating || !tdl?.available}>
              <RotateCw size={16} />
              {tdlUpdating ? t("updatingTdl") : t("updateTdl")}
            </button>
          </div>
          <label className="field compact">
            <span>{t("tdlNamespace")}</span>
            <input value={config.tdlNamespace || "default"} onChange={(event) => void onSaveConfig({ ...config, tdlNamespace: event.target.value })} />
          </label>
          <label className="field compact">
            <span>{t("tdlStorage")}</span>
            <input value={config.tdlStorage || ""} onChange={(event) => void onSaveConfig({ ...config, tdlStorage: event.target.value })} placeholder="type=bolt,path=C:\\Users\\me\\.tdl\\data" />
          </label>
          <p className="settings-help">{t("tdlStorageHelp")}</p>
        </section>

        <section className="settings-card">
          <h3>{t("cache")}</h3>
          <button className="danger-button" type="button" onClick={onClearCache}>
            <Trash2 size={16} />
            {t("clearCache")}
          </button>
          <p className="settings-help">{t("clearCacheHelp")}</p>
        </section>

        <section className="settings-card">
          <h3>{t("desktopUpdate")}</h3>
          <div className="settings-kv"><span>{t("currentVersion")}</span><strong>{currentDesktopVersion}</strong></div>
          <button className="ghost-button" type="button" onClick={onCheckDesktopUpdate} disabled={desktopUpdateChecking}>
            <RotateCw size={16} />
            {desktopUpdateChecking ? "..." : t("checkDesktopUpdate")}
          </button>
          {desktopUpdateStatus ? <p className="settings-help">{desktopUpdateStatus.message}</p> : null}
        </section>
      </div>
    </section>
  );
}
