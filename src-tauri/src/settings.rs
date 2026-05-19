//! Персистентные настройки Clod Helper в `%APPDATA%\Clod Helper\settings.json`.
//!
//! Зачем: после установки шим запоминает прокси в .vbs (в Startup-папке), но
//! сам Dashboard должен знать его чтобы показывать в UI и предлагать смену.
//! Также хранится флаг autostart-окна (отдельно от autostart-шима).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "settings.json";
const APP_DIR: &str = "Clod Helper";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// URL купленного upstream-прокси `http://login:pass@ip:port`.
    /// Хранится в открытом виде — это локальный файл пользователя.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Запускать Dashboard в трее при логине Windows.
    #[serde(default)]
    pub autostart_dashboard: bool,
}

fn settings_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir()
        .ok_or_else(|| "Не нашёл папку %APPDATA%".to_string())?;
    Ok(base.join(APP_DIR).join(SETTINGS_FILE))
}

pub fn load() -> Settings {
    let path = match settings_path() {
        Ok(p) => p,
        Err(_) => return Settings::default(),
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Не смог создать {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, text)
        .map_err(|e| format!("Не смог записать {}: {e}", path.display()))?;
    Ok(())
}

pub fn set_proxy_url(url: String) -> Result<Settings, String> {
    let mut s = load();
    s.proxy_url = Some(url);
    save(&s)?;
    Ok(s)
}
