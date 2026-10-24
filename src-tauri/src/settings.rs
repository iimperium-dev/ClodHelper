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
    /// Окно «Добавить прокси» в Telegram уже показывали (мастер, кнопка или
    /// автоподсказка) — сами больше не открываем, решает пользователь.
    #[serde(default)]
    pub telegram_prompted: bool,
    /// Раскладка «Приложений»: вид и папки с порядком (UI хранит как есть,
    /// бэкенд не интерпретирует). None — умная раскладка по умолчанию.
    #[serde(default)]
    pub apps_layout: Option<serde_json::Value>,
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
    /// Не выпускать телеметрию Claude (отчёты об ошибках в Sentry, Datadog):
    /// в них имя компьютера и железо. None — включено (по умолчанию).
    #[serde(default)]
    pub block_claude_telemetry: Option<bool>,
    /// Резервы прокси: URL прокси пула → его резервы по порядку (до 5).
    /// Ключ — URL, а не индекс: при перестановке пула резервы едут вместе
    /// со своим прокси.
    #[serde(default)]
    pub proxy_backups: std::collections::HashMap<String, Vec<String>>,
    /// Версия, о которой уже показали уведомление «Доступно обновление»:
    /// фоновая проверка не повторяет его каждые несколько часов.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_notified: Option<String>,
    /// Последняя запущенная версия приложения: запуск более новой = окно
    /// «Что нового». None — сборки до этого поля (или первый запуск).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_version_seen: Option<String>,
    /// Рассылки разработчика (последний скачанный notices.json) — колокольчик
    /// виден и без сети.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices_cache: Vec<crate::notices::Notice>,
    /// Прочитанные уведомления (id рассылок и «update-X.Y.Z»).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices_read: Vec<String>,
    /// Рассылки, уже всплывавшие в углу экрана (повторно не всплывают).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices_toasted: Vec<String>,
    /// «Сайты» только в выбранных браузерах (PAC их политикой), а не во всех
    /// (системная автонастройка). Какие выбраны — видно по их политикам.
    #[serde(default)]
    pub sites_scope_selected: bool,
    /// Программы, которые «Сайты» не трогают (антидетект-браузеры и т.п.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites_bypass: Vec<crate::browser_ext::BypassApp>,
    /// «Сайты во всех браузерах»: системная автонастройка прокси (PAC моста).
    #[serde(default)]
    pub system_pac: bool,
    /// Системные настройки прокси до включения — вернуть при выключении.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_pac_saved: Option<crate::system_pac::ProxyConf>,
    /// Чужие прокси-переменные (корпоративный прокси, свой VPN-тул), которые
    /// мы перезаписали при включении системного проксирования. «Выключить»
    /// возвращает их, а не стирает настройку пользователя насовсем.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_proxy_env: Option<std::collections::HashMap<String, String>>,
    /// Сайты, которые ходят через прокси (раздел «Сайты»): мост читает их из
    /// bridge-sites.json на лету, без перезапуска.
    #[serde(default)]
    pub sites: Vec<SiteEntry>,
}

/// Сайт пользователя: имя, его домены (с поддоменами) и прокси.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SiteEntry {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    /// Индекс прокси в пуле; None — «Авто» (основной + failover).
    #[serde(default)]
    pub proxy: Option<usize>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Полный пул прокси: proxies, а для легаси-установок — единственный proxy_url.
pub fn effective_proxies(s: &Settings) -> Vec<String> {
    if !s.proxies.is_empty() {
        s.proxies.clone()
    } else {
        s.proxy_url.clone().into_iter().collect()
    }
}

/// Сколько резервов можно привязать к одному прокси.
pub const MAX_BACKUPS: usize = 5;

