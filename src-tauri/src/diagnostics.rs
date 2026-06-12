use std::{fs, path::Path};

use chrono::Utc;
use tauri::AppHandle;

use crate::{
    redaction::redact_support_text,
    state::AppState,
    tdl::resolve_tdl,
    tdl_config::{normalize_namespace, session_targets},
    types::{
        DiagnosticAction, DiagnosticActionKind, DiagnosticCheck, DiagnosticOverall,
        DiagnosticSeverity, DiagnosticStatus, DiagnosticsSnapshot, DownloadStatus, HistoryHealth,
    },
    util::{lock, validate_download_dir},
};

pub fn diagnostics_snapshot(
    app: &AppHandle,
    state: &AppState,
) -> Result<DiagnosticsSnapshot, String> {
    let config = lock(&state.config)?.clone();
    let history = lock(&state.history)?.clone();
    let tdl = resolve_tdl(app, state)?;
    let mut checks = Vec::new();

    checks.push(path_check(
        "app-data",
        "startup",
        "应用数据目录",
        &state.app_dir,
        DiagnosticSeverity::Blocker,
        Some("应用需要在这里保存配置、历史、日志和缓存。"),
    ));

    checks.push(file_health_check(
        "config-file",
        "startup",
        "配置文件",
        &state.config_path(),
        "配置文件可读取，已加载当前设置。",
        "尚未创建配置文件，应用会在保存设置时自动生成。",
    ));

    checks.push(file_health_check(
        "history-file",
        "startup",
        "历史文件",
        &state.history_path(),
        "历史文件可读取。",
        "尚未创建历史文件，开始下载后会自动生成。",
    ));

    let tdl_detail = tdl.path.as_deref().map(redact_support_text);
    checks.push(if tdl.available {
        DiagnosticCheck {
            id: "tdl-binary".into(),
            scope: "startup".into(),
            label: "tdl 可执行文件".into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Ok,
            summary: format!(
                "tdl 可用{}。",
                tdl.version
                    .as_deref()
                    .map(|value| format!("，版本 {value}"))
                    .unwrap_or_default()
            ),
            detail: tdl_detail,
            action: None,
        }
    } else {
        DiagnosticCheck {
            id: "tdl-binary".into(),
            scope: "startup".into(),
            label: "tdl 可执行文件".into(),
            severity: DiagnosticSeverity::Blocker,
            status: DiagnosticStatus::Error,
            summary: "未找到可用的 tdl.exe。".into(),
            detail: Some("发布包通常会内置 tdl.exe，也可以在设置中更新 tdl。".into()),
            action: Some(action(
                DiagnosticActionKind::RefreshTdlInfo,
                "刷新 tdl 信息",
                Some("如果刚刚恢复或重新下载了 tdl.exe，请先刷新状态。"),
            )),
        }
    });

    checks.push(
        match validate_download_dir(&config.last_directory, &state.app_dir) {
            Ok(path) => DiagnosticCheck {
                id: "download-directory".into(),
                scope: "download".into(),
                label: "默认下载目录".into(),
                severity: DiagnosticSeverity::Info,
                status: DiagnosticStatus::Ok,
                summary: "默认下载目录格式有效。".into(),
                detail: Some(redact_support_text(&path.to_string_lossy())),
                action: None,
            },
            Err(error) => DiagnosticCheck {
                id: "download-directory".into(),
                scope: "download".into(),
                label: "默认下载目录".into(),
                severity: DiagnosticSeverity::Warning,
                status: DiagnosticStatus::Warning,
                summary: "默认下载目录需要处理。".into(),
                detail: Some(redact_support_text(&error)),
                action: Some(action(
                    DiagnosticActionKind::ChooseDirectory,
                    "选择下载目录",
                    Some("请选择一个普通本地目录，不要直接使用磁盘根目录或应用数据目录。"),
                )),
            },
        },
    );

    let log_dir = if config.log_directory.trim().is_empty() {
        state.default_log_dir()
    } else {
        config.log_directory.trim().into()
    };
    checks.push(path_parent_check(
        "log-directory",
        "diagnostics",
        "日志目录",
        &log_dir,
        DiagnosticSeverity::Warning,
        Some("诊断报告会保存到这个目录，应用不会自动上传。"),
    ));

    checks.push(match session_targets(&config) {
        Ok(targets) => DiagnosticCheck {
            id: "tdl-session-target".into(),
            scope: "login".into(),
            label: "tdl 登录数据".into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Ok,
            summary: format!(
                "当前命名空间：{}",
                normalize_namespace(&config.tdl_namespace)
            ),
            detail: Some(redact_support_text(
                &targets.namespace_dir.to_string_lossy(),
            )),
            action: Some(action(
                DiagnosticActionKind::CheckLogin,
                "检查登录状态",
                Some("在线确认登录状态需要访问 Telegram，可在右侧登录区域手动执行。"),
            )),
        },
        Err(error) => DiagnosticCheck {
            id: "tdl-session-target".into(),
            scope: "login".into(),
            label: "tdl 登录数据".into(),
            severity: DiagnosticSeverity::Warning,
            status: DiagnosticStatus::Warning,
            summary: "tdl 登录数据路径需要检查。".into(),
            detail: Some(redact_support_text(&error)),
            action: None,
        },
    });

    let running_count = lock(&state.running)?.len();
    let login_running = lock(&state.login)?.is_some();
    let update_running = *lock(&state.tdl_update_running)?;
    checks.push(operation_check(
        running_count,
        login_running,
        update_running,
    ));

    let history_health = HistoryHealth {
        status: if history
            .iter()
            .any(|record| record.status == DownloadStatus::Downloading)
        {
            DiagnosticStatus::Warning
        } else {
            DiagnosticStatus::Ok
        },
        total_records: history.len(),
        stale_downloading_count: history
            .iter()
            .filter(|record| record.status == DownloadStatus::Downloading)
            .count(),
        missing_request_count: history
            .iter()
            .filter(|record| record.request.is_none())
            .count(),
        warning: history
            .iter()
            .any(|record| record.status == DownloadStatus::Downloading)
            .then(|| {
                "历史中存在下载中记录。如应用刚重启，这些记录可能需要后续恢复处理。".to_string()
            }),
    };

    let overall = if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Error)
    {
        DiagnosticOverall::Blocked
    } else if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Warning)
        || history_health.status == DiagnosticStatus::Warning
    {
        DiagnosticOverall::NeedsAttention
    } else {
        DiagnosticOverall::Ready
    };

    Ok(DiagnosticsSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        overall,
        checks,
        history_health,
    })
}

