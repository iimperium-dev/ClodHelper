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

/// Установленные Chromium-браузеры (имя, путь к .exe / .app) — для «Сайтов»:
/// расширение-перехватчик в любом из них ломает сайты, даже если браузера
/// нет в «Приложениях».
pub fn installed_browsers() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let bases: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
            .collect();
        let candidates: &[(&str, &str)] = &[
            ("Google Chrome", "Google\\Chrome\\Application\\chrome.exe"),
            ("Microsoft Edge", "Microsoft\\Edge\\Application\\msedge.exe"),
            ("Brave", "BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            ("Яндекс Браузер", "Yandex\\YandexBrowser\\Application\\browser.exe"),
            ("Vivaldi", "Vivaldi\\Application\\vivaldi.exe"),
            ("Chromium", "Chromium\\Application\\chrome.exe"),
            ("Comet", "Perplexity\\Comet\\Application\\comet.exe"),
        ];
        for (name, rel) in candidates {
            if let Some(exe) = bases.iter().map(|b| b.join(rel)).find(|p| p.exists()) {
                out.push((name.to_string(), exe.to_string_lossy().to_string()));
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let dirs: Vec<PathBuf> = [Some(PathBuf::from("/Applications")), dirs::home_dir().map(|h| h.join("Applications"))]
            .into_iter()
            .flatten()
            .collect();
        for name in ["Google Chrome", "Microsoft Edge", "Brave Browser", "Yandex", "Vivaldi", "Chromium", "Comet", "Arc"] {
            if let Some(app) = dirs.iter().map(|d| d.join(format!("{name}.app"))).find(|p| p.exists()) {
                out.push((name.to_string(), app.to_string_lossy().to_string()));
            }
        }
    }
    out
}

/// Программа, которую «Сайты» не трогают (см. мост: bypass_apps).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BypassApp {
    pub name: String,
    /// Папка программы: все её exe внутри (у антидетект-браузера — все профили).
    pub dir: String,
    /// Запущена сейчас (только для подсказок в UI).
    #[serde(default)]
    pub running: bool,
}

/// Папка программы по пути к её exe: «…\Google\Chrome\Application\chrome.exe»
/// → «…\Google\Chrome»; «…\Local\1browser\1browser\Application\1browser.exe»
/// → «…\Local\1browser» (двойная папка — весь продукт с его ядрами профилей).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn app_dir(exe: &str) -> Option<String> {
    // macOS: программа — это бандл .app целиком (исполняемые файлы и
    // помощники браузера лежат внутри него).
    #[cfg(not(target_os = "windows"))]
    {
        let i = exe.find(".app/").map(|i| i + 4).or_else(|| exe.trim_end_matches('/').ends_with(".app").then(|| exe.trim_end_matches('/').len()))?;
        let bundle = &exe[..i];
        return (Path::new(bundle).components().count() >= 3).then(|| bundle.to_string());
    }
    #[cfg_attr(not(target_os = "windows"), allow(unreachable_code))]
    let p = Path::new(exe);
    let mut dir = p.parent()?.to_path_buf();
    if dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Application")) {
        dir = dir.parent()?.to_path_buf();
    }
    if let (Some(name), Some(parent)) = (dir.file_name(), dir.parent()) {
        if parent.file_name().is_some_and(|pn| pn.eq_ignore_ascii_case(name)) {
            dir = parent.to_path_buf();
        }
    }
    // Не папка-«корзина» вроде диска, Program Files или AppData\Local целиком.
    let s = dir.to_string_lossy().to_string();
    let too_wide = dir.components().count() < 3
        || [dirs::data_local_dir(), dirs::data_dir(), std::env::var_os("ProgramFiles").map(PathBuf::from), std::env::var_os("ProgramFiles(x86)").map(PathBuf::from)]
            .into_iter()
            .flatten()
            .any(|w| w == dir);
    (!too_wide).then_some(s)
}

