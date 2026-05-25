use std::path::{Path, PathBuf};

use crate::types::AppConfig;

pub const DEFAULT_TDL_NAMESPACE: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTargets {
    pub namespace_dir: PathBuf,
    pub legacy_files: Vec<PathBuf>,
}

pub fn normalize_namespace(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        DEFAULT_TDL_NAMESPACE.to_string()
    } else {
        value.to_string()
    }
}

pub fn tdl_global_args(config: &AppConfig) -> Vec<String> {
    let mut args = vec![
        "--ns".to_string(),
        normalize_namespace(&config.tdl_namespace),
    ];
    if !config.tdl_storage.trim().is_empty() {
        args.extend([
            "--storage".to_string(),
            config.tdl_storage.trim().to_string(),
        ]);
    }
    args
}

pub fn prepend_tdl_global_args(config: &AppConfig, args: Vec<String>) -> Vec<String> {
    let mut output = tdl_global_args(config);
    output.extend(args);
    output
}

pub fn add_missing_raw_global_args(config: &AppConfig, mut args: Vec<String>) -> Vec<String> {
    let has_namespace = has_flag_value(&args, "--ns") || has_flag_value(&args, "-n");
    let has_storage = has_flag_value(&args, "--storage");

    let mut prefix = Vec::new();
    if !has_namespace {
        prefix.extend([
            "--ns".to_string(),
            normalize_namespace(&config.tdl_namespace),
        ]);
    }
    if !has_storage && !config.tdl_storage.trim().is_empty() {
        prefix.extend([
            "--storage".to_string(),
            config.tdl_storage.trim().to_string(),
        ]);
    }
    prefix.append(&mut args);
    prefix
}

pub fn session_targets(config: &AppConfig) -> Result<SessionTargets, String> {
    session_targets_with_home(config, default_home_dir()?)
}

fn default_home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法定位用户目录，请手动检查 tdl 登录数据。".to_string())
}

fn session_targets_with_home(config: &AppConfig, home: PathBuf) -> Result<SessionTargets, String> {
    let namespace = normalize_namespace(&config.tdl_namespace);
    let storage = parse_storage(config.tdl_storage.trim());
    let storage_path = storage
        .path
        .unwrap_or_else(|| home.join(".tdl").join("data"));
    let namespace_dir = storage_path.join(&namespace);
    ensure_safe_namespace_target(&namespace_dir, &storage_path)?;

    let mut legacy_files = Vec::new();
    if config.tdl_storage.trim().is_empty() && namespace == DEFAULT_TDL_NAMESPACE {
        legacy_files.push(home.join(".tdl").join("data.kv"));
    }

    Ok(SessionTargets {
        namespace_dir,
        legacy_files,
    })
}

fn parse_storage(value: &str) -> ParsedStorage {
    let mut storage = ParsedStorage { path: None };
    for part in value.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("path") && !value.trim().is_empty() {
            storage.path = Some(PathBuf::from(value.trim()));
        }
    }
    storage
}

#[derive(Debug)]
struct ParsedStorage {
    path: Option<PathBuf>,
}

fn ensure_safe_namespace_target(target: &Path, storage_root: &Path) -> Result<(), String> {
    if target == storage_root || is_path_root(target) {
        return Err(format!(
            "tdl 登录数据路径不安全，拒绝清理: {}",
            target.display()
        ));
    }
    if target.file_name().is_none() {
        return Err(format!("tdl 登录数据路径不完整: {}", target.display()));
    }
    Ok(())
}

fn is_path_root(path: &Path) -> bool {
    path.parent().is_none() || path.parent().is_some_and(|parent| parent == path)
}

fn has_flag_value(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(namespace: &str, storage: &str) -> AppConfig {
        AppConfig {
            last_directory: String::new(),
            limit: 4,
            threads: 4,
            pool: 8,
            tdl_override_path: None,
            language: "zh".into(),
            log_directory: String::new(),
            desktop_update_url: String::new(),
            tdl_namespace: namespace.into(),
            tdl_storage: storage.into(),
        }
    }

    #[test]
    fn defaults_namespace() {
        assert_eq!(normalize_namespace("  "), DEFAULT_TDL_NAMESPACE);
    }

    #[test]
    fn builds_global_args() {
        assert_eq!(
            tdl_global_args(&config("work", "type=bolt,path=D:/tdl-data")),
            vec!["--ns", "work", "--storage", "type=bolt,path=D:/tdl-data"]
        );
    }

    #[test]
    fn raw_args_do_not_duplicate_existing_flags() {
        let args = add_missing_raw_global_args(
            &config("work", "type=bolt,path=D:/tdl-data"),
            vec![
                "--ns".into(),
                "other".into(),
                "--storage=type=bolt,path=E:/tdl".into(),
                "download".into(),
            ],
        );
        assert_eq!(args[0], "--ns");
        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_str() == "--storage=type=bolt,path=E:/tdl")
                .count(),
            1
        );
        assert!(!args
            .windows(2)
            .any(|items| items == ["--storage", "type=bolt,path=D:/tdl-data"]));
    }

    #[test]
    fn resolves_default_session_targets() {
        let targets =
            session_targets_with_home(&config("", ""), PathBuf::from("C:/Users/me")).unwrap();
        assert_eq!(
            targets.namespace_dir,
            PathBuf::from("C:/Users/me/.tdl/data/default")
        );
        assert_eq!(
            targets.legacy_files,
            vec![PathBuf::from("C:/Users/me/.tdl/data.kv")]
        );
    }

    #[test]
    fn resolves_custom_storage_target() {
        let targets = session_targets_with_home(
            &config("work", "type=bolt,path=D:/tdl-data"),
            PathBuf::from("C:/Users/me"),
        )
        .unwrap();
        assert_eq!(targets.namespace_dir, PathBuf::from("D:/tdl-data/work"));
        assert!(targets.legacy_files.is_empty());
    }
}
