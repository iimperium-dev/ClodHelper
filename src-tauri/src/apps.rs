//! Профили приложений, которые проксируются через мост.
//!
//! Модель: каждый профиль знает КАК его запустить (обычный exe с флагом
//! `--proxy-server`, MSIX-приложение через `shell:AppsFolder`, или кастомный
//! exe добавленный пользователем) и КАКИЕ процессы блокировать при killswitch
//! (сам процесс + его сетевые дети — node/git/python/агентный рантайм).
//!
//! Зачем `process_names` отдельно от пути запуска: killswitch-файрвол вешаем
//! по ИМЕНИ процесса, а не по полному пути. MSIX-приложения (Codex, Claude
//! Desktop) живут в `C:\Program Files\WindowsApps\<pkg>_<ВЕРСИЯ>_...` —
//! версионированный путь меняется при каждом апдейте из Store, и правило по
//! пути отвалилось бы. Имя (`ChatGPT.exe`) переживает апдейты.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::shim::LISTEN_PORT;

/// Как запускать приложение.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchKind {
    /// Обычный exe: запускаем напрямую с флагом `--proxy-server` и env-переменными.
    ExeFlag,
    /// MSIX-пакет из Microsoft Store: активируем через `shell:AppsFolder\<app_id>`.
    /// Флаг `--proxy-server` несём через ярлык (`.lnk`), т.к. в MSIX-активацию
    /// аргументы командной строки надёжнее доходят именно из ярлыка.
    Msix,
    /// Кастомный exe, добавленный пользователем. Ведём себя как ExeFlag, но
    /// помечаем отдельно чтобы UI показывал «своё приложение».
    Custom,
}

/// Профиль приложения для проксирования.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppProfile {
    /// Стабильный идентификатор (`vscode`, `codex`, или `custom-<n>`).
    pub id: String,
    /// Имя для UI («VS Code», «ChatGPT Codex»).
    pub name: String,
    pub kind: LaunchKind,
    /// Для ExeFlag/Custom — полный путь к exe. Для Msix — None.
    #[serde(default)]
    pub exe_path: Option<String>,
    /// Для Msix — AppUserModelID (`OpenAI.Codex_2p2nqsd0c76g0!App`). Иначе None.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Имена процессов для killswitch-файрвола (сам процесс + сетевые дети).
    /// Например Codex: ChatGPT.exe + node.exe + codex.exe + codex-command-runner.exe.
    #[serde(default)]
    pub process_names: Vec<String>,
    /// Включён ли профиль (проксируется). Пользователь отмечает галочкой.
    #[serde(default)]
    pub enabled: bool,
    /// Это встроенный пресет (нельзя удалить, только выключить) или кастомный.
    #[serde(default)]
    pub builtin: bool,
    /// Убран пользователем из списка (встроенные не удаляются, а прячутся;
    /// вернуть — через «Добавить»).
    #[serde(default)]
    pub hidden: bool,
    /// Свой прокси из пула (индекс) для приложений, запускаемых с
    /// --proxy-server (VS Code, свои программы). None — «Авто».
    /// Для Claude / Codex / Telegram выбор хранится в назначениях сервисов.
    #[serde(default)]
    pub proxy: Option<usize>,
    /// Весь трафик через прокси (true) или только AI-сайты (false).
    /// None — по умолчанию: свои программы целиком, встроенные — только AI.
    #[serde(default)]
    pub full_proxy: Option<bool>,
}

/// Runtime-статус профиля для UI — найдено ли приложение на этом ПК.
#[derive(Debug, Clone, Serialize)]
pub struct AppProfileStatus {
    pub id: String,
    pub name: String,
    pub kind: LaunchKind,
    pub enabled: bool,
    pub builtin: bool,
    pub hidden: bool,
    pub proxy: Option<usize>,
    pub full_proxy: bool,
    /// Найдено ли приложение (exe существует / MSIX-пакет установлен).
    pub installed: bool,
    /// Путь/AppID для показа в UI.
    pub location: Option<String>,
    /// Существует ли ярлык «<name> (proxy)» на рабочем столе.
    pub desktop_shortcut: bool,
    pub process_names: Vec<String>,
}

// ─── Пресеты ─────────────────────────────────────────────────────

/// Встроенные пресеты. Пути/AppID НЕ хардкодим намертво — при первом запросе
/// пробуем найти на диске; в settings храним то что нашли (или None).
#[cfg(not(target_os = "windows"))]
pub fn builtin_presets() -> Vec<AppProfile> {
    // macOS: все профили — обычные .app (exe_path = путь к бандлу), запуск
    // через `open -a … --args --proxy-server`. process_names — для единообразия
    // UI: killswitch на macOS недоступен.
    vec![
        crate::claude_desktop::preset(),
        AppProfile {
            id: "vscode".into(),
            name: "VS Code".into(),
            kind: LaunchKind::ExeFlag,
            exe_path: find_vscode_exe().map(|p| p.to_string_lossy().to_string()),
            app_id: None,
            process_names: vec!["Code".into(), "node".into(), "git".into()],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
        AppProfile {
            id: "codex".into(),
            name: "ChatGPT Codex".into(),
            kind: LaunchKind::ExeFlag,
            exe_path: find_codex_app().map(|p| p.to_string_lossy().to_string()),
            app_id: None,
            process_names: vec!["Codex".into(), "codex".into(), "node".into()],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
        // Telegram не понимает --proxy-server и HTTP-прокси: он ходит через
        // SOCKS-вход моста, настраивается в самом Telegram (telegram.rs),
        // а «Открыть» просто запускает его.
        AppProfile {
            id: TELEGRAM_ID.into(),
            name: "Telegram".into(),
            kind: LaunchKind::ExeFlag,
            exe_path: crate::macos::find_app(&["Telegram.app", "Telegram Desktop.app"])
                .map(|p| p.to_string_lossy().to_string()),
            app_id: None,
            process_names: vec!["Telegram".into()],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
    ]
}

/// Профиль Telegram.
pub const TELEGRAM_ID: &str = "telegram";
/// Бывший профиль «Терминал» (сборки до 0.7.0): убирается из настроек.
pub const OBSOLETE_IDS: &[&str] = &["terminal"];

/// На Mac Codex поставляется внутри десктопного ChatGPT.app.
#[cfg(not(target_os = "windows"))]
fn find_codex_app() -> Option<PathBuf> {
    crate::macos::find_app(&["Codex.app", "ChatGPT.app"])
}

#[cfg(target_os = "windows")]
pub fn builtin_presets() -> Vec<AppProfile> {
    vec![
        crate::claude_desktop::preset(),
        AppProfile {
            id: "vscode".into(),
            name: "VS Code".into(),
            kind: LaunchKind::ExeFlag,
            exe_path: find_vscode_exe().map(|p| p.to_string_lossy().to_string()),
            app_id: None,
            // Code.exe + сетевые дети, которые запускает Claude Code внутри
            // терминала VS Code. git/python могут ходить в сеть напрямую.
            process_names: vec![
                "Code.exe".into(),
                "node.exe".into(),
                "git.exe".into(),
                "python.exe".into(),
            ],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
        AppProfile {
            id: "codex".into(),
            name: "ChatGPT Codex".into(),
            kind: LaunchKind::Msix,
            exe_path: None,
            app_id: find_codex_app_id(),
            // Codex — Electron в MSIX: main-процесс ChatGPT.exe + встроенный
            // агентный рантайм (node.exe, codex.exe, codex-command-runner.exe).
            // Блокируем ВСЕ — иначе агент утечёт мимо моста своим node.
            process_names: vec![
                "ChatGPT.exe".into(),
                "Codex.exe".into(),
                "codex.exe".into(),
                "codex-command-runner.exe".into(),
                "codex-code-mode-host.exe".into(),
                "node.exe".into(),
            ],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
        // Telegram Desktop: ходит через SOCKS-вход моста (настройка — в его
        // окне прокси по ссылке tg://socks), «Открыть» — без параметров.
        AppProfile {
            id: TELEGRAM_ID.into(),
            name: "Telegram".into(),
            kind: LaunchKind::ExeFlag,
            exe_path: find_telegram_exe().map(|p| p.to_string_lossy().to_string()),
            app_id: None,
            process_names: vec!["Telegram.exe".into()],
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        },
    ]
}

/// Telegram Desktop на Windows. Сначала дешёвое: запись установщика (своя
/// папка вроде D:\Telegram Desktop), обычные папки, обработчик ссылок tg://;
/// затем «Приложения и возможности» и запущенный Telegram.exe (переносная
/// копия). Обработчик tg:// перезаписывает каждый запущенный Telegram, и при
/// втором аккаунте (копия exe под другим именем) он указывает на копию —
/// поэтому от него берём только Telegram.exe. Кэш 30 с: детект идёт при
/// каждом чтении настроек.
#[cfg(target_os = "windows")]
pub fn find_telegram_exe() -> Option<PathBuf> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Option<PathBuf>)>> = Mutex::new(None);
    if let Some((t, found)) = CACHE.lock().ok().and_then(|c| c.clone()) {
        if t.elapsed() < Duration::from_secs(30) {
            return found;
        }
    }
    let found = telegram_installer_entry()
        .or_else(|| {
            [dirs::config_dir(), dirs::data_local_dir().map(|d| d.join("Programs"))]
                .into_iter()
                .flatten()
                .map(|d| d.join("Telegram Desktop").join("Telegram.exe"))
                .find(|p| p.is_file())
        })
        .or_else(|| telegram_link_handler().filter(|p| is_telegram_exe_name(p)))
        .or_else(|| {
            crate::win::uninstall_entries()
                .into_iter()
                .filter(|(name, _)| name.starts_with("Telegram Desktop"))
                .filter_map(|(_, icon)| icon_exe(&icon).map(PathBuf::from))
                .find(|p| is_telegram_exe_name(p) && p.is_file())
        })
        .or_else(|| running_exes(|name, _| name.eq_ignore_ascii_case("Telegram.exe")).into_iter().next());
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), found.clone()));
    }
    found
}

