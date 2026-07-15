//! Персистентные настройки Clod Helper в `%APPDATA%\Clod Helper\settings.json`.
//!
//! Зачем: после установки шим запоминает прокси в .vbs (в Startup-папке), но
//! сам Dashboard должен знать его чтобы показывать в UI и предлагать смену.
//! Также хранится флаг autostart-окна (отдельно от autostart-шима).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::apps::AppProfile;

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
    /// Профили приложений (какие проксируем). Сеются пресетами при первом
    /// запуске; пользователь включает/выключает и добавляет свои.
    #[serde(default)]
    pub app_profiles: Vec<AppProfile>,
    /// Включён ли killswitch (firewall fail-closed). Отражает НАМЕРЕНИЕ
    /// пользователя; фактические правила проверяются отдельно в firewall.rs.
    #[serde(default)]
    pub killswitch_enabled: bool,
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

/// Загрузить настройки, гарантируя что встроенные пресеты присутствуют.
///
/// При первом запуске (или после апдейта, добавившего новый пресет) сеет
/// недостающие builtin-профили, сохраняя пользовательский `enabled` у уже
/// существующих. Кастомные профили не трогает.
pub fn load_with_presets() -> Settings {
    let mut s = load();
    let presets = crate::apps::builtin_presets();
    let mut changed = false;

    for preset in presets {
        if let Some(existing) = s.app_profiles.iter_mut().find(|p| p.id == preset.id) {
            // Профиль уже есть — обновим только автодетект-поля (путь/AppID могли
            // появиться если приложение поставили позже), сохранив enabled.
            if existing.exe_path.is_none() && preset.exe_path.is_some() {
                existing.exe_path = preset.exe_path.clone();
                changed = true;
            }
            if existing.app_id.is_none() && preset.app_id.is_some() {
                existing.app_id = preset.app_id.clone();
                changed = true;
            }
        } else {
            s.app_profiles.push(preset);
            changed = true;
        }
    }

    if changed {
        let _ = save(&s);
    }
    s
}

/// Заменить список профилей целиком (UI прислал новое состояние enabled/добавил своё).
pub fn set_profiles(profiles: Vec<AppProfile>) -> Result<Settings, String> {
    let mut s = load();
    s.app_profiles = profiles;
    save(&s)?;
    Ok(s)
}

pub fn set_killswitch(enabled: bool) -> Result<Settings, String> {
    let mut s = load();
    s.killswitch_enabled = enabled;
    save(&s)?;
    Ok(s)
}
