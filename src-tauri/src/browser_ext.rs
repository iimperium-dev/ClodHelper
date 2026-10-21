//! Расширения браузеров, перехватывающие прокси (macOS и Windows).
//!
//! В Chromium-браузерах расширение с правом `proxy` (FoxyProxy,
//! SwitchyOmega, VPN-расширения) ставит СВОЮ настройку прокси, и она главнее
//! параметра запуска `--proxy-server`: браузер, открытый через мост, всё
//! равно идёт мимо. Снаружи такое не исправить — но можно честно показать
//! «прокси перехватывает FoxyProxy» и открыть страницу расширения.
//!
//! Признак: в Secure Preferences / Preferences профиля у включённого
//! расширения (пустые disable_reasons) есть `preferences.proxy`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::Value;

/// Включённое расширение, которое задало браузеру свой прокси.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProxyExtension {
    pub id: String,
    pub name: String,
}

/// Имя файла программы без пути и расширения («…/Google Chrome.app» →
/// «Google Chrome», «C:\…\chrome.exe» → «chrome») — разбор без учёта ОС.
fn app_stem(app_path: &str) -> Option<&str> {
    let leaf = app_path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
    let stem = [".app", ".exe", ".EXE"]
        .iter()
        .find_map(|ext| leaf.strip_suffix(ext))
        .unwrap_or(leaf);
    (!stem.is_empty()).then_some(stem)
}

/// macOS: папка в ~/Library/Application Support по имени бандла.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn mac_data_rel(stem: &str) -> Option<&'static str> {
    Some(match stem {
        "Google Chrome" => "Google/Chrome",
        "Google Chrome Beta" => "Google/Chrome Beta",
        "Chromium" => "Chromium",
        "Brave Browser" => "BraveSoftware/Brave-Browser",
        "Microsoft Edge" => "Microsoft Edge",
        "Yandex" => "Yandex/YandexBrowser",
        "Comet" => "Comet",
        "Vivaldi" => "Vivaldi",
        "Arc" => "Arc/User Data",
        "Opera" => "com.operasoftware.Opera",
        _ => return None,
    })
}

/// Windows: папка в %LOCALAPPDATA% по имени exe (chrome.exe — и у Google
/// Chrome, и у Chromium: различаем по пути установки).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_data_rel(stem: &str, app_path: &str) -> Option<&'static str> {
    let path = app_path.to_ascii_lowercase();
    Some(match stem.to_ascii_lowercase().as_str() {
        "chrome" if path.contains("\\chromium\\") => "Chromium\\User Data",
        "chrome" => "Google\\Chrome\\User Data",
        "msedge" => "Microsoft\\Edge\\User Data",
        "brave" => "BraveSoftware\\Brave-Browser\\User Data",
        "browser" if path.contains("yandex") => "Yandex\\YandexBrowser\\User Data",
        "vivaldi" => "Vivaldi\\User Data",
        "comet" => "Perplexity\\Comet\\User Data",
        _ => return None,
    })
}

/// Папка данных браузера (профили Default, Profile N) по пути к нему.
fn data_dir(app_path: &str) -> Option<PathBuf> {
    let stem = app_stem(app_path)?;
    #[cfg(not(target_os = "windows"))]
    return Some(dirs::home_dir()?.join("Library/Application Support").join(mac_data_rel(stem)?));
    #[cfg(target_os = "windows")]
    return Some(dirs::data_local_dir()?.join(windows_data_rel(stem, app_path)?));
}

/// Имя расширения из манифеста; `__MSG_x__` — из _locales (ru → en → default).
fn ext_name(ext_dir: &Path, manifest: &Value) -> String {
    let raw = manifest.get("name").and_then(Value::as_str).unwrap_or("Расширение");
    let Some(key) = raw.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else {
        return raw.to_string();
    };
    let default = manifest.get("default_locale").and_then(Value::as_str).unwrap_or("en");
    for loc in ["ru", "en", "en_US", default] {
        let f = ext_dir.join("_locales").join(loc).join("messages.json");
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(msgs) = serde_json::from_str::<Value>(&text) else { continue };
        // Ключи сообщений нечувствительны к регистру.
        if let Some(obj) = msgs.as_object() {
            if let Some((_, v)) = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                if let Some(m) = v.get("message").and_then(Value::as_str) {
                    return m.to_string();
                }
            }
        }
    }
    raw.to_string()
}

/// Перехватчики в настройках одного профиля (без файловой системы — для тестов).
fn scan_prefs(prefs: &[Value]) -> Vec<String> {
    let mut ids = Vec::new();
    for p in prefs {
        let Some(settings) = p.pointer("/extensions/settings").and_then(Value::as_object) else {
            continue;
        };
        for (id, e) in settings {
            let has_proxy = e.pointer("/preferences/proxy").is_some();
            let disabled = e
                .get("disable_reasons")
                .and_then(Value::as_array)
                .is_some_and(|r| !r.is_empty())
                || e.get("state").and_then(Value::as_i64) == Some(0);
            if has_proxy && !disabled && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
    }
    ids
}

fn scan(data: &Path) -> Vec<ProxyExtension> {
    let Ok(entries) = std::fs::read_dir(data) else { return Vec::new() };
    let mut out: Vec<ProxyExtension> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        let profile = entry.path();
        let prefs: Vec<Value> = ["Secure Preferences", "Preferences"]
            .iter()
            .filter_map(|f| std::fs::read_to_string(profile.join(f)).ok())
            .filter_map(|t| serde_json::from_str(&t).ok())
            .collect();
        for id in scan_prefs(&prefs) {
            if out.iter().any(|e| e.id == id) {
                continue;
            }
            let ext_root = profile.join("Extensions").join(&id);
            let version_dir = std::fs::read_dir(&ext_root)
                .ok()
                .and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path())));
            let name = version_dir
                .as_ref()
                .and_then(|d| {
                    let m: Value = serde_json::from_str(&std::fs::read_to_string(d.join("manifest.json")).ok()?).ok()?;
                    Some(ext_name(d, &m))
                })
                .unwrap_or_else(|| "расширение".into());
            out.push(ProxyExtension { id, name });
        }
    }
    out
}