/// Telegram.exe по записи его установщика (Inno Setup, AppId Telegram
/// Desktop) — прямое чтение одного ключа реестра.
#[cfg(target_os = "windows")]
fn telegram_installer_entry() -> Option<PathBuf> {
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{53F49750-6209-4FBF-9CA8-7A333C87D1ED}_is1";
    const KEY_32: &str =
        r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\{53F49750-6209-4FBF-9CA8-7A333C87D1ED}_is1";
    use crate::win::{hkcu, hklm, string};
    [(hkcu(), KEY), (hklm(), KEY), (hklm(), KEY_32)].iter().find_map(|(root, key)| {
        string(root, key, "InstallLocation")
            .map(|dir| PathBuf::from(dir).join("Telegram.exe"))
            .or_else(|| string(root, key, "DisplayIcon").and_then(|i| icon_exe(&i)).map(PathBuf::from))
            .filter(|p| p.is_file())
    })
}

/// Exe обработчика ссылок tg:// (HKCU) — чтение реестра, микросекунды.
/// Им может быть и копия Telegram под другим именем.
#[cfg(target_os = "windows")]
fn telegram_link_handler() -> Option<PathBuf> {
    let command = crate::win::string(&crate::win::hkcu(), r"Software\Classes\tg\shell\open\command", "")?;
    link_handler_exe(&command).filter(|p| p.is_file())
}

/// Пути exe запущенных процессов, подходящих под условие (имя образа, путь).
#[cfg(target_os = "windows")]
fn running_exes(matches: impl Fn(&str, &Path) -> bool) -> Vec<PathBuf> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut out: Vec<PathBuf> = Vec::new();
    for proc in sys.processes().values() {
        let Some(exe) = proc.exe() else { continue };
        if !out.iter().any(|p| p == exe) && matches(&proc.name().to_string_lossy(), exe) {
            out.push(exe.to_path_buf());
        }
    }
    out
}

/// Другие запущенные Telegram Desktop, кроме `main`: копии под своим именем
/// (второй аккаунт — «Vtoroy-Acc.exe» в отдельной папке) и другие установки.
/// Узнаём по описанию файла, а не по имени; читаем его только у тех, рядом с
/// кем лежит папка данных Telegram «tdata» (у копии она своя), — чтение
/// версий всех процессов заняло бы секунду.
#[cfg(target_os = "windows")]
pub fn other_running_telegrams(main: &str) -> Vec<PathBuf> {
    running_exes(|name, exe| {
        if same_path(&exe.to_string_lossy(), main) {
            return false;
        }
        name.eq_ignore_ascii_case("Telegram.exe")
            || (exe.parent().is_some_and(|d| d.join("tdata").is_dir())
                && crate::win::file_description(exe).as_deref() == Some("Telegram Desktop"))
    })
}

/// Exe из команды обработчика ссылок: «"D:\…\Telegram.exe" -- "%1"».
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn link_handler_exe(command: &str) -> Option<PathBuf> {
    let c = command.trim();
    let exe = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split_whitespace().next()?,
    };
    exe.to_lowercase().ends_with(".exe").then(|| PathBuf::from(exe))
}

/// Сам Telegram.exe, а не копия или чужая программа со ссылками tg://.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_telegram_exe_name(p: &Path) -> bool {
    let path = p.to_string_lossy();
    path.rsplit(['\\', '/']).next().is_some_and(|n| n.eq_ignore_ascii_case("Telegram.exe"))
}

// ─── Детект приложений ───────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
pub fn find_vscode_exe() -> Option<PathBuf> {
    crate::macos::find_app(&["Visual Studio Code.app"])
}

#[cfg(target_os = "windows")]
pub fn find_vscode_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local).join("Programs/Microsoft VS Code/Code.exe"),
        );
    }
    if let Ok(pf) = std::env::var("PROGRAMFILES") {
        candidates.push(PathBuf::from(&pf).join("Microsoft VS Code/Code.exe"));
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(&pf86).join("Microsoft VS Code/Code.exe"));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// AppUserModelID установленного Codex в форме `<PackageFamilyName>!App` —
/// этим запускается MSIX. Пакет — из реестра пакетов пользователя (не
/// Get-AppxPackage: тот стоит секунды, а статусы опрашиваются постоянно).
#[cfg(not(target_os = "windows"))]
pub fn find_codex_app_id() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
pub fn find_codex_app_id() -> Option<String> {
    // PackageFamilyName Codex стабилен между версиями (меняется только версия
    // в PackageFullName).
    crate::win::store_packages("OpenAI.Codex")
        .first()
        .map(|p| format!("{}!App", p.family()))
}

/// Проверить, установлен ли профиль на этом ПК прямо сейчас.
fn profile_installed(p: &AppProfile) -> (bool, Option<String>) {
    if p.id == crate::claude_desktop::PROFILE_ID {
        return match crate::claude_desktop::detect() {
            Some(i) => (true, i.app_id.or(Some(i.exe_path))),
            None => (false, None),
        };
    }
    match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => {
            // Пресет мог быть сохранён без пути (не найден при установке) —
            // перепроверяем детектом на случай если приложение поставили позже.
            let path = p
                .exe_path
                .clone()
                .filter(|s| PathBuf::from(s).exists())
                .or_else(|| match p.id.as_str() {
                    "vscode" => find_vscode_exe().map(|x| x.to_string_lossy().to_string()),
                    #[cfg(not(target_os = "windows"))]
                    "codex" => find_codex_app().map(|x| x.to_string_lossy().to_string()),
                    _ => None,
                });
            (path.is_some(), path)
        }
        LaunchKind::Msix => {
            let id = p.app_id.clone().or_else(|| {
                if p.id == "codex" {
                    find_codex_app_id()
                } else {
                    None
                }
            });
            (id.is_some(), id)
        }
    }
}

/// Файл программы профиля на этом ПК (exe или .app): сохранённый путь или
/// найденный детектом. None — не найдена или известна только как AppID.
pub fn profile_file(p: &AppProfile) -> Option<String> {
    let exists = |s: &String| Path::new(s).exists();
    p.exe_path.clone().filter(exists).or_else(|| {
        let (installed, location) = profile_installed(p);
        location.filter(|l| installed && exists(l))
    })
}

/// Приложение из Microsoft Store, известное только по AppID (файла в
/// закрытой WindowsApps не видно): AppID, если оно установлено.
pub fn profile_store_id(p: &AppProfile) -> Option<String> {
    if p.kind != LaunchKind::Msix {
        return None;
    }
    let (installed, id) = profile_installed(p);
    id.filter(|i| installed && i.contains('!'))
}

/// Один ли это файл: на Windows регистр и «/» против «\» не важны.
pub fn same_path(a: &str, b: &str) -> bool {
    if cfg!(target_os = "windows") {
        a.replace('/', "\\").to_lowercase() == b.replace('/', "\\").to_lowercase()
    } else {
        a == b
    }
}

// ─── Ярлык на рабочем столе ──────────────────────────────────────

/// Рабочий стол через известную папку Windows (dirs → SHGetKnownFolderPath):
/// при OneDrive Known Folder Move реальный Desktop лежит в OneDrive, и сырой
/// `USERPROFILE\Desktop` его не видит — ярлык «не находился», а запуск падал
/// в fallback без прокси.
pub fn desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir().or_else(|| {
        std::env::var("USERPROFILE")
            .ok()
            .map(|p| PathBuf::from(p).join("Desktop"))
    })
}

/// Расширение ярлыка: `.lnk` на Windows, `.app`-обёртка на macOS.
#[cfg(target_os = "windows")]
pub const SHORTCUT_EXT: &str = "lnk";
#[cfg(not(target_os = "windows"))]
pub const SHORTCUT_EXT: &str = "app";

fn desktop_shortcut_path(name: &str) -> Option<PathBuf> {
    Some(desktop_dir()?.join(format!("{name} (proxy).{SHORTCUT_EXT}")))
}

/// Собрать статус всех профилей для UI.
pub fn statuses(profiles: &[AppProfile]) -> Vec<AppProfileStatus> {
    profiles
        .iter()
        .map(|p| {
            let (installed, location) = profile_installed(p);
            let desktop_shortcut = desktop_shortcut_path(&p.name)
                .map(|x| x.exists())
                .unwrap_or(false);
            AppProfileStatus {
                id: p.id.clone(),
                name: p.name.clone(),
                kind: p.kind.clone(),
                enabled: p.enabled,
                builtin: p.builtin,
                hidden: p.hidden,
                proxy: p.proxy,
                full_proxy: full_proxy_of(p),
                installed,
                location,
                desktop_shortcut,
                process_names: p.process_names.clone(),
            }
        })
        .collect()
}

// ─── Запуск ──────────────────────────────────────────────────────

/// Весь трафик приложения через прокси? Своё приложение (браузер) — по
/// умолчанию целиком; встроенные — только AI-сайты (умный общий вход).
pub fn full_proxy_of(p: &AppProfile) -> bool {
    p.full_proxy.unwrap_or(p.kind == LaunchKind::Custom)
}

/// Через какой вход моста запускать приложение:
/// - выбран конкретный прокси → закреплённый вход 8892+N (всё через него);
/// - «весь трафик» → строгий вход 8890 (без DIRECT);
/// - Chromium-браузер → вход сайтов 8887 (только AI и сайты пользователя,
///   что бы ни стояло в глобальном режиме);
/// - иначе общий умный вход 8889.
pub fn launch_proxy_url(p: &AppProfile) -> String {
    let pool = crate::settings::effective_proxies(&crate::settings::load()).len();
    match p.proxy {
        // Закреплённые входы есть у моста ≥1.7 (macOS и Windows).
        Some(i) if i < pool && pinned_listeners_available() => {
            format!("http://127.0.0.1:{}", crate::tasks::PIN_PORT_BASE + i as u16)
        }
        _ if full_proxy_of(p) => crate::claude_policy::PROXY_URL.to_string(),
        // VS Code и прочие — на общий вход: терминальный режим «всё через
        // прокси» должен действовать и в их терминалах.
        _ if is_chromium_browser(p) && sites_listener_available() => {
            format!("http://127.0.0.1:{}", crate::tasks::SITES_PORT)
        }
        _ => local_proxy_url(),
    }
}

/// Профиль — Chromium-браузер (Chrome, Edge, Brave, Chromium).
pub fn is_chromium_browser(p: &AppProfile) -> bool {
    p.exe_path.as_deref().and_then(crate::browser_policy::policy_domain).is_some()
}