/// Кандидаты в «Не трогать»: установленные Chromium-браузеры и запущенные
/// программы на движке Chromium (рядом с exe лежит chrome_elf.dll — так
/// находятся и антидетект-браузеры вроде 1browser, которых нет в списках).
pub fn bypass_candidates() -> Vec<BypassApp> {
    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut out: Vec<BypassApp> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet));
        let mut running: Vec<String> = sys
            .processes()
            .values()
            .filter_map(|p| p.exe().map(|e| e.to_string_lossy().to_string()))
            .collect();
        running.sort();
        running.dedup();
        let is_chromium = |exe: &str| {
            let Some(dir) = Path::new(exe).parent() else { return false };
            dir.join("chrome_elf.dll").exists()
                || std::fs::read_dir(dir)
                    .map(|rd| rd.flatten().any(|e| e.path().join("chrome_elf.dll").exists()))
                    .unwrap_or(false)
        };
        let me = std::env::current_exe().ok().and_then(|e| e.to_str().and_then(app_dir));
        let push = |name: String, exe: &str, is_running: bool, out: &mut Vec<BypassApp>| {
            let Some(dir) = app_dir(exe) else { return };
            if me.as_deref() == Some(dir.as_str()) {
                return;
            }
            if let Some(found) = out.iter_mut().find(|a| a.dir.eq_ignore_ascii_case(&dir)) {
                found.running |= is_running;
                return;
            }
            out.push(BypassApp { name, dir, running: is_running });
        };
        for (name, exe) in installed_browsers() {
            let is_running = running.iter().any(|r| r.eq_ignore_ascii_case(&exe));
            push(name, &exe, is_running, &mut out);
        }
        for exe in running.iter().filter(|e| e.to_ascii_lowercase().ends_with(".exe") && is_chromium(e)) {
            let name = app_stem(exe).unwrap_or("Программа").to_string();
            push(name, exe, true, &mut out);
        }
    }
    // macOS: установленные браузеры и запущенные программы, которые
    // зарегистрированы как браузер (открывают ссылки http) — так находятся и
    // антидетект-браузеры, которых нет в списке известных.
    #[cfg(not(target_os = "windows"))]
    {
        let running: Vec<String> = std::process::Command::new("/bin/ps")
            .args(["-axo", "comm="])
            .output()
            .map(|o| {
                let mut v: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| app_dir(l.trim())).collect();
                v.sort();
                v.dedup();
                v
            })
            .unwrap_or_default();
        let me = std::env::current_exe().ok().and_then(|e| e.to_str().and_then(app_dir));
        let push = |name: String, bundle: String, out: &mut Vec<BypassApp>| {
            if me.as_deref() == Some(bundle.as_str()) || out.iter().any(|a| a.dir == bundle) {
                return;
            }
            let is_running = running.iter().any(|r| r == &bundle);
            out.push(BypassApp { name, dir: bundle, running: is_running });
        };
        for (name, app) in installed_browsers() {
            push(name, app.trim_end_matches('/').to_string(), &mut out);
        }
        for bundle in running.iter().filter(|b| opens_http(b)) {
            let name = Path::new(bundle).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            push(name, bundle.clone(), &mut out);
        }
    }
    out
}

/// macOS: бандл — Chromium-браузер (известный или открывающий ссылки http
/// и собранный на Chromium: внутри есть «… Framework.framework»).
#[cfg(not(target_os = "windows"))]
pub fn is_chromium_browser(app: &str) -> bool {
    let Some(bundle) = app_dir(app) else { return false };
    if installed_browsers().iter().any(|(_, a)| a.trim_end_matches('/') == bundle) {
        return true;
    }
    let chromium = std::fs::read_dir(Path::new(&bundle).join("Contents/Frameworks"))
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().ends_with(" Framework.framework")))
        .unwrap_or(false);
    chromium && opens_http(&bundle)
}

/// macOS: бандл объявляет схему http в CFBundleURLTypes — это браузер.
#[cfg(not(target_os = "windows"))]
fn opens_http(bundle: &str) -> bool {
    let plist = Path::new(bundle).join("Contents/Info.plist");
    std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleURLTypes", "json", "-o", "-"])
        .arg(&plist)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("\"http\""))
        .unwrap_or(false)
}

/// Открыть страницу расширения в браузере по его пути (Windows: адрес
/// передаётся exe — открытый браузер просто покажет новую вкладку).
#[cfg(target_os = "windows")]
pub fn open_extension_page(app_path: &str, ext_id: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let url = extension_page_url(app_path, ext_id)?;
    crate::win::allow_foreground();
    std::process::Command::new(app_path)
        .arg(&url)
        .creation_flags(0x0800_0000)
        .spawn()
        .map(|_| ())
        .map_err(|_| format!("Откройте в браузере {url} и отключите расширение"))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_app_dir_is_bundle() {
        assert_eq!(
            app_dir("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome").as_deref(),
            Some("/Applications/Google Chrome.app")
        );
        assert_eq!(app_dir("/Applications/Arc.app").as_deref(), Some("/Applications/Arc.app"));
        assert_eq!(app_dir("/usr/bin/curl"), None, "не бандл — не программа для исключения");
    }

    /// Живая проверка: cargo test live_chromium -- --ignored --nocapture
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn live_chromium() {
        for a in ["Google Chrome", "Comet", "1browser", "Telegram", "Claude", "Visual Studio Code"] {
            println!("{a}: {}", is_chromium_browser(&format!("/Applications/{a}.app")));
        }
    }

    /// Живой список (ничего не меняет): cargo test live_bypass_candidates -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_bypass_candidates() {
        for c in bypass_candidates() {
            println!("{} | {} | running={}", c.name, c.dir, c.running);
        }
    }

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
        for a in bypass_candidates() {
            println!("кандидат «Не трогать»: {a:?}");
        }
        for (name, path) in installed_browsers() {
            println!("{name} ({path}) → {:?}", proxy_extensions(&path));
        }
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

    #[cfg(target_os = "windows")]
    #[test]
    fn app_dir_covers_the_whole_product() {
        assert_eq!(
            app_dir(r"C:\Program Files\Google\Chrome\Application\chrome.exe").as_deref(),
            Some(r"C:\Program Files\Google\Chrome")
        );
        assert_eq!(
            app_dir(r"C:\Users\u\AppData\Local\1browser\1browser\Application\1browser.exe").as_deref(),
            Some(r"C:\Users\u\AppData\Local\1browser"),
            "двойная папка 1browser\\1browser — весь продукт"
        );
        assert_eq!(app_dir(r"D:\Tools\MyBrowser\browser.exe").as_deref(), Some(r"D:\Tools\MyBrowser"));
        assert_eq!(app_dir(r"C:\x.exe"), None, "корень диска — слишком широко");
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
