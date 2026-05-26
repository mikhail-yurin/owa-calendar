use config::{Config, ConfigError, File};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize, Clone)]
pub struct AppConfig {
    pub calendar: CalendarConfig,
}

#[derive(Deserialize, Clone)]
pub struct CalendarConfig {
    pub host: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_fetch_interval")]
    pub fetch: u64,
    #[serde(default = "default_notify_minutes")]
    pub notify: i64,
    #[serde(default = "default_build_version")]
    pub build_version: String,
    #[serde(default = "default_action_calendar_view")]
    pub action_calendar_view: i32,
    #[serde(default = "default_action_calendar_folders")]
    pub action_calendar_folders: i32,
    #[serde(default = "default_action_get_folder")]
    pub action_get_folder: i32,
}

fn default_fetch_interval() -> u64 {
    10 // minutes
}

fn default_notify_minutes() -> i64 {
    15 // minutes
}

fn default_build_version() -> String {
    "15.2.1748.10".to_string()
}

fn default_action_calendar_view() -> i32 {
    -27
}

fn default_action_calendar_folders() -> i32 {
    -8
}

fn default_action_get_folder() -> i32 {
    -57
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = Self::get_config_path();

        // Create a dir for a config if it does not exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).ok();
        }

        // Create a config with default data if it does not exist
        if !config_path.exists() {
            Self::create_default_config(&config_path)?;
        }

        // Load config from file (fallback to env variables)
        let settings = Config::builder()
            .add_source(File::from(config_path).required(false))
            .add_source(config::Environment::with_prefix("OWA").separator("_"))
            .build()?;

        settings.try_deserialize()
    }

    pub fn get_config_path() -> PathBuf {
        if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("owa-calendar").join("config.toml")
        } else {
            PathBuf::from(".").join("config.toml")
        }
    }

    fn create_default_config(path: &PathBuf) -> Result<(), ConfigError> {
        let default_config = r#"[calendar]
# Интервал обновления календаря (в минутах)
fetch = 10

# За сколько минут до события показывать уведомление
notify = 15

# хост OWA сервиса. Пример: "https://owa.example.com/"
host = ""

# логин с доменом от учетной записи. Пример "DOMAIN\\username"
username = "DOMAIN\\username"

# пароль от учетной записи
password = ""

# Версия Exchange сервера (X-OWA-ClientBuildVersion)
build_version = "15.2.1748.10"

# OWA action ID для GetCalendarView
action_calendar_view = -27

# OWA action ID для GetCalendarFolders
action_calendar_folders = -8

# OWA action ID для GetFolder
action_get_folder = -57
"#;

        fs::write(path, default_config).map_err(|e| ConfigError::Message(e.to_string()))?;

        println!("✓ Created default config at: {}", path.display());

        // Open config in a default application
        Self::open_file_in_default_app(path);

        Ok(())
    }

    pub fn open_url_in_default_browser(url: &str) {
        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("xdg-open").arg(url).spawn();
        }

        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("open").arg(url).spawn();
        }

        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("cmd").args(&["/C", "start", "", url]).spawn();
        }
    }

    pub fn open_file_in_default_app(path: &PathBuf) {
        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("xdg-open").arg(path).spawn();
        }

        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("open").arg(path).spawn();
        }

        #[cfg(target_os = "windows")]
        {
            let _ = Command::new("cmd")
                .args(&["/C", "start", "", path.to_str().unwrap_or("")])
                .spawn();
        }
    }
}