fn action(kind: DiagnosticActionKind, label: &str, detail: Option<&str>) -> DiagnosticAction {
    DiagnosticAction {
        kind,
        label: label.into(),
        detail: detail.map(str::to_string),
    }
}

fn path_check(
    id: &str,
    scope: &str,
    label: &str,
    path: &Path,
    failure_severity: DiagnosticSeverity,
    detail: Option<&str>,
) -> DiagnosticCheck {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Ok,
            summary: "目录可用。".into(),
            detail: Some(redact_support_text(&path.to_string_lossy())),
            action: None,
        },
        Ok(_) => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: failure_severity,
            status: DiagnosticStatus::Error,
            summary: "路径不是目录。".into(),
            detail: Some(redact_support_text(&path.to_string_lossy())),
            action: None,
        },
        Err(error) => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: failure_severity,
            status: DiagnosticStatus::Error,
            summary: "目录不可用。".into(),
            detail: Some(format!(
                "{} · {}",
                redact_support_text(&path.to_string_lossy()),
                redact_support_text(&error.to_string())
            )),
            action: None,
        },
    }
    .with_fallback_detail(detail)
}

fn path_parent_check(
    id: &str,
    scope: &str,
    label: &str,
    path: &Path,
    failure_severity: DiagnosticSeverity,
    detail: Option<&str>,
) -> DiagnosticCheck {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            return DiagnosticCheck {
                id: id.into(),
                scope: scope.into(),
                label: label.into(),
                severity: DiagnosticSeverity::Info,
                status: DiagnosticStatus::Ok,
                summary: "目录配置可用。".into(),
                detail: Some(redact_support_text(&path.to_string_lossy())),
                action: None,
            }
            .with_fallback_detail(detail);
        }
        Ok(_) => {
            return DiagnosticCheck {
                id: id.into(),
                scope: scope.into(),
                label: label.into(),
                severity: failure_severity,
                status: DiagnosticStatus::Warning,
                summary: "路径不是目录。".into(),
                detail: Some(redact_support_text(&path.to_string_lossy())),
                action: None,
            }
            .with_fallback_detail(detail);
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return DiagnosticCheck {
                id: id.into(),
                scope: scope.into(),
                label: label.into(),
                severity: failure_severity,
                status: DiagnosticStatus::Warning,
                summary: "目录配置需要检查。".into(),
                detail: Some(redact_support_text(&format!(
                    "{} · {error}",
                    path.display()
                ))),
                action: None,
            }
            .with_fallback_detail(detail);
        }
        Err(_) => {}
    }

    if let Some(parent) = path.parent() {
        if fs::metadata(parent)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
        {
            return DiagnosticCheck {
                id: id.into(),
                scope: scope.into(),
                label: label.into(),
                severity: DiagnosticSeverity::Info,
                status: DiagnosticStatus::Ok,
                summary: "目录尚未创建，父目录可用。".into(),
                detail: Some(redact_support_text(&path.to_string_lossy())),
                action: None,
            }
            .with_fallback_detail(detail);
        }
    }

    DiagnosticCheck {
        id: id.into(),
        scope: scope.into(),
        label: label.into(),
        severity: failure_severity,
        status: DiagnosticStatus::Warning,
        summary: "目录配置需要检查。".into(),
        detail: Some(redact_support_text(&path.to_string_lossy())),
        action: None,
    }
    .with_fallback_detail(detail)
}

