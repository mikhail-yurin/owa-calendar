use config::{Config, ConfigError, File};
use keyring::Entry;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const KEYRING_SERVICE: &str = "owa-calendar";

#[derive(Deserialize, Clone)]
pub struct AppConfig {
    pub calendar: CalendarConfig,
}

#[derive(Deserialize, Clone)]
pub struct CalendarConfig {
    pub host: String,
    pub username: String,
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

/// Rewrites the `username = "..."` assignment in a TOML config string,
/// preserving every other line. Returns `None` when there is no such line.
fn replace_username_line(content: &str, username: &str) -> Option<String> {
    let escaped = username.replace('\\', "\\\\").replace('"', "\\\"");
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        let is_username_assignment = !replaced
            && trimmed
                .strip_prefix("username")
                .map(|rest| rest.trim_start().starts_with('='))
                .unwrap_or(false);

        if is_username_assignment {
            let indent = &line[..line.len() - trimmed.len()];
            out.push(format!("{}username = \"{}\"", indent, escaped));
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }

    if !replaced {
        return None;
    }

    let mut result = out.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
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

    pub fn get_password(username: &str) -> Result<String, keyring::Error> {
        Entry::new(KEYRING_SERVICE, username)?.get_password()
    }

    pub fn set_credentials(username: &str, password: &str) -> Result<(), keyring::Error> {
        Entry::new(KEYRING_SERVICE, username)?.set_password(password)?;
        // Keep config.toml's `username` in sync so the next launch looks the
        // password up under the same keyring key. Best-effort: a failed
        // config write shouldn't fail an otherwise-successful login.
        if let Err(e) = Self::write_username_to_config(username) {
            eprintln!("Failed to persist username to config: {}", e);
        }
        Ok(())
    }

    fn write_username_to_config(username: &str) -> std::io::Result<()> {
        let path = Self::get_config_path();
        let content = fs::read_to_string(&path)?;
        match replace_username_line(&content, username) {
            Some(updated) if updated != content => fs::write(&path, updated),
            _ => Ok(()),
        }
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

#[cfg(test)]
mod tests {
    use super::replace_username_line;

    #[test]
    fn replaces_the_username_line_and_keeps_the_rest() {
        let src = "[calendar]\n# comment\nhost = \"h\"\nusername = \"DOMAIN\\\\old\"\nfetch = 10\n";
        let out = replace_username_line(src, "ACME\\new").unwrap();
        assert_eq!(
            out,
            "[calendar]\n# comment\nhost = \"h\"\nusername = \"ACME\\\\new\"\nfetch = 10\n"
        );
    }

    #[test]
    fn escapes_backslashes_and_quotes() {
        let out = replace_username_line("username = \"\"\n", "d\\u\"x").unwrap();
        assert_eq!(out, "username = \"d\\\\u\\\"x\"\n");
    }

    #[test]
    fn preserves_trailing_newline_state() {
        assert!(replace_username_line("username = \"a\"", "b")
            .unwrap()
            .ends_with("\"b\""));
        assert!(replace_username_line("username = \"a\"\n", "b")
            .unwrap()
            .ends_with("\"b\"\n"));
    }

    #[test]
    fn returns_none_when_no_username_line() {
        assert!(replace_username_line("[calendar]\nhost = \"h\"\n", "b").is_none());
    }

    #[test]
    fn ignores_commented_and_lookalike_keys() {
        let src = "# username = \"x\"\nusername_extra = \"y\"\n";
        assert!(replace_username_line(src, "b").is_none());
    }

    #[test]
    fn matches_without_spaces_around_equals() {
        let out = replace_username_line("username=\"a\"\n", "b").unwrap();
        assert_eq!(out, "username = \"b\"\n");
    }
}
