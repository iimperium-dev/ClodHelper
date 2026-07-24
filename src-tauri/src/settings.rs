//! Персистентные настройки Clod Helper в `%APPDATA%\Clod Helper\settings.json`.
//!
//! Зачем: после установки шим запоминает прокси в .vbs (в Startup-папке), но
//! сам Dashboard должен знать его чтобы показывать в UI и предлагать смену.
//! Также хранится флаг autostart-окна (отдельно от autostart-шима).

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::apps::AppProfile;

/// Все мутации настроек — под одной блокировкой: tauri-команды конкурентны,
/// а load-modify-save без неё теряет чужие изменения (второй писатель молча
/// откатывает, например, только что сохранённый пул прокси).
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

fn with_settings<F>(f: F) -> Result<Settings, String>
where
    F: FnOnce(&mut Settings) -> Result<(), String>,
{
    let _g = SETTINGS_LOCK
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    let mut s = load();
    f(&mut s)?;
    save(&s)?;
    Ok(s)
}

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
    /// Режим маршрутизации моста: "all" (всё через upstream, как раньше) |
    /// "smart" (через upstream только AI-домены, остальное напрямую).
    /// None = "all" — консервативный дефолт для старых установок.
    #[serde(default)]
    pub route_mode: Option<String>,
    /// Прокси-пул (до 5). [0] — основной; proxy_url держится с ним в синхроне
    /// для обратной совместимости (vbs/старый код читают proxy_url).
    /// Пустой список = легаси-установка, пул из одного proxy_url.
    #[serde(default)]
    pub proxies: Vec<String>,
    /// Назначения «сервис → индекс прокси в пуле»: ключи — группы из
    /// tasks::SERVICE_GROUPS ("anthropic", "openai", …). Отсутствие ключа =
    /// авто (основной + failover).
    #[serde(default)]
    pub proxy_assignments: std::collections::HashMap<String, usize>,
    /// Пользовательские имена (тэги) прокси: URL → имя («Немецкий», «Резерв»).
    #[serde(default)]
    pub proxy_labels: std::collections::HashMap<String, String>,
    /// Чужие прокси-переменные (корпоративный прокси, свой VPN-тул), которые
    /// мы перезаписали при включении системного проксирования. «Выключить»
    /// возвращает их, а не стирает настройку пользователя насовсем.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_proxy_env: Option<std::collections::HashMap<String, String>>,
}