/// Режим браузера для раздела «Сайты».
pub fn browser_mode(p: &AppProfile) -> &'static str {
    let pool = crate::settings::effective_proxies(&crate::settings::load()).len();
    if p.proxy.is_some_and(|i| i < pool) || full_proxy_of(p) {
        "all"
    } else if sites_listener_available() {
        "sites"
    } else {
        "ai"
    }
}

/// Мост поднимает вход сайтов (≥1.9)? Проба версии exe кэшируется.
pub fn sites_listener_available() -> bool {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::SystemTime, bool)>> = Mutex::new(None);
    let Some(exe) = crate::shim::bridge_exe_path().filter(|p| p.exists()) else {
        return false;
    };
    let mtime = std::fs::metadata(&exe).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    if let Ok(mut c) = CACHE.lock() {
        if let Some((t, v)) = *c {
            if t == mtime {
                return v;
            }
        }
        let v = crate::tasks::bridge_supports_sites(&exe);
        *c = Some((mtime, v));
        return v;
    }
    false
}

/// WebRTC (звонки, видео в браузере) ходит по UDP к STUN-серверам мимо
/// любого HTTP-прокси — сайты так видят настоящий IP (ipleak: «WebRTC
/// detection»). Флаг Chromium запрещает WebRTC ходить мимо прокси; на
/// не-Chromium приложения не влияет.
const WEBRTC_GUARD: &str = "--webrtc-ip-handling-policy=disable_non_proxied_udp";

/// Программа, которая не запускается с флагами прокси Chromium в командной
/// строке. Gemini для Windows (Electron, `%LOCALAPPDATA%\Google\Gemini\`)
/// с `--proxy-server` пишет в лог «refusing to start» и сразу выходит; через
/// мост его ведёт системный PAC. Матчим по папке установки — внутри неё
/// и заглушка Gemini.exe, и app-<версия>\Gemini.exe, и GeminiAppLauncher.exe.
pub fn rejects_proxy_switch(exe: &str) -> bool {
    let p = exe.replace('/', "\\").to_lowercase();
    let file = p.rsplit('\\').next().unwrap_or("");
    p.contains("\\google\\gemini\\") && file.starts_with("gemini") && file.ends_with(".exe")
}

/// Мост поднимает закреплённые входы (≥1.7)? Проба версии exe кэшируется.
fn pinned_listeners_available() -> bool {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::SystemTime, bool)>> = Mutex::new(None);
    let Some(exe) = crate::shim::bridge_exe_path().filter(|p| p.exists()) else {
        return false;
    };
    let mtime = std::fs::metadata(&exe).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    if let Ok(mut c) = CACHE.lock() {
        if let Some((t, v)) = *c {
            if t == mtime {
                return v;
            }
        }
        let v = crate::tasks::bridge_supports_pin_listen(&exe);
        *c = Some((mtime, v));
        return v;
    }
    false
}

/// Строка прокси для локального моста, которую впрыскиваем приложению.
fn local_proxy_url() -> String {
    format!("http://127.0.0.1:{LISTEN_PORT}")
}

/// Запустить приложение через мост.
///
/// ExeFlag/Custom: прямой запуск exe с `--proxy-server` + env HTTP(S)_PROXY.
///   Дочерние процессы (терминал, node, git) наследуют env → Claude Code
///   внутри VS Code проксируется автоматически.
/// Msix: активируем через ярлык `.lnk` (несёт `--proxy-server`), если он есть;
///   иначе — голая MSIX-активация (UI-прокси может не подхватиться, но
///   killswitch-файрвол всё равно держит трафик в мосте).
/// Приложение уже открыто мимо моста: настройки прокси применяются только при
/// старте, поэтому закрываем его штатно (как Cmd+Q — с вопросами о
/// несохранённом) и запускаем заново через мост.
#[cfg(not(target_os = "windows"))]
pub fn relaunch(p: &AppProfile) -> Result<(), String> {
    let (installed, location) = profile_installed(p);
    if !installed {
        return Err(format!("{} не найден", p.name));
    }
    let exe = location.ok_or_else(|| format!("Путь к {} не определён", p.name))?;
    let name = std::path::Path::new(exe.trim_end_matches('/'))
        .file_stem()
        .map(|s| s.to_string_lossy().replace('"', ""))
        .ok_or_else(|| format!("Путь к {} не определён", p.name))?;
    let is_running = || {
        Command::new("/usr/bin/osascript")
            .args(["-e", &format!("application \"{name}\" is running")])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false)
    };
    if is_running() {
        let _ = Command::new("/usr/bin/osascript")
            .args(["-e", &format!("tell application \"{name}\" to quit")])
            .status();
        // Ждём до 20с: приложение может спросить про несохранённое.
        for _ in 0..40 {
            if !is_running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if is_running() {
            return Err(format!("{} не закрылся — закройте его и нажмите «Открыть»", p.name));
        }
        // Chromium-браузер: вернуть все окна и вкладки, как было до закрытия
        // (на Windows это делает закрытие «как перед обновлением системы»).
        if crate::browser_ext::is_chromium_browser(&exe) {
            return open_app_with(&exe, &launch_proxy_url(p), &p.name, &["--restore-last-session"]);
        }
    }
    launch(p)
}

/// Windows: закрыть так же, как Windows закрывает программы для обновления
/// (win::end_session — программа сохраняет состояние и выходит), дождаться
/// выхода и запустить через мост. Кто после этого не вышел сам (Electron
/// ждёт, что его завершит система), — завершаем: состояние уже сохранено.
/// Трогаем только главный процесс программы (по пути к exe): не node.exe
/// Codex и не claude.exe Claude Code в терминалах.
#[cfg(target_os = "windows")]
pub fn relaunch(p: &AppProfile) -> Result<(), String> {
    use std::time::{Duration, Instant};
    let Some(image) = p.process_names.first().map(|n| {
        let n = n.to_lowercase();
        if n.ends_with(".exe") { n } else { format!("{n}.exe") }
    }) else {
        return launch(p);
    };
    // Claude Desktop из Store: в профиле — AppID, путь к exe — из детекта.
    let exe = if p.id == crate::claude_desktop::PROFILE_ID {
        crate::claude_desktop::detect().map(|i| i.exe_path)
    } else {
        profile_installed(p).1
    }
    .filter(|l| l.to_lowercase().ends_with(".exe") && l.contains('\\'));
    let pids = || main_pids(&image, exe.as_deref());
    let wait = |secs: u64| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !pids().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
        }
        pids().is_empty()
    };
    let not_closed = || format!("{} не закрылся — закройте его и нажмите «Открыть»", p.name);
    let running = pids();
    if running.is_empty() {
        return launch(p);
    }
    if !crate::win::end_session(&running) {
        return Err(not_closed());
    }
    if !wait(5) {
        kill_pids(&pids());
        if !wait(5) {
            return Err(not_closed());
        }
    }
    // Браузер на Chromium: вернуть все окна и вкладки, как его собственный
    // перезапуск (chrome://restart), что бы ни стояло в «При запуске».
    let restore = exe.as_deref().and_then(crate::browser_policy::policy_domain).is_some();
    if restore && matches!(p.kind, LaunchKind::ExeFlag | LaunchKind::Custom) {
        return launch_exe(p, &["--restore-last-session"]);
    }
    launch(p)
}

/// PID главного процесса программы: имя образа и, если известен, путь к exe.
#[cfg(target_os = "windows")]
fn main_pids(image: &str, exe: Option<&str>) -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    sys.processes()
        .iter()
        .filter(|(_, proc)| proc.name().to_string_lossy().eq_ignore_ascii_case(image))
        .filter(|(_, proc)| match exe {
            Some(path) => proc.exe().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(path)),
            None => true,
        })
        .map(|(pid, _)| pid.as_u32())
        .collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn kill_pids(pids: &[u32]) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    for pid in pids {
        if let Some(proc) = sys.process(Pid::from_u32(*pid)) {
            proc.kill();
        }
    }
}

pub fn launch(p: &AppProfile) -> Result<(), String> {
    // Уже запущенная программа на «Открыть» выводит своё окно вперёд.
    #[cfg(target_os = "windows")]
    crate::win::allow_foreground();
    if p.id == crate::claude_desktop::PROFILE_ID {
        return crate::claude_desktop::launch();
    }
    match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => launch_exe(p, &[]),
        LaunchKind::Msix => launch_msix(p),
    }
}

/// `extra` — дополнительные флаги запуска (например, восстановить сессию).
fn launch_exe(p: &AppProfile, extra: &[&str]) -> Result<(), String> {
    let (installed, location) = profile_installed(p);
    if !installed {
        return Err(format!("{} не найден на этом ПК", p.name));
    }
    let exe = location.ok_or_else(|| format!("Путь к {} не определён", p.name))?;
    // Свои программы (браузер и т.п.) — ЦЕЛИКОМ через прокси: строгий вход
    // 8890 без DIRECT. Общий вход в умном режиме пускал бы не-AI сайты
    // напрямую (2ip.io показывал IP пользователя), а человек, добавивший
    // браузер, ждёт, что весь браузер идёт через прокси.
    let proxy = launch_proxy_url(p);

    // Терминал VS Code (и Claude Code в нём) покрывается глобальными User
    // env-переменными (env_proxy.rs) — правка settings.json больше не нужна.

    // macOS: бандл запускаем через LaunchServices. Env через `open` не
    // передаётся — его дают глобальные переменные (launchctl setenv).
    #[cfg(not(target_os = "windows"))]
    {
        if p.id == TELEGRAM_ID {
            // Прокси Telegram — в его собственных настройках (SOCKS моста):
            // флаги командной строки он не понимает.
            let ok = Command::new("/usr/bin/open")
                .arg("-a")
                .arg(&exe)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            return if ok { Ok(()) } else { Err("Не удалось запустить Telegram".into()) };
        }
        if exe.trim_end_matches('/').ends_with(".app") {
            return open_app(&exe, &proxy, &p.name);
        }
    }

    if p.id == TELEGRAM_ID {
        // Прокси Telegram — в его настройках (SOCKS моста), флагов он не знает.
        let mut cmd = Command::new(&exe);
        apply_no_window(&mut cmd);
        cmd.spawn().map_err(|e| format!("Не удалось запустить Telegram: {e}"))?;
        return Ok(());
    }
    if rejects_proxy_switch(&exe) {
        // Gemini с --proxy-server в командной строке отказывается стартовать
        // («refusing to start — … network-override switch»). Запускаем как
        // есть: Electron берёт системный прокси, и его ведёт PAC моста
        // («Сайты во всех браузерах», gemini.google.com в AI-доменах).
        let mut cmd = Command::new(&exe);
        apply_no_window(&mut cmd);
        cmd.spawn()
            .map_err(|e| format!("Не удалось запустить {}: {e}", p.name))?;
        return Ok(());
    }
    let mut cmd = Command::new(&exe);
    cmd.arg(format!("--proxy-server={proxy}"));
    cmd.arg(WEBRTC_GUARD);
    cmd.args(extra);
    // env для дочерних процессов (терминал → Claude Code → bash/node/git).
    cmd.env("HTTP_PROXY", &proxy);
    cmd.env("HTTPS_PROXY", &proxy);
    cmd.env("http_proxy", &proxy);
    cmd.env("https_proxy", &proxy);
    // NO_PROXY: локалхост и служебные — не гонять через мост.
    cmd.env("NO_PROXY", "localhost,127.0.0.1,::1");
    apply_no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("Не удалось запустить {}: {e}", p.name))?;
    Ok(())
}