/// Прокси для моста: пул (индексы 0..n — как в назначениях и закреплённых
/// входах), затем резервы всех прокси по порядку пула. Резервы — в конце,
/// чтобы индексы пула не сдвигались. Один и тот же URL — один раз.
pub fn bridge_upstreams(s: &Settings) -> Vec<String> {
    let mut list = effective_proxies(s);
    let pool = list.clone();
    for parent in &pool {
        for b in s.proxy_backups.get(parent).into_iter().flatten() {
            if !list.contains(b) {
                list.push(b.clone());
            }
        }
    }
    list
}

/// Резервы для rules-файла моста: индекс прокси пула → индексы его
/// резервов в списке bridge_upstreams.
pub fn bridge_fallbacks(s: &Settings) -> std::collections::BTreeMap<String, Vec<usize>> {
    let all = bridge_upstreams(s);
    effective_proxies(s)
        .iter()
        .enumerate()
        .filter_map(|(i, parent)| {
            let idx: Vec<usize> = s
                .proxy_backups
                .get(parent)?
                .iter()
                .filter_map(|b| all.iter().position(|u| u == b))
                .filter(|&j| j != i)
                .collect();
            (!idx.is_empty()).then(|| (i.to_string(), idx))
        })
        .collect()
}

/// Задать резервы прокси пула (пустой список — убрать все).
pub fn set_proxy_backups(parent: String, backups: Vec<String>) -> Result<Settings, String> {
    if backups.len() > MAX_BACKUPS {
        return Err(format!("У прокси может быть до {MAX_BACKUPS} резервов"));
    }
    with_settings(|s| {
        let pool = effective_proxies(s);
        if !pool.contains(&parent) {
            return Err("Этого прокси уже нет в списке".into());
        }
        let mut clean: Vec<String> = Vec::new();
        for b in backups {
            if b == parent || pool.contains(&b) {
                return Err("Этот прокси уже есть в списке — резервом его сделать нельзя".into());
            }
            if !clean.contains(&b) {
                clean.push(b);
            }
        }
        if clean.is_empty() {
            s.proxy_backups.remove(&parent);
        } else {
            s.proxy_backups.insert(parent, clean);
        }
        retain_labels(s);
        Ok(())
    })
}

/// Имена — только у прокси пула и их резервов.
fn retain_labels(s: &mut Settings) {
    let keep = bridge_upstreams(s);
    s.proxy_labels.retain(|u, _| keep.contains(u));
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
    // Прокси сайтов — так же по URL; исчезнувший → «Авто».
    for site in &mut s.sites {
        site.proxy = site
            .proxy
            .and_then(|idx| old_pool.get(idx))
            .and_then(|url| new_pool.iter().position(|u| u == url));
    }
}

pub fn set_sites(sites: Vec<SiteEntry>) -> Result<Settings, String> {
    with_settings(|s| {
        let pool_len = effective_proxies(s).len();
        s.sites = sites
            .into_iter()
            .map(|mut site| {
                site.proxy = site.proxy.filter(|i| *i < pool_len);
                site
            })
            .collect();
        Ok(())
    })
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
        s.proxy_backups.retain(|u, _| new_pool.contains(u));
        s.proxy_url = Some(url);
        s.proxies = new_pool;
        retain_labels(s);
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
        replace_pool(s, urls);
        Ok(())
    })
}

/// Новый пул в настройках: назначения и сайты — по URL, резервы едут со
/// своим прокси (ключ — URL); удалённый прокси уносит свои резервы, а
/// прокси, ставший обычным, резервом больше не числится.
fn replace_pool(s: &mut Settings, urls: Vec<String>) {
    let old_pool = effective_proxies(s);
    remap_assignments(s, &old_pool, &urls);
    s.proxy_backups.retain(|u, _| urls.contains(u));
    for list in s.proxy_backups.values_mut() {
        list.retain(|b| !urls.contains(b));
    }
    s.proxy_backups.retain(|_, list| !list.is_empty());
    s.proxy_url = urls.first().cloned();
    s.proxies = urls;
    retain_labels(s);
}

