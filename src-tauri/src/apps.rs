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

use std::path::PathBuf;
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

/// Профиль Telegram (только macOS).
pub const TELEGRAM_ID: &str = "telegram";

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
    ]
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

/// Кэш AppUserModelID Codex: Get-AppxPackage стоит СЕКУНДЫ, а статус профилей
/// поллится UI каждые 5с (двумя вызовами: builtin_presets + profile_installed).
/// TTL 60с достаточно — установка/удаление Codex случается редко.
static CODEX_APP_ID_CACHE: std::sync::Mutex<
    Option<(std::time::Instant, Option<String>)>,
> = std::sync::Mutex::new(None);

/// Найти AppUserModelID установленного Codex через PowerShell Get-AppxPackage.
/// Возвращаем в форме `<PackageFamilyName>!App` — этим запускается MSIX.
pub fn find_codex_app_id() -> Option<String> {
    if let Ok(guard) = CODEX_APP_ID_CACHE.lock() {
        if let Some((t, cached)) = guard.as_ref() {
            if t.elapsed() < std::time::Duration::from_secs(60) {
                return cached.clone();
            }
        }
    }
    let result = find_codex_app_id_uncached();
    if let Ok(mut guard) = CODEX_APP_ID_CACHE.lock() {
        *guard = Some((std::time::Instant::now(), result.clone()));
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn find_codex_app_id_uncached() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
fn find_codex_app_id_uncached() -> Option<String> {
    // Известный PackageFamilyName Codex стабилен между версиями (меняется только
    // версия в PackageFullName). Проверяем что пакет установлен через Get-AppxPackage.
    let out = run_silent(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-AppxPackage | Where-Object { $_.Name -eq 'OpenAI.Codex' } | Select-Object -First 1; if ($p) { $p.PackageFamilyName } else { '' }",
        ],
    )?;
    let pfn = out.trim();
    if pfn.is_empty() {
        return None;
    }
    Some(format!("{pfn}!App"))
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
/// - иначе общий умный вход 8889.
fn launch_proxy_url(p: &AppProfile) -> String {
    let pool = crate::settings::effective_proxies(&crate::settings::load()).len();
    match p.proxy {
        // Закреплённые входы мост поднимает пока только на macOS.
        Some(i) if i < pool && cfg!(not(target_os = "windows")) => {
            format!("http://127.0.0.1:{}", crate::tasks::PIN_PORT_BASE + i as u16)
        }
        _ if full_proxy_of(p) => crate::claude_policy::PROXY_URL.to_string(),
        _ => local_proxy_url(),
    }
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
    }
    launch(p)
}

pub fn launch(p: &AppProfile) -> Result<(), String> {
    if p.id == crate::claude_desktop::PROFILE_ID {
        return crate::claude_desktop::launch();
    }
    match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => launch_exe(p),
        LaunchKind::Msix => launch_msix(p),
    }
}