/// Windows: открыть адрес в браузере с теми же флагами, что и «Открыть».
/// Уже запущенный браузер флаги игнорирует и открывает адрес вкладкой; не
/// запущенный — стартует сразу через мост, а не мимо него.
#[cfg(target_os = "windows")]
pub fn open_in_browser(p: &AppProfile, url: &str) -> Result<(), String> {
    let (installed, location) = profile_installed(p);
    let exe = location
        .filter(|_| installed)
        .ok_or_else(|| format!("{} не найден на этом ПК", p.name))?;
    crate::win::allow_foreground();
    let mut cmd = Command::new(&exe);
    cmd.arg(format!("--proxy-server={}", launch_proxy_url(p)));
    cmd.arg(WEBRTC_GUARD);
    cmd.arg(url);
    apply_no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("Не удалось открыть {}: {e}", p.name))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn open_app(app: &str, proxy: &str, name: &str) -> Result<(), String> {
    open_app_with(app, proxy, name, &[])
}

/// open_app с дополнительными флагами запуска.
#[cfg(not(target_os = "windows"))]
fn open_app_with(app: &str, proxy: &str, name: &str, extra: &[&str]) -> Result<(), String> {
    let out = Command::new("/usr/bin/open")
        .arg("-a")
        .arg(app)
        .arg("--args")
        .arg(format!("--proxy-server={proxy}"))
        .arg(WEBRTC_GUARD)
        .args(extra)
        .output()
        .map_err(|e| format!("Не удалось запустить {name}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Не удалось запустить {name}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn launch_msix(p: &AppProfile) -> Result<(), String> {
    // MSIX-приложение (Codex) НЕ проксируется через `shell:AppsFolder` —
    // активатор глотает аргументы, а Electron/Chromium НЕ читает env-прокси
    // (только `--proxy-server`). Проверено вживую: `shell:AppsFolder` → Codex
    // идёт мимо моста.
    //
    // Рабочий способ (подтверждён): запустить главный exe ПРЯМО из папки
    // установленного пакета WindowsApps с флагом `--proxy-server`. Путь
    // версионированный — резолвим на лету через Get-AppxPackage, поэтому
    // апдейт Store не ломает. Тест: 4 соединения к мосту, 0 прямых.
    let proxy = local_proxy_url();

    if let Some(exe) = msix_main_exe(p) {
        let mut cmd = Command::new(&exe);
        cmd.arg(format!("--proxy-server={proxy}"));
        cmd.arg(WEBRTC_GUARD);
        // env — на случай если рантайм внутри (node/codex) читает их.
        cmd.env("HTTP_PROXY", &proxy);
        cmd.env("HTTPS_PROXY", &proxy);
        cmd.env("http_proxy", &proxy);
        cmd.env("https_proxy", &proxy);
        cmd.env("NO_PROXY", "localhost,127.0.0.1,::1");
        apply_no_window(&mut cmd);
        cmd.spawn()
            .map_err(|e| format!("Не удалось запустить {}: {e}", p.name))?;
        return Ok(());
    }

    // Fallback: если не нашли exe в пакете — голая активация (без прокси, но
    // хотя бы откроется; killswitch подстрахует от утечки).
    let app_id = p
        .app_id
        .clone()
        .or_else(|| {
            if p.id == "codex" {
                find_codex_app_id()
            } else {
                None
            }
        })
        .ok_or_else(|| format!("{} не установлен (MSIX не найден)", p.name))?;
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", "start", "", &format!("shell:AppsFolder\\{app_id}")]);
    apply_no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("Не удалось запустить {}: {e}", p.name))?;
    Ok(())
}

/// Найти главный exe MSIX-приложения в установленном пакете (для прямого
/// запуска с `--proxy-server`). Путь версионированный — резолвим каждый раз.
fn msix_main_exe(p: &AppProfile) -> Option<PathBuf> {
    let root = if p.id == "codex" {
        codex_install_root()?
    } else {
        // Для будущих MSIX-профилей: ищем корень по app_id (PackageFamilyName!App).
        let app_id = p.app_id.as_ref()?;
        let family = app_id.split('!').next()?;
        appx_install_location(family)?
    };
    // Главный exe: у Codex это app\ChatGPT.exe. Ищем первый .exe с именем из
    // process_names (первое имя = главный процесс), иначе — эвристикой.
    let main_name = p
        .process_names
        .first()
        .cloned()
        .unwrap_or_else(|| "app.exe".into());
    let direct = root.join("app").join(&main_name);
    if direct.exists() {
        return Some(direct);
    }
    // Иначе ищем рекурсивно.
    find_in_dir(&root, &main_name)
        .into_iter()
        .next()
        .map(PathBuf::from)
}

/// Папка установленного MSIX-пакета по PackageFamilyName (Имя_Издатель).
#[cfg(target_os = "windows")]
fn appx_install_location(family: &str) -> Option<PathBuf> {
    let name = family.split('_').next()?;
    crate::win::store_packages(name)
        .into_iter()
        .find(|p| p.family().eq_ignore_ascii_case(family))
        .map(|p| p.root)
        .filter(|root| root.exists())
}

#[cfg(not(target_os = "windows"))]
fn appx_install_location(_family: &str) -> Option<PathBuf> {
    None
}

// ─── Ярлык (Desktop / Taskbar) ───────────────────────────────────

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutTarget {
    Desktop,
    /// Меню Пуск. На Win11 программный «Pin to taskbar» удалён Microsoft, а вот
    /// ярлык в Start Menu создаётся штатно и оттуда пользователь может закрепить
    /// на панель задач правым кликом. Кладём туда + открываем папку.
    StartMenu,
}

/// Создать ярлык «<name> (proxy)» на рабочем столе или закрепить в панель задач.
///
/// Ярлык несёт `--proxy-server=http://127.0.0.1:<port>` в аргументах и, для
/// exe-приложений, задаёт рабочую среду. Для MSIX цель ярлыка — сам
/// `shell:AppsFolder\<app_id>`, аргумент прокси добавляется в поле Arguments.
///
/// Реализовано через PowerShell (WScript.Shell.CreateShortcut) — нативно, без
/// сторонних зависимостей. Возвращает путь к созданному ярлыку.
///
/// macOS: `.app`-обёртка (см. macos::create_app_wrapper) на рабочем столе или
/// в `~/Applications` («Программы» — оттуда её можно перетащить в Dock).
#[cfg(not(target_os = "windows"))]
pub fn create_shortcut(
    p: &AppProfile,
    target: ShortcutTarget,
) -> Result<String, String> {
    use crate::macos::{self, sh_quote};

    let (installed, location) = profile_installed(p);
    if !installed {
        return Err(format!("{} не найден — сначала установите приложение", p.name));
    }
    let app = location.ok_or_else(|| "Нет пути к приложению".to_string())?;
    let body = if p.id == crate::claude_desktop::PROFILE_ID {
        // Лаунчер сам проверит мост/политику и покажет причину отказа.
        let launcher = crate::claude_desktop::install_launcher()?;
        format!(
            "exec /bin/sh {} --show-error",
            sh_quote(&launcher.to_string_lossy())
        )
    } else {
        // Тот же вход, что и у «Открыть»: свой прокси / весь трафик.
        format!(
            "exec /usr/bin/open -a {} --args {} {}",
            sh_quote(&app),
            sh_quote(&format!("--proxy-server={}", launch_proxy_url(p))),
            WEBRTC_GUARD
        )
    };
    let dir = match target {
        ShortcutTarget::Desktop => desktop_dir(),
        ShortcutTarget::StartMenu => dirs::home_dir().map(|h| h.join("Applications")),
    }
    .ok_or_else(|| "Не нашёл папку для ярлыка".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let name = format!("{} (proxy)", p.name);
    let dest = dir.join(format!("{name}.{SHORTCUT_EXT}"));
    macos::create_app_wrapper(&dest, &name, &body, Some(std::path::Path::new(&app)))?;
    if matches!(target, ShortcutTarget::StartMenu) {
        macos::reveal_in_finder(&dest);
    }
    Ok(dest.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
pub fn create_shortcut(
    p: &AppProfile,
    target: ShortcutTarget,
) -> Result<String, String> {
    if p.id == crate::claude_desktop::PROFILE_ID {
        let installation = crate::claude_desktop::detect_fresh()?
            .ok_or_else(|| "Claude Desktop не найден — сначала установите приложение".to_string())?;
        let launcher = crate::claude_desktop::install_launcher()?;
        let system32 = std::env::var("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32"))
            .map_err(|_| "Не найдена папка Windows".to_string())?;
        // conhost --headless: когда консоль по умолчанию — Windows Terminal,
        // -WindowStyle Hidden не прячет его вкладку; headless-conhost окна не создаёт.
        let conhost = system32.join("conhost.exe");
        let powershell = system32.join("WindowsPowerShell/v1.0/powershell.exe");
        let arguments = format!(
            "--headless \"{}\" -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\" -Port {} -ShowError",
            powershell.display(), launcher.display(), crate::claude_desktop::PROXY_PORT
        );
        return run_create_shortcut_ps(
            &format!("{} (proxy)", p.name),
            &conhost.to_string_lossy(),
            &arguments,
            &installation.exe_path,
            target,
        );
    }
    let (installed, location) = profile_installed(p);
    if !installed {
        return Err(format!("{} не найден — сначала установите приложение", p.name));
    }
    let proxy = launch_proxy_url(p);

    // Куда указывает ярлык + аргументы.
    let (target_path, arguments) = match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => {
            let exe = location.ok_or_else(|| "Нет пути к exe".to_string())?;
            let args = if rejects_proxy_switch(&exe) {
                String::new()
            } else {
                format!("--proxy-server={proxy} {WEBRTC_GUARD}")
            };
            (exe, args)
        }
        LaunchKind::Msix => {
            // Ярлык на ПРЯМОЙ exe пакета с флагом --proxy-server (как в launch)
            // — только так Codex реально проксируется. shell:AppsFolder аргумент
            // глотает. Путь версионированный, но ярлык одноразовый; при апдейте
            // Store пользователь пересоздаёт (или запускает через «Открыть»).
            let exe = msix_main_exe(p)
                .ok_or_else(|| "Не нашёл exe пакета для ярлыка".to_string())?;
            (
                exe.to_string_lossy().to_string(),
                format!("--proxy-server={proxy} {WEBRTC_GUARD}"),
            )
        }
    };

    let name = format!("{} (proxy)", p.name);
    let out = run_create_shortcut_ps(&name, &target_path, &arguments, &target_path, target)?;
    Ok(out)
}

#[cfg(target_os = "windows")]
fn run_create_shortcut_ps(
    name: &str,
    target_path: &str,
    arguments: &str,
    icon_path: &str,
    target: ShortcutTarget,
) -> Result<String, String> {
    // Экранируем одинарные кавычки для PS single-quoted строк.
    let esc = |s: &str| s.replace('\'', "''");
    let name_e = esc(name);
    let target_e = esc(target_path);
    let args_e = esc(arguments);
    let icon_e = esc(icon_path);

    // Desktop: .lnk в папку рабочего стола.
    // StartMenu: .lnk в пользовательскую папку Programs меню Пуск + открываем
    //   папку, чтобы пользователь мог правым кликом «Закрепить на панели задач»
    //   (на Win11 это единственный рабочий путь — программный pin удалён).
    let script = match target {
        ShortcutTarget::Desktop => format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $desktop = [Environment]::GetFolderPath('Desktop'); \
             $path = Join-Path $desktop '{name}.lnk'; \
             $lnk = $ws.CreateShortcut($path); \
             $lnk.TargetPath = '{target}'; \
             $lnk.Arguments = '{args}'; \
             $lnk.IconLocation = '{icon}'; \
             $lnk.Save(); \
             Write-Output ('SHORTCUT_OK:' + $path)",
            name = name_e,
            target = target_e,
            args = args_e,
            icon = icon_e,
        ),
        ShortcutTarget::StartMenu => format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $dir = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs'; \
             if (-not (Test-Path $dir)) {{ New-Item -ItemType Directory -Path $dir -Force | Out-Null }}; \
             $path = Join-Path $dir '{name}.lnk'; \
             $lnk = $ws.CreateShortcut($path); \
             $lnk.TargetPath = '{target}'; \
             $lnk.Arguments = '{args}'; \
             $lnk.IconLocation = '{icon}'; \
             $lnk.Save(); \
             Start-Process explorer.exe -ArgumentList ('/select,' + '\"' + $path + '\"'); \
             Write-Output ('SHORTCUT_OK:' + $path)",
            name = name_e,
            target = target_e,
            args = args_e,
            icon = icon_e,
        ),
    };

    // Preserve Unicode desktop paths and stop before reporting success if COM
    // or Save fails (the UI must not claim that a shortcut was created).
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $ErrorActionPreference='Stop'; {script}"
    );
    let out = run_silent(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .ok_or_else(|| "powershell.exe не выполнил создание ярлыка".to_string())?;

    if let Some(path) = out.lines().find_map(|l| l.strip_prefix("SHORTCUT_OK:")) {
        Ok(path.trim().to_string())
    } else {
        Err(format!("Не удалось создать ярлык: {out}"))
    }
}