/// Полный пул прокси: proxies, а для легаси-установок — единственный proxy_url.
pub fn effective_proxies(s: &Settings) -> Vec<String> {
    if !s.proxies.is_empty() {
        s.proxies.clone()
    } else {
        s.proxy_url.clone().into_iter().collect()
    }
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
    // Атомарно (tmp + rename): параллельный читатель не должен увидеть
    // обрезанный JSON и молча откатиться к Settings::default().
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)
        .map_err(|e| format!("Не смог записать {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("Не смог заменить {}: {e}", path.display()))?;
    Ok(())
}

/// Перемапить индексы назначений на новый порядок пула ПО URL: назначение
/// обязано продолжать указывать на ТОТ ЖЕ прокси (тот же выходной IP), как бы
/// пул ни переставляли/укорачивали. Съехавший индекс = закреплённый Claude
/// молча уезжает на другой IP. Назначения на исчезнувший URL снимаются
/// (сервис возвращается в «Авто»).
fn remap_assignments(s: &mut Settings, old_pool: &[String], new_pool: &[String]) {
    s.proxy_assignments = std::mem::take(&mut s.proxy_assignments)
        .into_iter()
        .filter_map(|(svc, idx)| {
            let url = old_pool.get(idx)?;
            let new_idx = new_pool.iter().position(|u| u == url)?;
            Some((svc, new_idx))
        })
        .collect();
}

pub fn set_proxy_url(url: String) -> Result<Settings, String> {
    with_settings(|s| {
        // Основной прокси и пул держим в синхроне: [0] всегда = proxy_url.
        let old_pool = effective_proxies(s);
        let mut new_pool = old_pool.clone();
        if let Some(pos) = new_pool.iter().position(|u| *u == url) {
            // URL уже в пуле — «сделать основным» перестановкой, не дублируя.
            new_pool.remove(pos);
            new_pool.insert(0, url.clone());
        } else if new_pool.is_empty() {
            new_pool.push(url.clone());
        } else {
            new_pool[0] = url.clone();
        }
        remap_assignments(s, &old_pool, &new_pool);
        s.proxy_labels.retain(|u, _| new_pool.contains(u));
        s.proxy_url = Some(url);
        s.proxies = new_pool;
        Ok(())
    })
}

/// Заменить пул целиком (1..=5, [0] — основной). Индексы назначений
/// перемапливаются по URL; назначения на удалённые прокси снимаются.
pub fn set_proxies(urls: Vec<String>) -> Result<Settings, String> {
    if urls.is_empty() || urls.len() > 5 {
        return Err("Пул прокси: от 1 до 5 адресов".into());
    }
    with_settings(|s| {
        let old_pool = effective_proxies(s);
        remap_assignments(s, &old_pool, &urls);
        s.proxy_labels.retain(|url, _| urls.contains(url));
        s.proxy_url = Some(urls[0].clone());
        s.proxies = urls;
        Ok(())
    })
}

/// Задать/убрать имя (тэг) прокси. Пустое имя = убрать.
pub fn set_proxy_label(url: String, label: String) -> Result<Settings, String> {
    with_settings(|s| {
        let label = label.trim().to_string();
        if label.is_empty() {
            s.proxy_labels.remove(&url);
        } else {
            s.proxy_labels.insert(url, label);
        }
        Ok(())
    })
}

pub fn set_proxy_assignments(
    map: std::collections::HashMap<String, usize>,
) -> Result<Settings, String> {
    with_settings(|s| {
        let pool_len = effective_proxies(s).len();
        s.proxy_assignments = map.into_iter().filter(|(_, idx)| *idx < pool_len).collect();
        Ok(())
    })
}

/// Загрузить настройки, гарантируя что встроенные пресеты присутствуют.
///
/// При первом запуске (или после апдейта, добавившего новый пресет) сеет
/// недостающие builtin-профили, сохраняя пользовательский `enabled` у уже
/// существующих. Кастомные профили не трогает.
pub fn load_with_presets() -> Settings {
    let _g = SETTINGS_LOCK.lock().ok();
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
/// UI шлёт runtime-снимок, где exe_path/app_id пустеют, если приложение в
/// момент опроса недоступно (отключённый диск, «не найдено») — надёжные
/// сохранённые автодетект-поля пустотой не затираем.
pub fn set_profiles(profiles: Vec<AppProfile>) -> Result<Settings, String> {
    with_settings(|s| {
        let old = std::mem::take(&mut s.app_profiles);
        s.app_profiles = profiles
            .into_iter()
            .map(|mut p| {
                if let Some(prev) = old.iter().find(|o| o.id == p.id) {
                    if p.exe_path.is_none() {
                        p.exe_path = prev.exe_path.clone();
                    }
                    if p.app_id.is_none() {
                        p.app_id = prev.app_id.clone();
                    }
                }
                p
            })
            .collect();
        Ok(())
    })
}

/// Снимок чужих env-переменных до перезаписи (None = снять снимок).
pub fn set_saved_proxy_env(
    value: Option<std::collections::HashMap<String, String>>,
) -> Result<Settings, String> {
    with_settings(|s| {
        s.saved_proxy_env = value;
        Ok(())
    })
}

pub fn set_killswitch(enabled: bool) -> Result<Settings, String> {
    with_settings(|s| {
        s.killswitch_enabled = enabled;
        Ok(())
    })
}

pub fn set_route_mode(mode: String) -> Result<Settings, String> {
    with_settings(|s| {
        s.route_mode = Some(mode);
        Ok(())
    })
}
