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
    util::{read_json, write_json},
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
    pub next_id: AtomicU64,
}

impl AppState {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let app_dir = home.join(APP_DIR_NAME);
        let _ = fs::create_dir_all(&app_dir);

        let config = read_json(&app_dir.join(CONFIG_FILE)).unwrap_or_else(default_config);
        let history = read_json(&app_dir.join(HISTORY_FILE)).unwrap_or_default();
        let seed = Utc::now().timestamp_millis().max(0) as u64;

        Self {
            app_dir,
            config: Arc::new(Mutex::new(config)),
            history: Arc::new(Mutex::new(history)),
            running: Arc::new(Mutex::new(HashMap::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            login: Arc::new(Mutex::new(None)),
            login_cancelled: Arc::new(Mutex::new(false)),
            next_id: AtomicU64::new(seed),
        }
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
    let downloads = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));

    AppConfig {
        last_directory: downloads.to_string_lossy().to_string(),
        limit: 4,
        threads: 4,
        pool: 8,
        tdl_override_path: None,
    }
}