// ─── Резолв процессов в пути (для firewall-killswitch) ───────────

/// Разрешить `process_names` включённых профилей в конкретные полные пути exe
/// для правил файрвола. Windows Firewall принимает только путь, не имя.
///
/// Логика по имени:
/// - `Code.exe` → путь VS Code (детект).
/// - `ChatGPT.exe` / `Codex.exe` / `codex-*.exe` → внутри версионированной
///   папки MSIX-пакета Codex (`WindowsApps\OpenAI.Codex_<ver>_...\app\...`).
///   Путь пересобираем КАЖДЫЙ раз → апдейт Store не ломает правило.
/// - `node.exe` / `git.exe` / `python.exe` → системные, ищем через `where`
///   + добавляем node/codex-рантаймы внутри MSIX Codex если он включён.
///
/// Возвращает уникальный список существующих путей.
pub fn resolve_process_paths(enabled_profiles: &[AppProfile]) -> Vec<String> {
    // macOS: killswitch недоступен (firewall_stub.rs) — путей не ищем.
    if cfg!(not(target_os = "windows")) {
        return Vec::new();
    }
    let mut paths: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if !p.is_empty() && !paths.contains(&p) && PathBuf::from(&p).exists() {
            paths.push(p);
        }
    };

    let codex_enabled = enabled_profiles.iter().any(|p| p.id == "codex" && p.enabled);
    let codex_root = if codex_enabled { codex_install_root() } else { None };

    for prof in enabled_profiles.iter().filter(|p| p.enabled) {
        if prof.id == crate::claude_desktop::PROFILE_ID {
            if let Ok(Some(installation)) = crate::claude_desktop::detect_fresh() {
                push(installation.exe_path);
            }
            continue;
        }
        for name in &prof.process_names {
            let lname = name.to_lowercase();
            match lname.as_str() {
                "code.exe" => {
                    if let Some(p) = find_vscode_exe() {
                        push(p.to_string_lossy().to_string());
                    }
                }
                "chatgpt.exe" | "codex.exe" | "codex-command-runner.exe"
                | "codex-code-mode-host.exe" => {
                    if let Some(root) = &codex_root {
                        for found in find_in_dir(root, name) {
                            push(found);
                        }
                    }
                }
                "node.exe" => {
                    // Системный node (для VS Code / Claude Code) + node внутри Codex.
                    for p in which_all("node.exe") {
                        push(p);
                    }
                    if let Some(root) = &codex_root {
                        for found in find_in_dir(root, "node.exe") {
                            push(found);
                        }
                    }
                }
                "git.exe" => {
                    for p in which_all("git.exe") {
                        push(p);
                    }
                }
                "python.exe" => {
                    for p in which_all("python.exe") {
                        push(p);
                    }
                }
                _ => {
                    // Кастомный профиль: если у него есть exe_path — блокируем его.
                    if let Some(ep) = &prof.exe_path {
                        push(ep.clone());
                    }
                }
            }
        }
    }
    paths
}

/// Корень установленного MSIX-пакета Codex (версионированный путь).
#[cfg(target_os = "windows")]
fn codex_install_root() -> Option<PathBuf> {
    crate::win::store_packages("OpenAI.Codex")
        .into_iter()
        .map(|p| p.root)
        .find(|root| root.exists())
}

#[cfg(not(target_os = "windows"))]
fn codex_install_root() -> Option<PathBuf> {
    None
}

/// Найти все файлы с именем `name` рекурсивно в `dir` (для MSIX-рантаймов).
/// Ограничиваем глубину и кол-во — WindowsApps может быть большой.
fn find_in_dir(dir: &std::path::Path, name: &str) -> Vec<String> {
    let out = run_silent(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Get-ChildItem -LiteralPath '{}' -Filter '{}' -Recurse -ErrorAction SilentlyContinue -File | Select-Object -First 10 -ExpandProperty FullName",
                dir.to_string_lossy().replace('\'', "''"),
                name.replace('\'', "''"),
            ),
        ],
    );
    match out {
        Some(s) => s
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// Все пути exe по имени (может вернуть несколько). Через PowerShell
/// Get-Command с UTF-8-прелюдией: `where.exe` пишет в пайп OEM-866, и пути
/// с кириллицей (профиль «Пётр») превращались в кашу → killswitch молча
/// пропускал эти exe.
fn which_all(name: &str) -> Vec<String> {
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         (Get-Command '{}' -All -ErrorAction SilentlyContinue -CommandType Application).Source",
        name.replace('\'', "''"),
    );
    let out = run_silent(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    );
    match out {
        Some(s) => s
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && l.to_lowercase().ends_with(".exe"))
            .collect(),
        None => Vec::new(),
    }
}

// ─── helpers ─────────────────────────────────────────────────────

fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cmd;
    }
}

fn run_silent(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    apply_no_window(&mut cmd);
    let out = cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() && !out.status.success() {
        None
    } else {
        Some(s)
    }
}

// ─── Поиск установленных приложений ─────────────────────────────

/// Найденное приложение для «Добавить»: имя как в Finder / меню «Пуск» и
/// путь к бандлу / exe.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct FoundApp {
    pub name: String,
    pub path: String,
    /// Иконка (data-URL PNG) сразу с ответом — чтобы подсказка не
    /// «допрыгивала» иконкой следом (Windows; на macOS — отдельным запросом).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Кириллица → латиница: «телег» находит Telegram, «хром» — Chrome.