/// Перехватчики прокси у браузера по пути к его .app / .exe. Кэш по mtime
/// файла настроек: опрос статуса идёт раз в 5 секунд, а JSON профиля — мегабайты.
pub fn proxy_extensions(app_path: &str) -> Vec<ProxyExtension> {
    static CACHE: Mutex<Option<HashMap<PathBuf, (SystemTime, Vec<ProxyExtension>)>>> = Mutex::new(None);
    let Some(data) = data_dir(app_path) else {
        return Vec::new();
    };
    let mtime = std::fs::metadata(data.join("Default/Secure Preferences"))
        .or_else(|_| std::fs::metadata(data.join("Default/Preferences")))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Ok(mut guard) = CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        if let Some((t, v)) = cache.get(&data) {
            if *t == mtime {
                return v.clone();
            }
        }
        let found = scan(&data);
        cache.insert(data, (mtime, found.clone()));
        return found;
    }
    Vec::new()
}

/// Адрес страницы расширения в браузере. id расширения Chromium — 32 буквы
/// a–p; другое в командную строку и AppleScript не пускаем.
pub fn extension_page_url(app_path: &str, ext_id: &str) -> Result<String, String> {
    if ext_id.len() != 32 || !ext_id.chars().all(|c| ('a'..='p').contains(&c)) {
        return Err("Некорректный идентификатор расширения".into());
    }
    let stem = app_stem(app_path).unwrap_or_default().to_ascii_lowercase();
    let scheme = match stem.as_str() {
        "yandex" | "browser" => "browser",
        "msedge" | "microsoft edge" => "edge",
        _ => "chrome",
    };
    Ok(format!("{scheme}://extensions/?id={ext_id}"))
}

/// Открыть в браузере страницу расширения, чтобы его отключить.
#[cfg(not(target_os = "windows"))]
pub fn open_extension_page(app_path: &str, ext_id: &str) -> Result<(), String> {
    let app = Path::new(app_path.trim_end_matches('/'))
        .file_stem()
        .map(|s| s.to_string_lossy().replace('"', ""))
        .ok_or("Браузер не найден")?;
    let url = extension_page_url(app_path, ext_id)?;
    // chrome:// нельзя открыть «как ссылку» снаружи — только новой вкладкой
    // через AppleScript самого браузера.
    let script = format!(
        "tell application \"{app}\"\nactivate\nif (count of windows) = 0 then make new window\nmake new tab at end of tabs of front window with properties {{URL:\"{url}\"}}\nend tell"
    );
    let ok = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(format!("Откройте в браузере {url} и отключите расширение"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_only_enabled_proxy_controllers() {
        let prefs = vec![json!({"extensions": {"settings": {
            "foxy": {"disable_reasons": [], "preferences": {"proxy": {"mode": "system"}}},
            "omega": {"disable_reasons": [1], "preferences": {"proxy": {"mode": "fixed_servers"}}},
            "adblock": {"disable_reasons": []},
            "legacy": {"state": 0, "preferences": {"proxy": {}}}
        }}})];
        assert_eq!(scan_prefs(&prefs), vec!["foxy".to_string()]);
    }

    /// Живая проверка: cargo test live_chrome -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_chrome() {
        println!("chrome → {:?}", proxy_extensions("/Applications/Google Chrome.app"));
    }

    #[test]
    fn unknown_app_has_no_data_dir() {
        assert!(mac_data_rel("Telegram").is_none());
        assert_eq!(app_stem("/Applications/Google Chrome.app/"), Some("Google Chrome"));
        assert_eq!(mac_data_rel("Google Chrome"), Some("Google/Chrome"));
    }

    #[test]
    fn windows_profiles_by_exe() {
        let rel = |p: &str| windows_data_rel(app_stem(p).unwrap(), p);
        assert_eq!(rel("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"), Some("Google\\Chrome\\User Data"));
        assert_eq!(rel("C:\\Users\\u\\AppData\\Local\\Chromium\\Application\\chrome.exe"), Some("Chromium\\User Data"));
        assert_eq!(rel("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe"), Some("Microsoft\\Edge\\User Data"));
        assert_eq!(rel("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"), Some("BraveSoftware\\Brave-Browser\\User Data"));
        assert_eq!(rel("D:\\Telegram Desktop\\Telegram.exe"), None);
    }

    #[test]
    fn extension_page_only_for_real_extension_ids() {
        let id = "gcknhkkoolaabfmlnjonogaaifnjlfnp";
        assert_eq!(
            extension_page_url("C:\\x\\chrome.exe", id).unwrap(),
            format!("chrome://extensions/?id={id}")
        );
        assert!(extension_page_url("C:\\x\\msedge.exe", id).unwrap().starts_with("edge://"));
        assert!(extension_page_url("/Applications/Yandex.app", id).unwrap().starts_with("browser://"));
        assert!(extension_page_url("C:\\x\\chrome.exe", "foxy").is_err());
        assert!(extension_page_url("C:\\x\\chrome.exe", "gcknhkkoolaabfmlnjonogaaifnjlfn\"").is_err());
    }
}
