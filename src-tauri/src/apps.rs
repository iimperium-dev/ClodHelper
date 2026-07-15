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
}

/// Runtime-статус профиля для UI — найдено ли приложение на этом ПК.
#[derive(Debug, Clone, Serialize)]
pub struct AppProfileStatus {
    pub id: String,
    pub name: String,
    pub kind: LaunchKind,
    pub enabled: bool,
    pub builtin: bool,
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
pub fn builtin_presets() -> Vec<AppProfile> {
    vec![
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
        },
    ]
}

// ─── Детект приложений ───────────────────────────────────────────

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

/// Найти AppUserModelID установленного Codex через PowerShell Get-AppxPackage.
/// Возвращаем в форме `<PackageFamilyName>!App` — этим запускается MSIX.
pub fn find_codex_app_id() -> Option<String> {
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
    match p.kind {
        LaunchKind::ExeFlag | LaunchKind::Custom => {
            // Пресет мог быть сохранён без пути (не найден при установке) —
            // перепроверяем детектом на случай если приложение поставили позже.
            let path = p
                .exe_path
                .clone()
                .filter(|s| PathBuf::from(s).exists())
                .or_else(|| {
                    if p.id == "vscode" {
                        find_vscode_exe().map(|x| x.to_string_lossy().to_string())
                    } else {
                        None
                    }
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

fn desktop_shortcut_path(name: &str) -> Option<PathBuf> {
    let userprofile = std::env::var("USERPROFILE").ok()?;
    Some(
        PathBuf::from(userprofile)
            .join("Desktop")
            .join(format!("{name} (proxy).lnk")),
    )
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
                installed,
                location,
                desktop_shortcut,
                process_names: p.process_names.clone(),
            }
        })
        .collect()
}

// ─── Запуск ──────────────────────────────────────────────────────

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
pub fn launch(p: &AppProfile) -> Result<(), String> {
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
    let proxy = local_proxy_url();

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
pub fn create_shortcut(
    p: &AppProfile,
    target: ShortcutTarget,
) -> Result<String, String> {
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
    let out = run_create_shortcut_ps(&name, &target_path, &arguments, target)?;
    Ok(out)
}

fn run_create_shortcut_ps(
    name: &str,
    target_path: &str,
    arguments: &str,
    target: ShortcutTarget,
) -> Result<String, String> {
    // Экранируем одинарные кавычки для PS single-quoted строк.
    let esc = |s: &str| s.replace('\'', "''");
    let name_e = esc(name);
    let target_e = esc(target_path);
    let args_e = esc(arguments);

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
            icon = target_e,
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
            icon = target_e,
        ),
    };

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
    let mut paths: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if !p.is_empty() && !paths.contains(&p) && PathBuf::from(&p).exists() {
            paths.push(p);
        }
    };

    let codex_enabled = enabled_profiles.iter().any(|p| p.id == "codex" && p.enabled);
    let codex_root = if codex_enabled { codex_install_root() } else { None };

    for prof in enabled_profiles.iter().filter(|p| p.enabled) {
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

/// Все пути exe по имени через `where` (может вернуть несколько).
fn which_all(name: &str) -> Vec<String> {
    let out = run_silent("where", &[name]);
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