fn translit(q: &str) -> String {
    const MAP: &[(char, &str)] = &[
        ('а', "a"), ('б', "b"), ('в', "v"), ('г', "g"), ('д', "d"), ('е', "e"), ('ё', "e"),
        ('ж', "zh"), ('з', "z"), ('и', "i"), ('й', "y"), ('к', "k"), ('л', "l"), ('м', "m"),
        ('н', "n"), ('о', "o"), ('п', "p"), ('р', "r"), ('с', "s"), ('т', "t"), ('у', "u"),
        ('ф', "f"), ('х', "h"), ('ц', "c"), ('ч', "ch"), ('ш', "sh"), ('щ', "sch"), ('ъ', ""),
        ('ы', "y"), ('ь', ""), ('э', "e"), ('ю', "yu"), ('я', "ya"),
    ];
    q.to_lowercase()
        .chars()
        .map(|c| MAP.iter().find(|(k, _)| *k == c).map(|(_, v)| v.to_string()).unwrap_or_else(|| c.to_string()))
        .collect()
}

/// Чем выше — тем лучше совпадение: начало имени > начало слова > внутри.
fn match_score(name: &str, needles: &[String]) -> Option<u32> {
    let n = name.to_lowercase();
    needles
        .iter()
        .filter(|q| !q.is_empty())
        .filter_map(|q| {
            if n.starts_with(q.as_str()) {
                Some(300)
            } else if n.split([' ', '-', '_', '.']).any(|w| w.starts_with(q.as_str())) {
                Some(200)
            } else if n.contains(q.as_str()) {
                Some(100)
            } else if q.chars().count() >= 2
                && n
                    .split([' ', '-', '_', '.'])
                    .filter_map(|w| w.chars().next())
                    .collect::<String>()
                    .starts_with(q.as_str())
            {
                // Первые буквы слов: «vsc» → Visual Studio Code.
                Some(150)
            } else {
                None
            }
        })
        .max()
}

/// Отбор и сортировка ответа Spotlight (`путь   kMDItemDisplayName = Имя`).
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn rank_found(mdfind_out: &str, query: &str, home: &str) -> Vec<FoundApp> {
    let needles = vec![query.trim().to_lowercase(), translit(query.trim())];
    let mut found: Vec<(u32, FoundApp)> = Vec::new();
    for line in mdfind_out.lines() {
        let (path, name) = match line.split_once("   kMDItemDisplayName = ") {
            Some((p, n)) => (p.trim(), n.trim().trim_end_matches(".app").to_string()),
            None => continue,
        };
        // Служебные программы внутри других бандлов, системные потроха и
        // резервные копии в ~/Library — не то, что человек ищет.
        let inner = path.trim_end_matches(".app");
        if inner.contains(".app/")
            || path.starts_with("/System/Library")
            || path.starts_with("/Library/")
            || path.starts_with(&format!("{home}/Library"))
            || path.starts_with("/private")
            || path.starts_with("/Volumes")
        {
            continue;
        }
        let file = std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(mut score) = match_score(&name, &needles).max(match_score(&file, &needles)) else {
            continue;
        };
        if path.starts_with("/Applications/") {
            score += 20;
        }
        if found.iter().any(|(_, f)| f.path == path) {
            continue;
        }
        found.push((score, FoundApp { name, path: path.to_string(), icon: None }));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.len().cmp(&b.1.name.len())));
    found.into_iter().take(6).map(|(_, f)| f).collect()
}

/// Поиск среди установленных программ через Spotlight: берём все бандлы
/// приложений разом (с именами как в Finder, в т.ч. «Терминал») и ранжируем
/// сами — так работают и транслит («телег»), и первые буквы слов («vsc»).
#[cfg(not(target_os = "windows"))]
pub fn search_installed(query: &str) -> Vec<FoundApp> {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Vec::new();
    }
    // Список всех программ кэшируется на 30 с: запрос идёт на каждую букву.
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, String)>> = std::sync::Mutex::new(None);
    let cached = CACHE.lock().ok().and_then(|c| {
        c.as_ref()
            .filter(|(t, _)| t.elapsed() < std::time::Duration::from_secs(30))
            .map(|(_, s)| s.clone())
    });
    let out = cached.unwrap_or_else(|| {
        let s = Command::new("/usr/bin/mdfind")
            .args([
                "-attr",
                "kMDItemDisplayName",
                "kMDItemContentType == 'com.apple.application-bundle'",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if let Ok(mut c) = CACHE.lock() {
            *c = Some((std::time::Instant::now(), s.clone()));
        }
        s
    });
    let home = dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_default();
    rank_found(&out, q, &home)
}

/// Windows: ярлыки меню «Пуск» (общие и пользователя) и «Приложения и
/// возможности» — то, что человек видит как установленные программы
/// (Telegram в своей папке без ярлыка в «Пуске» — тоже). Ярлыки разбираем
/// сами (lnk.rs, без PowerShell): ответ на каждую букву мгновенный.
#[cfg(target_os = "windows")]
pub fn search_installed(query: &str) -> Vec<FoundApp> {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Vec::new();
    }
    let mut out: Vec<FoundApp> = Vec::new();
    for (name, exe) in rank_shortcuts(&installed_apps(), q) {
        if out.iter().any(|f| f.path.eq_ignore_ascii_case(&exe)) {
            continue;
        }
        out.push(FoundApp { name, path: exe, icon: None });
        if out.len() == 6 {
            break;
        }
    }
    with_icons(out)
}

/// Windows: иконки — в том же ответе (миллисекунды; потом из кэша).
#[cfg(target_os = "windows")]
fn with_icons(mut found: Vec<FoundApp>) -> Vec<FoundApp> {
    let paths: Vec<String> = found.iter().filter(|f| f.icon.is_none()).map(|f| f.path.clone()).collect();
    let mut icons = icons(&paths);
    for f in found.iter_mut().filter(|f| f.icon.is_none()) {
        f.icon = icons.remove(&f.path);
    }
    found
}

/// macOS: иконки подгружает окно поиска отдельным запросом.
#[cfg(not(target_os = "windows"))]
fn with_icons(found: Vec<FoundApp>) -> Vec<FoundApp> {
    found
}

/// Поиск с учётом убранных из списка: встроенные приложения не удаляются, а
/// прячутся, и находятся всегда — выбор вернёт их на место, даже если
/// программы нет ни в «Пуске», ни в Spotlight. `hidden` — (имя, путь к exe).
pub fn search_with_hidden(query: &str, hidden: &[(String, String)]) -> Vec<FoundApp> {
    let mut found = search_installed(query);
    let q = query.trim();
    if q.chars().count() < 2 {
        return found;
    }
    let needles = vec![q.to_lowercase(), translit(q)];
    for (name, path) in hidden {
        if match_score(name, &needles).is_some() && !found.iter().any(|f| same_path(&f.path, path)) {
            found.insert(0, FoundApp { name: name.clone(), path: path.clone(), icon: None });
        }
    }
    found.truncate(6);
    with_icons(found)
}

/// Программы из ярлыков меню «Пуск» и «Приложений и возможностей»: (имя,
/// exe); имя ярлыка главнее. Кэш 30 с — запрос идёт на каждую букву.
#[cfg(target_os = "windows")]
fn installed_apps() -> Vec<(String, String)> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::Instant, Vec<(String, String)>)>> = Mutex::new(None);
    if let Ok(c) = CACHE.lock() {
        if let Some((t, v)) = c.as_ref() {
            if t.elapsed() < std::time::Duration::from_secs(30) {
                return v.clone();
            }
        }
    }
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let mut shortcuts = Vec::new();
    for root in [std::env::var_os("ProgramData").map(PathBuf::from), dirs::data_dir()]
        .into_iter()
        .flatten()
    {
        collect_shortcuts(&root.join("Microsoft\\Windows\\Start Menu\\Programs"), 0, &mut shortcuts);
    }
    let mut apps: Vec<(String, String)> = shortcuts
        .into_iter()
        .filter(|(name, _)| !is_service_shortcut(name))
        .filter_map(|(name, s)| Some((name, shortcut_app(&s.target, &s.args, &system_root)?)))
        .collect();
    for (name, icon) in crate::win::uninstall_entries() {
        let Some(exe) = icon_exe(&icon).and_then(|e| shortcut_app(&e, "", &system_root)) else {
            continue;
        };
        if !is_service_shortcut(&name) && !apps.iter().any(|(_, known)| known.eq_ignore_ascii_case(&exe)) {
            apps.push((clean_app_name(&name), exe));
        }
    }
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((std::time::Instant::now(), apps.clone()));
    }
    apps
}

#[cfg(target_os = "windows")]
fn collect_shortcuts(dir: &std::path::Path, depth: u32, out: &mut Vec<(String, crate::lnk::Shortcut)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 4 {
                collect_shortcuts(&path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            let parsed = std::fs::read(&path).ok().and_then(|d| crate::lnk::parse(&d, dir));
            if let (Some(stem), Some(shortcut)) = (path.file_stem(), parsed) {
                out.push((stem.to_string_lossy().to_string(), shortcut));
            }
        }
    }
}

/// exe из DisplayIcon «Приложений и возможностей»: «"C:\…\x.exe",0» → C:\…\x.exe.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn icon_exe(display_icon: &str) -> Option<String> {
    let s = display_icon.trim();
    let s = match s.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => s
            .rsplit_once(',')
            .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
            .map_or(s, |(path, _)| path),
    }
    .trim();
    s.to_lowercase().ends_with(".exe").then(|| s.to_string())
}

/// Имя из «Приложений и возможностей» без разрядности и версии:
/// «7-Zip 23.01 (x64)» → «7-Zip»; «Telegram Desktop» — как есть.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn clean_app_name(name: &str) -> String {
    let mut s = name.trim().to_string();
    while s.ends_with(')') {
        match s.rfind('(') {
            Some(i) => s = s[..i].trim_end().to_string(),
            None => break,
        }
    }
    let version = |w: &str| {
        let w = w.trim_start_matches(['v', 'V']);
        w.chars().any(|c| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit() || c == '.')
    };
    let mut words: Vec<&str> = s.split_whitespace().collect();
    while words.len() > 1 && words.last().is_some_and(|w| version(w) || w.eq_ignore_ascii_case("version")) {
        words.pop();
    }
    words.join(" ")
}

/// Служебные ярлыки (удаление, справка, сайт) и наши же «… (proxy)» — не
/// программы, которые стоит добавлять.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_service_shortcut(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with("(proxy)")
        || [
            "uninstall", "удалить", "удаление", "readme", "read me", "help", "справка",
            "documentation", "license", "website", "release notes", "what's new",
        ]
        .iter()
        .any(|w| n.contains(w))
}