fn launch_exe(p: &AppProfile) -> Result<(), String> {
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

    let mut cmd = Command::new(&exe);
    cmd.arg(format!("--proxy-server={proxy}"));
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

#[cfg(not(target_os = "windows"))]
pub(crate) fn open_app(app: &str, proxy: &str, name: &str) -> Result<(), String> {
    let out = Command::new("/usr/bin/open")
        .arg("-a")
        .arg(app)
        .arg("--args")
        .arg(format!("--proxy-server={proxy}"))
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

/// InstallLocation MSIX-пакета по PackageFamilyName.
fn appx_install_location(family: &str) -> Option<PathBuf> {
    let out = run_silent(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "$p = Get-AppxPackage | Where-Object {{ $_.PackageFamilyName -eq '{}' }} | Select-Object -First 1; if ($p) {{ $p.InstallLocation }} else {{ '' }}",
                family.replace('\'', "''"),
            ),
        ],
    )?;
    let loc = out.trim();
    if loc.is_empty() {
        None
    } else {
        let path = PathBuf::from(loc);
        path.exists().then_some(path)
    }
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
        format!(
            "exec /usr/bin/open -a {} --args {}",
            sh_quote(&app),
            sh_quote(&format!("--proxy-server={}", local_proxy_url()))
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
        let powershell = std::env::var("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .map_err(|_| "Не найдена папка Windows".to_string())?;
        let arguments = format!(
            "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\" -Port {} -ShowError",
            launcher.display(), crate::claude_desktop::PROXY_PORT
        );
        return run_create_shortcut_ps(
            &format!("{} (proxy)", p.name),
            &powershell.to_string_lossy(),
            &arguments,
            &installation.exe_path,
            target,
        );
    }
    let (installed, location) = profile_installed(p);
    if !installed {
        return Err(format!("{} не найден — сначала установите приложение", p.name));
    }
    let proxy = local_proxy_url();

    // Куда указывает ярлык + аргументы.
    let (target_path, arguments) = match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => {
            let exe = location.ok_or_else(|| "Нет пути к exe".to_string())?;
            (exe, format!("--proxy-server={proxy}"))
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
                format!("--proxy-server={proxy}"),
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
fn codex_install_root() -> Option<PathBuf> {
    let out = run_silent(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-AppxPackage | Where-Object { $_.Name -eq 'OpenAI.Codex' } | Select-Object -First 1; if ($p) { $p.InstallLocation } else { '' }",
        ],
    )?;
    let loc = out.trim();
    if loc.is_empty() {
        return None;
    }
    let path = PathBuf::from(loc);
    if path.exists() {
        Some(path)
    } else {
        None
    }
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

// ─── Поиск установленных приложений (macOS) ─────────────────────

/// Найденное приложение для «Добавить»: имя как в Finder и путь к бандлу.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct FoundApp {
    pub name: String,
    pub path: String,
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
        found.push((score, FoundApp { name, path: path.to_string() }));
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
    let out = Command::new("/usr/bin/mdfind")
        .args([
            "-attr",
            "kMDItemDisplayName",
            "kMDItemContentType == 'com.apple.application-bundle'",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let home = dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_default();
    rank_found(&out, q, &home)
}

#[cfg(target_os = "windows")]
pub fn search_installed(_query: &str) -> Vec<FoundApp> {
    Vec::new()
}

#[cfg(test)]
mod search_tests {
    use super::*;

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
    #[cfg(target_os = "macos")]
    fn live_search() {
        for q in ["телег", "vs", "vsc", "терм", "chat", "хром", "ватс"] {
            println!("{q} → {:?}", search_installed(q).iter().map(|f| &f.name).collect::<Vec<_>>());
        }
        let icons = icons(&["/System/Applications/Utilities/Terminal.app".to_string()]);
        println!("icon → {}", icons.values().next().map(|v| &v[..40]).unwrap_or("нет"));
    }
}

// ─── Иконки программ (macOS) ────────────────────────────────────

#[cfg(not(target_os = "windows"))]
const ICON_SCRIPT: &str = include_str!("../resources/app-icons.js");

/// Иконки программ как data-URL PNG 64×64 (для подсказок поиска и строк
/// своих приложений). Кэш в памяти: иконки не меняются за время работы.
#[cfg(not(target_os = "windows"))]
pub fn icons(paths: &[String]) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let missing: Vec<&String> = {
        let c = cache.lock().map(|c| c.clone()).unwrap_or_default();
        paths
            .iter()
            .filter(|p| !c.contains_key(*p) && p.ends_with(".app") && std::path::Path::new(p).exists())
            .take(24)
            .collect()
    };
    if !missing.is_empty() {
        let child = Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-"])
            .args(missing.iter().map(|p| p.as_str()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(ICON_SCRIPT.as_bytes());
            }
            if let Ok(out) = child.wait_with_output() {
                if let Ok(map) =
                    serde_json::from_slice::<HashMap<String, String>>(&out.stdout)
                {
                    if let Ok(mut c) = cache.lock() {
                        for (k, v) in map {
                            c.insert(k, format!("data:image/png;base64,{v}"));
                        }
                    }
                }
            }
        }
    }
    let c = cache.lock().map(|c| c.clone()).unwrap_or_default();
    paths
        .iter()
        .filter_map(|p| c.get(p).map(|v| (p.clone(), v.clone())))
        .collect()
}

#[cfg(target_os = "windows")]
pub fn icons(_paths: &[String]) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}
