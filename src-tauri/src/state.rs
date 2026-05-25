use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    process::Child,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use chrono::Utc;

use crate::{
    types::{AppConfig, DownloadRecord},
    util::{default_download_dir, read_json, write_json},
};

const APP_DIR_NAME: &str = ".tdl-desktop";
const CONFIG_FILE: &str = "config.json";
const HISTORY_FILE: &str = "history.json";

pub struct AppState {
    pub app_dir: PathBuf,
    pub config: Arc<Mutex<AppConfig>>,
    pub history: Arc<Mutex<Vec<DownloadRecord>>>,
    pub running: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    pub cancelled: Arc<Mutex<HashSet<String>>>,
    pub login: Arc<Mutex<Option<Arc<Mutex<Child>>>>>,
    pub login_cancelled: Arc<Mutex<bool>>,
    pub tdl_update_running: Arc<Mutex<bool>>,
    pub cache_generation: Arc<AtomicU64>,
    pub next_id: AtomicU64,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let home = dirs::home_dir()
            .ok_or_else(|| "无法定位用户目录，请确认 USERPROFILE 环境变量可用。".to_string())?;
        let app_dir = home.join(APP_DIR_NAME);
        fs::create_dir_all(&app_dir)
            .map_err(|error| format!("无法创建应用数据目录 ({}): {error}", app_dir.display()))?;

        let mut config = read_json(&app_dir.join(CONFIG_FILE)).unwrap_or_else(default_config);
        fill_config_defaults(&mut config, &app_dir);
        let history = read_json(&app_dir.join(HISTORY_FILE)).unwrap_or_default();
        let seed = Utc::now().timestamp_millis().max(0) as u64;

        Ok(Self {
            app_dir,
            config: Arc::new(Mutex::new(config)),
            history: Arc::new(Mutex::new(history)),
            running: Arc::new(Mutex::new(HashMap::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            login: Arc::new(Mutex::new(None)),
            login_cancelled: Arc::new(Mutex::new(false)),
            tdl_update_running: Arc::new(Mutex::new(false)),
            cache_generation: Arc::new(AtomicU64::new(0)),
            next_id: AtomicU64::new(seed),
        })
    }

    pub fn config_path(&self) -> PathBuf {
        self.app_dir.join(CONFIG_FILE)
    }

    pub fn history_path(&self) -> PathBuf {
        self.app_dir.join(HISTORY_FILE)
    }

    pub fn local_tdl_path(&self) -> PathBuf {
        self.app_dir.join("bin").join("tdl.exe")
    }

    pub fn default_log_dir(&self) -> PathBuf {
        self.app_dir.join("logs")
    }

    pub fn next_id(&self, prefix: &str) -> String {
        let value = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{value}")
    }

    pub fn persist_config(&self, config: &AppConfig) -> Result<(), String> {
        write_json(&self.config_path(), config)
    }

    pub fn persist_history(&self, history: &[DownloadRecord]) -> Result<(), String> {
        write_json(&self.history_path(), history)
    }

    pub fn refs(&self) -> StateRefs {
        StateRefs {
            history: Arc::clone(&self.history),
            running: Arc::clone(&self.running),
            cancelled: Arc::clone(&self.cancelled),
            history_path: self.history_path(),
        }
    }
}

#[derive(Clone)]
pub struct StateRefs {
    pub history: Arc<Mutex<Vec<DownloadRecord>>>,
    pub running: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    pub cancelled: Arc<Mutex<HashSet<String>>>,
    pub history_path: PathBuf,
}

fn default_config() -> AppConfig {
    let downloads = default_download_dir().unwrap_or_default();

    AppConfig {
        last_directory: downloads.to_string_lossy().to_string(),
        limit: 4,
        threads: 4,
        pool: 8,
        tdl_override_path: None,
        language: "zh".into(),
        log_directory: String::new(),
        desktop_update_url: String::new(),
        tdl_namespace: "default".into(),
        tdl_storage: String::new(),
    }
}

fn fill_config_defaults(config: &mut AppConfig, app_dir: &std::path::Path) {
    if config.language.trim().is_empty() {
        config.language = "zh".into();
    }
    if config.log_directory.trim().is_empty() {
        config.log_directory = app_dir.join("logs").to_string_lossy().to_string();
    }
    if config.tdl_namespace.trim().is_empty() {
        config.tdl_namespace = "default".into();
    }
}