/// Подходящие по имени ярлыки — лучшие сначала, не больше 12.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn rank_shortcuts<T: Clone>(items: &[(String, T)], query: &str) -> Vec<(String, T)> {
    let needles = vec![query.trim().to_lowercase(), translit(query.trim())];
    let mut found: Vec<(u32, &(String, T))> = items
        .iter()
        .filter(|(name, _)| !is_service_shortcut(name))
        .filter_map(|s| Some((match_score(&s.0, &needles)?, s)))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1 .0.len().cmp(&b.1 .0.len())));
    found.into_iter().take(12).map(|(_, s)| s.clone()).collect()
}

/// Программа, которую запускает ярлык: существующий .exe, не системная
/// утилита, не деинсталлятор и не версионная папка Store (WindowsApps —
/// путь меняется с каждым обновлением). Squirrel-приложения (Discord,
/// Slack…) запускаются через Update.exe --processStart X.exe — берём X.exe.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn shortcut_app(target: &str, args: &str, system_root: &str) -> Option<String> {
    let path = std::path::Path::new(target);
    let file = path.file_name()?.to_string_lossy().to_lowercase();
    if !file.ends_with(".exe") || file.starts_with("unins") || file.contains("uninstall") {
        return None;
    }
    let lower = target.to_lowercase();
    if lower.starts_with(&format!("{}\\", system_root.trim_end_matches('\\').to_lowercase()))
        || lower.contains("\\windowsapps\\")
    {
        return None;
    }
    let exe = if file == "update.exe" {
        squirrel_app(path, args)?
    } else {
        path.to_path_buf()
    };
    exe.is_file().then(|| exe.to_string_lossy().to_string())
}

/// Squirrel: «Update.exe --processStart X.exe» → постоянная заглушка X.exe
/// рядом с Update.exe (переживает обновления), иначе X.exe в новейшей app-*.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn squirrel_app(update: &std::path::Path, args: &str) -> Option<PathBuf> {
    let mut tokens = args.split_whitespace();
    tokens.find(|t| t.eq_ignore_ascii_case("--processStart"))?;
    let name = tokens.next()?.trim_matches('"');
    let root = update.parent()?;
    let stub = root.join(name);
    if stub.is_file() {
        return Some(stub);
    }
    let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let dir = e.file_name().to_string_lossy().to_string();
            let version = dir.strip_prefix("app-")?;
            let key = version.split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect();
            Some((key, e.path().join(name)))
        })
        .filter(|(_, exe)| exe.is_file())
        .collect();
    versions.sort();
    versions.pop().map(|(_, exe)| exe)
}

#[cfg(test)]
mod proxy_switch_tests {
    use super::rejects_proxy_switch;

    #[test]
    fn gemini_is_launched_without_proxy_flags() {
        let root = r"C:\Users\u\AppData\Local\Google\Gemini";
        for exe in ["Gemini.exe", r"app-1.16.0\Gemini.exe", "GeminiAppLauncher.exe"] {
            assert!(rejects_proxy_switch(&format!(r"{root}\{exe}")), "{exe}");
        }
        assert!(!rejects_proxy_switch(r"C:\Program Files\Google\Chrome\Application\chrome.exe"));
        assert!(!rejects_proxy_switch(r"C:\Users\u\AppData\Local\Google\Gemini\Uninstall Gemini.exe"));
        assert!(!rejects_proxy_switch(r"D:\Apps\GeminiClone\gemini.exe"));
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn same_path_as_the_os_sees_it() {
        assert!(same_path("D:\\Telegram Desktop\\Telegram.exe", "D:\\Telegram Desktop\\Telegram.exe"));
        assert!(!same_path("D:\\Telegram Desktop\\Telegram.exe", "D:\\Telegram Desktop\\Updater.exe"));
        // Путь VS Code в настройках — со смешанными слэшами.
        assert_eq!(
            same_path(
                "C:\\Users\\u\\AppData\\Local\\Programs/Microsoft VS Code/Code.exe",
                "c:\\users\\u\\appdata\\local\\programs\\microsoft vs code\\code.exe"
            ),
            cfg!(windows)
        );
        assert_eq!(same_path("D:\\Мессенджеры\\Telegram.exe", "d:\\мессенджеры\\telegram.exe"), cfg!(windows));
    }

    #[test]
    fn telegram_from_link_handler_command() {
        assert_eq!(
            link_handler_exe("\"D:\\Telegram Desktop\\Telegram.exe\"  -- \"%1\""),
            Some(PathBuf::from("D:\\Telegram Desktop\\Telegram.exe"))
        );
        assert_eq!(
            link_handler_exe("C:\\Apps\\TG\\telegram.EXE -- %1"),
            Some(PathBuf::from("C:\\Apps\\TG\\telegram.EXE"))
        );
        assert_eq!(link_handler_exe(""), None);
        // Обработчиком бывает копия Telegram (второй аккаунт) или другая
        // программа: путь отдаём, но основным Telegram считаем только Telegram.exe.
        let copy = link_handler_exe("\"E:\\Telegram\\Second Acc\\Vtoroy-Acc.exe\"  -- \"%1\"").unwrap();
        assert_eq!(copy, PathBuf::from("E:\\Telegram\\Second Acc\\Vtoroy-Acc.exe"));
        assert!(!is_telegram_exe_name(&copy));
        assert!(is_telegram_exe_name(Path::new("D:\\Telegram Desktop\\Telegram.exe")));
        assert!(is_telegram_exe_name(Path::new("C:\\Apps\\TG\\telegram.EXE")));
    }

    #[test]
    fn apps_and_features_entries() {
        assert_eq!(icon_exe("\"D:\\Telegram Desktop\\Telegram.exe\",0").as_deref(), Some("D:\\Telegram Desktop\\Telegram.exe"));
        assert_eq!(icon_exe("C:\\Program Files\\7-Zip\\7zFM.exe,1").as_deref(), Some("C:\\Program Files\\7-Zip\\7zFM.exe"));
        assert_eq!(icon_exe("C:\\Apps\\a,b\\App.exe").as_deref(), Some("C:\\Apps\\a,b\\App.exe"), "запятая в пути — не номер иконки");
        assert_eq!(icon_exe("C:\\Program Files\\App\\app.ico"), None);
        assert_eq!(icon_exe(""), None);
        assert_eq!(clean_app_name("Telegram Desktop"), "Telegram Desktop");
        assert_eq!(clean_app_name("7-Zip 23.01 (x64)"), "7-Zip");
        assert_eq!(clean_app_name("Notepad++ (64-bit x64)"), "Notepad++");
        assert_eq!(clean_app_name("Mozilla Firefox (x64 ru)"), "Mozilla Firefox");
        assert_eq!(clean_app_name("Telegram Desktop version 5.6.3"), "Telegram Desktop");
        assert_eq!(clean_app_name("Python 3.12.1"), "Python");
        assert_eq!(clean_app_name("Office 365"), "Office", "версия-число в конце уходит");
        assert_eq!(clean_app_name("1Password"), "1Password");
    }

    #[test]
    fn ranks_start_menu_shortcuts() {
        let s = |n: &str| (n.to_string(), format!("C:\\Start\\{n}.lnk"));
        let shortcuts = vec![
            s("Telegram"),
            s("Uninstall Telegram"),
            s("Удалить Telegram"),
            s("Google Chrome"),
            s("Visual Studio Code"),
            s("VS Code (proxy)"),
            s("WebTelegram"),
        ];
        let names = |q: &str| rank_shortcuts(&shortcuts, q).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names("телег"), vec!["Telegram", "WebTelegram"]);
        // Свой ярлык Clod Helper «… (proxy)» — не программа для добавления.
        assert_eq!(names("vs"), vec!["Visual Studio Code"]);
        assert_eq!(names("vsc"), vec!["Visual Studio Code"]);
        assert_eq!(names("хром"), vec!["Google Chrome"], "транслит: hrom внутри chrome");
        assert_eq!(names("chr"), vec!["Google Chrome"]);
    }

    #[test]
    fn shortcut_targets_are_real_programs() {
        let dir = std::env::temp_dir().join(format!("clod-squirrel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app-1.0.9").join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("app-1.0.10")).unwrap();
        std::fs::write(dir.join("Update.exe"), b"").unwrap();
        std::fs::write(dir.join("app-1.0.9").join("Discord.exe"), b"").unwrap();
        std::fs::write(dir.join("app-1.0.10").join("Discord.exe"), b"").unwrap();
        std::fs::write(dir.join("unins000.exe"), b"").unwrap();
        let update = dir.join("Update.exe").to_string_lossy().to_string();
        let root = "C:\\Windows";
        // Новейшая app-* по номеру версии (1.0.10 > 1.0.9), не по алфавиту.
        assert_eq!(
            shortcut_app(&update, "--processStart Discord.exe", root),
            Some(dir.join("app-1.0.10").join("Discord.exe").to_string_lossy().to_string())
        );
        // Постоянная заглушка рядом с Update.exe важнее версионной папки.
        std::fs::write(dir.join("Discord.exe"), b"").unwrap();
        assert_eq!(
            shortcut_app(&update, "--processStart \"Discord.exe\"", root),
            Some(dir.join("Discord.exe").to_string_lossy().to_string())
        );
        assert_eq!(shortcut_app(&update, "", root), None);
        assert_eq!(shortcut_app(&dir.join("unins000.exe").to_string_lossy(), "", root), None);
        assert_eq!(shortcut_app("C:\\Windows\\System32\\cmd.exe", "", root), None);
        assert_eq!(
            shortcut_app("C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.9_x64__2p2\\app\\ChatGPT.exe", "", root),
            None
        );
        assert_eq!(shortcut_app(&dir.join("readme.txt").to_string_lossy(), "", root), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ranks_and_filters_spotlight_results() {
        let out = "/Applications/Telegram.app   kMDItemDisplayName = Telegram\n\
                   /Users/u/Library/Backup/Clod Helper (telegram).app   kMDItemDisplayName = Clod Helper (telegram)\n\
                   /Applications/Foo.app/Contents/Helpers/Telegram Helper.app   kMDItemDisplayName = Telegram Helper\n\
                   /Applications/WebTelegram.app   kMDItemDisplayName = WebTelegram\n\
                   /System/Applications/Utilities/Terminal.app   kMDItemDisplayName = Терминал\n";
        let r = rank_found(out, "телег", "/Users/u");
        let names: Vec<&str> = r.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["Telegram", "WebTelegram"]);
        let r = rank_found(out, "терм", "/Users/u");
        assert_eq!(r[0].name, "Терминал");
        let vs = "/Applications/Visual Studio Code.app   kMDItemDisplayName = Visual Studio Code\n";
        assert_eq!(rank_found(vs, "vsc", "/Users/u")[0].name, "Visual Studio Code");
        assert_eq!(translit("хром"), "hrom");
    }

    /// Живой поиск на этой машине: cargo test live_search -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_search() {
        for q in ["телег", "vs", "vsc", "терм", "chat", "хром", "ватс"] {
            println!("{q} → {:?}", search_installed(q).iter().map(|f| (&f.name, &f.path)).collect::<Vec<_>>());
        }
        let probe = if cfg!(target_os = "windows") {
            search_installed("chrome").first().map(|f| f.path.clone()).unwrap_or_default()
        } else {
            "/System/Applications/Utilities/Terminal.app".to_string()
        };
        let icons = icons(&[probe]);
        println!("icon → {}", icons.values().next().map(|v| &v[..40]).unwrap_or("нет"));
    }
}