/// Задать/убрать имя (тэг) прокси. Пустое имя = убрать.
/// Блокировать ли телеметрию Claude (по умолчанию — да).
pub fn claude_telemetry_blocked(s: &Settings) -> bool {
    s.block_claude_telemetry.unwrap_or(true)
}

pub fn set_claude_telemetry_blocked(on: bool) -> Result<Settings, String> {
    with_settings(|s| {
        s.block_claude_telemetry = Some(on);
        Ok(())
    })
}

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
    // Поиск установленных приложений — до блокировки: остальные чтения
    // настроек (опрос статусов, окна) не должны ждать детекта.
    let presets = crate::apps::builtin_presets();
    let _g = SETTINGS_LOCK.lock().ok();
    let mut s = load();
    if merge_presets(&mut s.app_profiles, presets) {
        let _ = save(&s);
    }
    s
}

/// Merge detected presets without filesystem access so migrations can be checked
/// independently of the installed apps and the user's saved proxy settings.
fn merge_presets(profiles: &mut Vec<AppProfile>, presets: Vec<AppProfile>) -> bool {
    let mut changed = false;
    let before = profiles.len();
    profiles.retain(|p| !crate::apps::OBSOLETE_IDS.contains(&p.id.as_str()));
    changed |= profiles.len() != before;

    for preset in presets {
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == preset.id) {
            // Claude can move between a Store package and a legacy installer.
            // Refresh detection fields, preserving the user's enabled choice.
            if preset.id == crate::claude_desktop::PROFILE_ID {
                if existing.kind != preset.kind || existing.exe_path != preset.exe_path
                    || existing.app_id != preset.app_id
                {
                    existing.kind = preset.kind;
                    existing.exe_path = preset.exe_path;
                    existing.app_id = preset.app_id;
                    changed = true;
                }
                continue;
            }
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
            profiles.push(preset);
            changed = true;
        }
    }
    changed
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

pub fn set_sites_scope_selected(selected: bool) -> Result<Settings, String> {
    with_settings(|s| {
        s.sites_scope_selected = selected;
        Ok(())
    })
}

pub fn set_sites_bypass(apps: Vec<crate::browser_ext::BypassApp>) -> Result<Settings, String> {
    with_settings(|s| {
        s.sites_bypass = apps;
        Ok(())
    })
}

pub fn set_notices_cache(list: Vec<crate::notices::Notice>) -> Result<Settings, String> {
    with_settings(|s| {
        // Отметки о прочтении — только для живых id: файл рассылок может
        // похудеть, а список прочитанного не должен расти вечно.
        let alive: Vec<String> = list.iter().map(|n| n.id.clone()).collect();
        s.notices_read.retain(|id| id.starts_with("update-") || alive.contains(id));
        s.notices_toasted.retain(|id| alive.contains(id));
        s.notices_cache = list;
        Ok(())
    })
}

pub fn mark_notices_read(ids: &[String]) -> Result<Settings, String> {
    with_settings(|s| {
        for id in ids {
            if !s.notices_read.contains(id) {
                s.notices_read.push(id.clone());
            }
        }
        // Обновления: держим отметки только о последних версиях.
        let updates: Vec<usize> = s.notices_read.iter().enumerate().filter(|(_, i)| i.starts_with("update-")).map(|(n, _)| n).collect();
        if updates.len() > 5 {
            let drop: Vec<String> = updates[..updates.len() - 5].iter().map(|&n| s.notices_read[n].clone()).collect();
            s.notices_read.retain(|i| !drop.contains(i));
        }
        Ok(())
    })
}

pub fn mark_notices_toasted(ids: &[String]) -> Result<Settings, String> {
    with_settings(|s| {
        for id in ids {
            if !s.notices_toasted.contains(id) {
                s.notices_toasted.push(id.clone());
            }
        }
        Ok(())
    })
}

pub fn set_system_pac(enabled: bool) -> Result<Settings, String> {
    with_settings(|s| {
        s.system_pac = enabled;
        Ok(())
    })
}

