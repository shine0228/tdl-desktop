import { AlertCircle, LogIn, LoaderCircle, QrCode, ShieldCheck, Square } from "lucide-react";

import { loggedInLabel } from "../appUtils";
import { TRANSLATIONS, type TranslationKey } from "../i18n";
import type { LoginStatus } from "../types";

export function LoginPanel({
  status,
  checking,
  running,
  qr,
  logs,
  desktopPath,
  desktopPasscode,
  disabled,
  onDesktopPathChange,
  onDesktopPasscodeChange,
  onRefresh,
  onDesktopLogin,
  onQrLogin,
  onCancel,
  onLogout,
  t,
}: {
  status: LoginStatus;
  checking: boolean;
  running: boolean;
  qr: string;
  logs: string[];
  desktopPath: string;
  desktopPasscode: string;
  disabled: boolean;
  onDesktopPathChange: (value: string) => void;
  onDesktopPasscodeChange: (value: string) => void;
  onRefresh: () => void;
  onDesktopLogin: () => void;
  onQrLogin: () => void;
  onCancel: () => void;
  onLogout: () => void;
  t: (key: TranslationKey) => string;
}) {
  const StatusIcon = status.loggedIn ? ShieldCheck : running || checking ? LoaderCircle : AlertCircle;
  const statusMessage = !status.loggedIn && (status.message === TRANSLATIONS.zh.loginUnchecked || status.message === TRANSLATIONS.en.loginUnchecked)
    ? t("loginUnchecked")
    : status.message;

  return (
    <div className="login-panel">
      <div className="section-header">
        <h2>{t("telegramLogin")}</h2>
        <button className="text-button" onClick={onRefresh} disabled={checking || running}>
          {checking ? t("checking") : t("check")}
        </button>
      </div>

      <div className={`login-state ${status.loggedIn ? "ready" : "warning"} ${checking || running ? "loading" : ""}`}>
        <StatusIcon size={17} />
        <span>{status.loggedIn ? loggedInLabel(status, t) : statusMessage}</span>
      </div>
      {status.detail ? <p className="login-detail">{status.detail}</p> : null}

      {!status.loggedIn || running ? (
        <>
          <div className="login-fields">
            <label className="field compact">
              <span>{t("desktopDataDir")}</span>
              <input
                value={desktopPath}
                onChange={(event) => onDesktopPathChange(event.target.value)}
                placeholder={t("autoDetectBlank")}
              />
            </label>
            <label className="field compact">
              <span>{t("desktopLocalPassword")}</span>
              <input
                type="password"
                value={desktopPasscode}
                onChange={(event) => onDesktopPasscodeChange(event.target.value)}
                placeholder={t("blankNoPassword")}
              />
            </label>
          </div>

          <div className="login-actions">
            <button className="ghost-button" onClick={onDesktopLogin} disabled={disabled || running || checking}>
              <LogIn size={16} />
              {t("connectDesktop")}
            </button>
            <button className="ghost-button" onClick={onQrLogin} disabled={disabled || running || checking}>
              <QrCode size={16} />
              {t("qrLogin")}
            </button>
            <button className="danger-button" onClick={onCancel} disabled={!running}>
              <Square size={16} />
              {t("cancel")}
            </button>
          </div>
        </>
      ) : (
        <div className="login-actions">
          <button className="danger-button" onClick={onLogout} disabled={disabled || checking}>
            <LogIn size={16} />
            {t("logout")}
          </button>
        </div>
      )}

      {!status.loggedIn && qr ? <pre className="qr-box">{qr}</pre> : null}
      {!status.loggedIn && logs.length ? (
        <div className="login-log">
          {logs.map((line, index) => (
            <p key={`${line}-${index}`}>{line}</p>
          ))}
        </div>
      ) : null}
    </div>
  );
}