// ─── Иконки программ ────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
const ICON_SCRIPT: &str = include_str!("../resources/app-icons.js");

/// Иконки программ как data-URL PNG (для подсказок поиска и строк
/// приложений). Кэш в памяти и на диске: иконка не меняется, пока не
/// обновится программа — ключ включает дату её файла.
pub fn icons(paths: &[String]) -> std::collections::HashMap<String, String> {
    use base64::Engine;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let b64 = base64::engine::general_purpose::STANDARD;

    // Дисковый кэш (…/ru.iimperium.clodhelper/icons): ключ — путь и дата
    // файла программы (Info.plist бандла / сам exe).
    let disk_dir = dirs::cache_dir().map(|d| d.join("ru.iimperium.clodhelper").join(ICON_CACHE_DIR));
    let disk_key = |p: &str| -> Option<PathBuf> {
        let m = std::fs::metadata(icon_stamp(p))
            .and_then(|m| m.modified())
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (p, m).hash(&mut h);
        Some(disk_dir.as_ref()?.join(format!("{:016x}.png", h.finish())))
    };
    if let Ok(mut c) = cache.lock() {
        for p in paths {
            if c.contains_key(p) {
                continue;
            }
            if let Some(bytes) = disk_key(p).and_then(|f| std::fs::read(f).ok()) {
                c.insert(p.clone(), format!("data:image/png;base64,{}", b64.encode(bytes)));
            }
        }
    }

    let missing: Vec<&String> = {
        let c = cache.lock().map(|c| c.clone()).unwrap_or_default();
        paths
            .iter()
            .filter(|p| !c.contains_key(*p) && has_icon(p))
            .take(24)
            .collect()
    };
    if !missing.is_empty() {
        let found = extract_icons(&missing);
        if let Ok(mut c) = cache.lock() {
            for (k, v) in found {
                if let (Some(f), Ok(bytes)) = (disk_key(&k), b64.decode(&v)) {
                    if let Some(dir) = f.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&f, bytes);
                }
                c.insert(k, format!("data:image/png;base64,{v}"));
            }
        }
    }
    let c = cache.lock().map(|c| c.clone()).unwrap_or_default();
    paths
        .iter()
        .filter_map(|p| c.get(p).map(|v| (p.clone(), v.clone())))
        .collect()
}

/// Папка дискового кэша иконок (на Windows — новые 64 px вместо прежних 32).
#[cfg(not(target_os = "windows"))]
const ICON_CACHE_DIR: &str = "icons";
#[cfg(target_os = "windows")]
const ICON_CACHE_DIR: &str = "icons64-v2";

/// Файл, по дате которого видно, что программа обновилась.
#[cfg(not(target_os = "windows"))]
fn icon_stamp(p: &str) -> PathBuf {
    std::path::Path::new(p).join("Contents/Info.plist")
}
#[cfg(target_os = "windows")]
fn icon_stamp(p: &str) -> PathBuf {
    match msix_root(p) {
        Some((root, _)) => root.join("AppxManifest.xml"),
        None => PathBuf::from(p),
    }
}

/// У пути есть иконка программы: бандл .app (macOS) / файл .exe или
/// приложение Store по AppUserModelId «Семейство!Приложение» (Windows).
fn has_icon(p: &str) -> bool {
    #[cfg(target_os = "windows")]
    if msix_root(p).is_some() {
        return true;
    }
    let ext = if cfg!(target_os = "windows") { ".exe" } else { ".app" };
    p.to_ascii_lowercase().ends_with(ext) && std::path::Path::new(p).exists()
}

/// Пакет Store, к которому относится путь: AppUserModelId «Семейство!Id»
/// или exe внутри папки пакета (рядом выше лежит AppxManifest.xml).
/// Возвращает корень пакета и Id приложения (если известен).
#[cfg(target_os = "windows")]
fn msix_root(p: &str) -> Option<(PathBuf, Option<String>)> {
    if let Some((family, app)) = p.split_once('!') {
        if !p.contains(['\\', '/']) {
            return Some((appx_install_location(family)?, Some(app.to_string())));
        }
    }
    let path = std::path::Path::new(p);
    if !p.to_ascii_lowercase().contains("windowsapps") {
        return None;
    }
    path.ancestors()
        .skip(1)
        .take(4)
        .find(|d| d.join("AppxManifest.xml").exists())
        .map(|d| (d.to_path_buf(), None))
}

/// Логотип приложения Store (PNG из пакета): у Claude, Codex и других
/// MSIX иконка в exe — заглушка, настоящая лежит в Assets пакета.
#[cfg(target_os = "windows")]
fn msix_logo(p: &str) -> Option<Vec<u8>> {
    let (root, app) = msix_root(p)?;
    let xml = std::fs::read_to_string(root.join("AppxManifest.xml")).ok()?;
    let block = app
        .as_deref()
        .and_then(|id| {
            xml.split("<Application ")
                .skip(1)
                .find(|b| b.split('>').next().is_some_and(|tag| tag.contains(&format!("Id=\"{id}\""))))
        })
        .or_else(|| xml.split("<Application ").nth(1))
        .unwrap_or(&xml);
    let attr = |name: &str| -> Option<String> {
        let start = block.find(&format!("{name}=\""))? + name.len() + 2;
        Some(block[start..].split('"').next()?.to_string())
    };
    let rel = attr("Square44x44Logo")
        .or_else(|| attr("Square150x150Logo"))
        .or_else(|| Some(xml.split("<Logo>").nth(1)?.split("</Logo>").next()?.trim().to_string()))?;
    best_logo_variant(&root.join(rel.replace('/', "\\")))
}

/// Лучший вариант ресурса MSIX: «targetsize-N_altform-unplated» (без
/// цветной подложки) крупнейший, затем targetsize-N, затем scale-N.
#[cfg(target_os = "windows")]
fn best_logo_variant(base: &std::path::Path) -> Option<Vec<u8>> {
    let dir = base.parent()?;
    let stem = base.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let num = |s: &str, key: &str| -> u32 {
        s.split(key).nth(1).map(|r| r.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    let mut best: Option<(u32, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.ends_with(".png") || !(name == format!("{stem}.png") || name.starts_with(&format!("{stem}."))) {
            continue;
        }
        // Светлая тема (lightunplated) — тёмный глиф, на нашем фоне не виден.
        if name.contains("lightunplated") || name.contains("contrast-") {
            continue;
        }
        let score = if name.contains("targetsize-") {
            // Больше 256 не нужно; без подложки — лучше всего.
            let n = num(&name, "targetsize-").min(256);
            if name.contains("altform-unplated") { 3000 + n } else { 2000 + n }
        } else if name.contains("scale-") {
            1000 + num(&name, "scale-")
        } else {
            1
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, e.path()));
        }
    }
    std::fs::read(best?.1).ok()
}

/// macOS: PNG 64×64 иконок бандлов (JXA, NSWorkspace): путь → base64.
#[cfg(not(target_os = "windows"))]
fn extract_icons(paths: &[&String]) -> std::collections::HashMap<String, String> {
    use std::io::Write;
    let child = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-"])
        .args(paths.iter().map(|p| p.as_str()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Default::default() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(ICON_SCRIPT.as_bytes());
    }
    child
        .wait_with_output()
        .ok()
        .and_then(|out| serde_json::from_slice(&out.stdout).ok())
        .unwrap_or_default()
}

/// Windows: иконка из ресурсов exe, 64×64 в PNG — нативно, миллисекунды.
#[cfg(target_os = "windows")]
fn extract_icons(paths: &[&String]) -> std::collections::HashMap<String, String> {
    use base64::Engine;
    paths
        .iter()
        .filter_map(|p| {
            let png = msix_logo(p).or_else(|| crate::win::exe_icon_png(p, 64))?;
            Some((p.to_string(), base64::engine::general_purpose::STANDARD.encode(png)))
        })
        .collect()
}

#[cfg(test)]
mod store_search_smoke {
    /// Только чтение: убранный ChatGPT Codex (Store) находится поиском.
    #[test]
    #[ignore = "смотрит настоящие настройки и установленные программы"]
    fn hidden_store_app_is_found() {
        let hidden: Vec<(String, String)> = crate::settings::load_with_presets()
            .app_profiles
            .into_iter()
            .filter(|p| p.hidden)
            .filter_map(|p| Some((super::profile_file(&p).or_else(|| super::profile_store_id(&p))?, p)))
            .map(|(path, p)| (p.name, path))
            .collect();
        for q in ["chatg", "codex", "чатгпт"] {
            let found = super::search_with_hidden(q, &hidden);
            eprintln!("{q}: {:?}", found.iter().map(|f| (&f.name, &f.path)).collect::<Vec<_>>());
        }
    }
}