pub fn set_system_pac_saved(conf: Option<crate::system_pac::ProxyConf>) -> Result<Settings, String> {
    with_settings(|s| {
        s.system_pac_saved = conf;
        Ok(())
    })
}

pub fn set_update_notified(version: &str) -> Result<Settings, String> {
    with_settings(|s| {
        s.update_notified = Some(version.to_string());
        Ok(())
    })
}

/// Запомнить текущую версию как увиденную. Возвращает версию, о которой
/// стоит показать «Что нового», — если приложение только что обновилось.
/// Без записанной версии (обновление со сборки до этого поля) обновлением
/// считается уже настроенная установка; первый запуск — нет.
pub fn take_whats_new(current: &str) -> Result<Option<String>, String> {
    let mut news = None;
    with_settings(|s| {
        let configured = s.proxy_url.is_some() || !s.proxies.is_empty();
        if just_updated(s.last_version_seen.as_deref(), configured, current) {
            news = Some(current.to_string());
        }
        s.last_version_seen = Some(current.to_string());
        Ok(())
    })?;
    Ok(news)
}

fn just_updated(seen: Option<&str>, configured: bool, current: &str) -> bool {
    match seen {
        Some(seen) => matches!(
            (crate::updater::parse_ver(seen), crate::updater::parse_ver(current)),
            (Some(a), Some(b)) if b > a
        ),
        None => configured,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::LaunchKind;

    #[test]
    fn backups_go_after_the_pool_and_keep_pool_indexes() {
        let mut s = Settings::default();
        s.proxies = vec!["a".into(), "b".into()];
        s.proxy_backups.insert("b".into(), vec!["b1".into(), "b2".into()]);
        s.proxy_backups.insert("a".into(), vec!["b1".into()]);
        // Пул на своих местах, резервы — после, общий резерв — один раз.
        assert_eq!(bridge_upstreams(&s), vec!["a", "b", "b1", "b2"]);
        let fb = bridge_fallbacks(&s);
        assert_eq!(fb.get("0"), Some(&vec![2]));
        assert_eq!(fb.get("1"), Some(&vec![2, 3]));
    }

    #[test]
    fn removed_proxy_takes_its_backups_and_promoted_backup_stops_being_one() {
        let mut s = Settings::default();
        s.proxies = vec!["a".into(), "b".into()];
        s.proxy_backups.insert("a".into(), vec!["x".into()]);
        s.proxy_backups.insert("b".into(), vec!["y".into(), "a2".into()]);
        s.proxy_labels.insert("y".into(), "Резерв B".into());
        // «b» убрали, его резерв «a2» сделали обычным прокси.
        replace_pool(&mut s, vec!["a".into(), "a2".into()]);
        assert_eq!(bridge_upstreams(&s), vec!["a", "a2", "x"]);
        assert!(!s.proxy_backups.contains_key("b"));
        assert!(!s.proxy_labels.contains_key("y"), "имя ушедшего резерва убрано");
    }

    #[test]
    fn whats_new_only_after_real_update() {
        assert!(just_updated(Some("0.7.1"), true, "0.8.0"));
        assert!(!just_updated(Some("0.8.0"), true, "0.8.0"), "повторный запуск");
        assert!(!just_updated(Some("0.9.0"), true, "0.8.0"), "откат версии");
        // Сборки до поля: обновлением считается только настроенная установка.
        assert!(just_updated(None, true, "0.8.0"));
        assert!(!just_updated(None, false, "0.8.0"), "первый запуск после установки");
    }

    fn claude_preset(kind: LaunchKind, exe_path: Option<&str>, app_id: Option<&str>) -> AppProfile {
        AppProfile {
            id: crate::claude_desktop::PROFILE_ID.into(),
            name: "Claude Desktop".into(),
            kind,
            exe_path: exe_path.map(str::to_owned),
            app_id: app_id.map(str::to_owned),
            process_names: vec!["Claude.exe".into()],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        }
    }

    #[test]
    fn adds_claude_preset_once_preserving_existing_profiles() {
        let custom = AppProfile {
            id: "custom-1".into(),
            name: "My application".into(),
            kind: LaunchKind::Custom,
            exe_path: Some(r"C:\Tools\My application.exe".into()),
            app_id: None,
            process_names: vec!["My application.exe".into()],
            enabled: true,
            builtin: false,
            hidden: false,
            proxy: None,
            full_proxy: None,
        };
        let codex = AppProfile {
            id: "codex".into(),
            name: "ChatGPT Codex".into(),
            kind: LaunchKind::Msix,
            exe_path: None,
            app_id: Some("OpenAI.Codex_2p2nqsd0c76g0!App".into()),
            process_names: vec!["ChatGPT.exe".into()],
            enabled: true,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        };
        let mut profiles = vec![custom, codex.clone()];
        let previous_profiles = serde_json::to_value(&profiles).unwrap();
        let claude = claude_preset(
            LaunchKind::Msix,
            Some(r"C:\WindowsApps\AnthropicPBC.Claude_2\app\Claude.exe"),
            Some("AnthropicPBC.Claude_fnn82j28hfe8t!Claude"),
        );
        let mut codex_preset = codex;
        codex_preset.enabled = false;
        let presets = vec![claude, codex_preset];

        assert!(merge_presets(&mut profiles, presets.clone()));
        assert_eq!(serde_json::to_value(&profiles[..2]).unwrap(), previous_profiles);
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[2].id, crate::claude_desktop::PROFILE_ID);
        assert!(profiles[2].builtin);
        assert!(!profiles[2].enabled);
        assert!(!merge_presets(&mut profiles, presets));
        assert_eq!(profiles.len(), 3);
    }

    #[test]
    fn site_proxy_follows_url_when_pool_changes() {
        let site = |proxy| SiteEntry { id: "s".into(), name: "s".into(), domains: vec!["x.com".into()], proxy, enabled: true };
        let mut s = Settings { sites: vec![site(Some(1)), site(Some(2)), site(None)], ..Default::default() };
        let old = vec!["a".to_string(), "b".into(), "c".into()];
        let new = vec!["b".to_string(), "a".into()];
        remap_assignments(&mut s, &old, &new);
        let got: Vec<Option<usize>> = s.sites.iter().map(|x| x.proxy).collect();
        assert_eq!(got, vec![Some(0), None, None], "тот же прокси по URL; удалённый — «Авто»");
    }

    #[test]
    fn refreshes_claude_installation_without_losing_enabled_choice() {
        let app_id = Some("AnthropicPBC.Claude_fnn82j28hfe8t!Claude");
        let mut installed = claude_preset(
            LaunchKind::Msix,
            Some(r"C:\WindowsApps\AnthropicPBC.Claude_1\app\Claude.exe"),
            app_id,
        );
        installed.enabled = true;
        let mut profiles = vec![installed];
        let changes = [
            claude_preset(
                LaunchKind::Msix,
                Some(r"C:\WindowsApps\AnthropicPBC.Claude_2\app\Claude.exe"),
                app_id,
            ),
            claude_preset(
                LaunchKind::ExeFlag,
                Some(r"C:\Users\Иван\AppData\Local\Claude\app-3.0\Claude.exe"),
                None,
            ),
            claude_preset(LaunchKind::Msix, None, None),
        ];

        for detected in changes {
            assert!(merge_presets(&mut profiles, vec![detected.clone()]));
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].kind, detected.kind);
            assert_eq!(profiles[0].exe_path, detected.exe_path);
            assert_eq!(profiles[0].app_id, detected.app_id);
            assert!(profiles[0].enabled);
            assert!(profiles[0].builtin);
            assert_eq!(profiles[0].process_names, ["Claude.exe"]);
            assert!(!merge_presets(&mut profiles, vec![detected]));
        }
    }
}