fn file_health_check(
    id: &str,
    scope: &str,
    label: &str,
    path: &Path,
    existing_summary: &str,
    missing_summary: &str,
) -> DiagnosticCheck {
    match fs::read_to_string(path) {
        Ok(_) => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Ok,
            summary: existing_summary.into(),
            detail: Some(redact_support_text(&path.to_string_lossy())),
            action: None,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Skipped,
            summary: missing_summary.into(),
            detail: Some(redact_support_text(&path.to_string_lossy())),
            action: None,
        },
        Err(error) => DiagnosticCheck {
            id: id.into(),
            scope: scope.into(),
            label: label.into(),
            severity: DiagnosticSeverity::Warning,
            status: DiagnosticStatus::Warning,
            summary: "文件暂时不可读取。".into(),
            detail: Some(redact_support_text(&format!(
                "{} · {error}",
                path.display()
            ))),
            action: Some(action(
                DiagnosticActionKind::CollectLogs,
                "生成诊断报告",
                Some("报告会脱敏本机路径和 Telegram 信息。"),
            )),
        },
    }
}

fn operation_check(
    running_count: usize,
    login_running: bool,
    update_running: bool,
) -> DiagnosticCheck {
    let mut details = Vec::new();
    if running_count > 0 {
        details.push(format!("下载任务 {running_count} 个"));
    }
    if login_running {
        details.push("登录流程正在运行".into());
    }
    if update_running {
        details.push("tdl 更新正在运行".into());
    }

    if details.is_empty() {
        DiagnosticCheck {
            id: "operation-state".into(),
            scope: "runtime".into(),
            label: "运行状态".into(),
            severity: DiagnosticSeverity::Info,
            status: DiagnosticStatus::Ok,
            summary: "当前没有互斥任务。".into(),
            detail: None,
            action: None,
        }
    } else {
        DiagnosticCheck {
            id: "operation-state".into(),
            scope: "runtime".into(),
            label: "运行状态".into(),
            severity: DiagnosticSeverity::Warning,
            status: DiagnosticStatus::Warning,
            summary: "当前有任务正在运行，部分操作需要等待。".into(),
            detail: Some(details.join("，")),
            action: None,
        }
    }
}

trait DiagnosticCheckExt {
    fn with_fallback_detail(self, fallback: Option<&str>) -> Self;
}

impl DiagnosticCheckExt for DiagnosticCheck {
    fn with_fallback_detail(mut self, fallback: Option<&str>) -> Self {
        if self.detail.is_none() {
            self.detail = fallback.map(str::to_string);
        } else if let Some(fallback) = fallback {
            if matches!(
                self.status,
                DiagnosticStatus::Ok | DiagnosticStatus::Skipped
            ) {
                let detail = self.detail.take().unwrap_or_default();
                self.detail = Some(format!("{} · {}", detail, fallback));
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::operation_check;
    use crate::types::DiagnosticStatus;

    #[test]
    fn operation_check_is_ok_when_idle() {
        let check = operation_check(0, false, false);
        assert_eq!(check.status, DiagnosticStatus::Ok);
    }

    #[test]
    fn operation_check_warns_when_busy() {
        let check = operation_check(2, true, false);
        assert_eq!(check.status, DiagnosticStatus::Warning);
        assert!(check.detail.unwrap().contains("下载任务 2 个"));
    }
}
