//! Claude Desktop discovery and launch share one script with the setup wizard
//! and desktop shortcuts. Store paths are resolved again on every launch.

use std::{
    path::PathBuf,
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::apps::{AppProfile, LaunchKind};

pub const PROFILE_ID: &str = "claude-desktop";
pub const PROXY_PORT: u16 = 8890;
#[cfg(target_os = "windows")]
const LAUNCHER: &str = include_str!("../resources/claude-desktop-launch.ps1");
/// macOS: exe_path - путь к бандлу `Claude.app`.
#[cfg(not(target_os = "windows"))]
const LAUNCHER: &str = include_str!("../resources/claude-desktop-launch.sh");
static INSTALL_CACHE: Mutex<Option<(Instant, Option<Installation>)>> = Mutex::new(None);

#[derive(Debug, Clone, Deserialize)]
pub struct Installation {
    pub exe_path: String,
    pub app_id: Option<String>,
    pub kind: LaunchKind,
}

#[cfg(not(target_os = "windows"))]
const DETECT_ARGS: &str = "--detect-only";

#[cfg(target_os = "windows")]
fn launch_args() -> String {
    format!("-Port {PROXY_PORT}")
}
/// Порт зашит в sh-лаунчер (8890), как и в его проверке /healthz.
#[cfg(not(target_os = "windows"))]
fn launch_args() -> String {
    String::new()
}

/// macOS: тот же лаунчер, что у ярлыка, - через stdin, без временных файлов.
#[cfg(not(target_os = "windows"))]
fn run_script(arguments: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("/bin/sh")
        .arg("-s")
        .arg("--")
        .args(arguments.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось запустить Claude Desktop: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(LAUNCHER.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Не удалось запустить Claude Desktop: {e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() {
            "Не удалось запустить Claude Desktop через прокси".into()
        } else {
            error
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn run_script(arguments: &str) -> Result<String, String> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
    ])
    .arg(format!(
        "& {{\n{}\n}} {arguments}",
        LAUNCHER.trim_start_matches('\u{feff}')
    ));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить Claude Desktop: {e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() {
            "Не удалось запустить Claude Desktop через прокси".into()
        } else {
            error
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_start_matches('\u{feff}')
        .to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn detect_fresh() -> Result<Option<Installation>, String> {
    let json = run_script(DETECT_ARGS)?;
    serde_json::from_str(&json)
        .map_err(|e| format!("Не удалось определить установку Claude Desktop: {e}"))
}

/// Windows: та же логика, что Resolve-ClaudeDesktopInstallation в
/// claude-desktop-launch.ps1, но без PowerShell (Get-AppxPackage - секунды
/// на каждый опрос): пакет Store из реестра пакетов пользователя (новые
/// версии первыми), иначе известные папки установщика.
#[cfg(target_os = "windows")]
pub fn detect_fresh() -> Result<Option<Installation>, String> {
    let found = |path: PathBuf, app_id: Option<String>, kind: LaunchKind| {
        Ok(Some(Installation { exe_path: path.to_string_lossy().to_string(), app_id, kind }))
    };
    // Современный пакет Store; старые сборки MSIX назывались «Claude».
    for name in ["AnthropicPBC.Claude", "Claude"] {
        for package in crate::win::store_packages(name) {
            if let Some((exe, id)) = crate::win::manifest_app(&package.root, "Claude.exe") {
                let path = package.root.join(&exe);
                if path.is_file() {
                    return found(path, Some(format!("{}!{id}", package.family())), LaunchKind::Msix);
                }
            }
            // Повреждённый манифест не должен прятать рабочий exe.
            for rel in ["app\\Claude.exe", "Claude.exe"] {
                let path = package.root.join(rel);
                if path.is_file() {
                    return found(path, None, LaunchKind::Msix);
                }
            }
        }
    }
    // Только известные папки установщика - Claude Code CLI из PATH не берём.
    let Some(local) = dirs::data_local_dir() else { return Ok(None) };
    for rel in ["AnthropicClaude", "Claude", "Programs\\claude-desktop", "Programs\\Claude", "Programs\\AnthropicClaude"] {
        let root = local.join(rel);
        let direct = root.join("Claude.exe");
        if direct.is_file() {
            return found(direct, None, LaunchKind::ExeFlag);
        }
        // Squirrel оставляет несколько app-* после обновлений - новейшая версия.
        let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let dir = e.file_name().to_string_lossy().to_string();
                let version = dir.strip_prefix("app-")?.split('-').next()?.to_string();
                let key = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
                Some((key, e.path().join("Claude.exe")))
            })
            .filter(|(_, exe)| exe.is_file())
            .collect();
        versions.sort();
        if let Some((_, exe)) = versions.pop() {
            return found(exe, None, LaunchKind::ExeFlag);
        }
    }
    Ok(None)
}

/// Проверка без кэша, с записью в кэш: мастер ждёт установку Claude Desktop,
/// и после находки остальной код не должен минуту видеть старое «нет».
pub fn detect_now() -> Option<Installation> {
    let installation = detect_fresh().ok().flatten();
    if let Ok(mut guard) = INSTALL_CACHE.lock() {
        *guard = Some((Instant::now(), installation.clone()));
    }
    installation
}

pub fn detect() -> Option<Installation> {
    let mut guard = INSTALL_CACHE.lock().ok()?;
    if let Some((time, installation)) = guard.as_ref() {
        if time.elapsed() < Duration::from_secs(60)
            && installation
                .as_ref()
                .map_or(true, |i| PathBuf::from(&i.exe_path).exists())
        {
            return installation.clone();
        }
    }
    let installation = detect_fresh().ok().flatten();
    *guard = Some((Instant::now(), installation.clone()));
    installation
}

pub fn preset() -> AppProfile {
    let installation = detect();
    AppProfile {
        id: PROFILE_ID.into(),
        name: "Claude Desktop".into(),
        kind: installation
            .as_ref()
            .map(|i| i.kind.clone())
            .unwrap_or(if cfg!(target_os = "windows") {
                LaunchKind::Msix
            } else {
                LaunchKind::ExeFlag
            }),
        exe_path: installation.as_ref().map(|i| i.exe_path.clone()),
        app_id: installation.and_then(|i| i.app_id),
        process_names: vec![if cfg!(target_os = "windows") { "Claude.exe" } else { "Claude" }.into()],
        enabled: false,
        builtin: true,
        hidden: false,
        proxy: None,
        full_proxy: None,
    }
}

pub fn launch() -> Result<(), String> {
    // The launcher checks the bridge and rejects existing Desktop processes:
    // Electron's single-instance handoff would otherwise ignore the proxy flag.
    run_script(&launch_args()).map(|_| ())
}

/// Claude Desktop открыт мимо прокси: не ошибка, а предложение. Интерфейс
/// узнаёт его по точному тексту и показывает кнопку «Перезапустить», в
/// отчёты об ошибках оно не уходит.
pub const RESTART_OFFER: &str = "Claude Desktop открыт без прокси. Перезапустить его с прокси?";

/// Лаунчер отказал, потому что Claude Desktop уже открыт (его увидел
/// скрипт, а детект - нет: например, гонка с запуском).
pub fn is_already_running_error(e: &str) -> bool {
    e.contains("Claude Desktop уже запущен")
}

/// Процесс по этому пути - та же установка Claude Desktop, как в
/// Test-ClaudeDesktopRunning лаунчера: тот же exe, прежняя версия пакета
/// Store (обновление из Store оставляет старую версию открытой) или
/// соседняя папка app-* установщика.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn same_install(installed: &str, msix: bool, process: &str) -> bool {
    let (installed, process) = (installed.to_lowercase(), process.to_lowercase());
    if installed == process {
        return true;
    }
    let parts: Vec<&str> = process.split('\\').collect();
    let n = parts.len();
    if msix && n >= 3 && parts[n - 1] == "claude.exe" {
        let pkg = if parts[n - 2] == "app" { parts[n - 3] } else { parts[n - 2] };
        if pkg.starts_with("anthropicpbc.claude_") || pkg.starts_with("claude_") {
            return true;
        }
    }
    // Папка exe и всё, что выше неё.
    let split = |p: &str| -> Option<(String, String)> {
        let (dir, _) = p.rsplit_once('\\')?;
        let (root, leaf) = dir.rsplit_once('\\')?;
        Some((leaf.to_string(), root.to_string()))
    };
    match (split(&installed), split(&process)) {
        (Some((a, ra)), Some((b, rb))) => a.starts_with("app-") && b.starts_with("app-") && ra == rb,
        _ => false,
    }
}

/// PID открытого Claude Desktop этой установки (claude.exe Claude Code в
/// терминалах не считается: у него другой путь).
#[cfg(target_os = "windows")]
pub fn running_pids() -> Vec<u32> {
    let Some(i) = detect() else { return Vec::new() };
    let msix = matches!(i.kind, LaunchKind::Msix);
    crate::apps::pids_where("claude.exe", |path| same_install(&i.exe_path, msix, path))
}

/// Открыт ли Claude Desktop этой установки, в том числе прежняя версия.
pub fn running() -> bool {
    #[cfg(target_os = "windows")]
    {
        !running_pids().is_empty()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let Some(i) = detect() else { return false };
        let app = i.exe_path.trim_end_matches('/');
        Command::new("/usr/bin/pgrep")
            .args(["-f", &format!("{app}/Contents/MacOS/Claude")])
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

/// Вывести уже открытый Claude Desktop вперёд. Повторный запуск Electron
/// отдаёт открытому окну и сразу выходит, прокси при этом не меняется.
pub fn focus() -> Result<(), String> {
    let i = detect().ok_or_else(|| "Claude Desktop не найден".to_string())?;
    #[cfg(target_os = "windows")]
    let mut cmd = match &i.app_id {
        Some(id) => {
            let mut c = Command::new("explorer.exe");
            c.arg(format!("shell:AppsFolder\\{id}"));
            c
        }
        None => Command::new(&i.exe_path),
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("/usr/bin/open");
        c.arg(&i.exe_path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Не удалось открыть Claude Desktop: {e}"))
}

/// Install a stable launcher outside WindowsApps. A .lnk never pins a Store
/// package version; the script also works when the execution alias is disabled.
pub fn install_launcher() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| "Не найдена папка LOCALAPPDATA".to_string())?
        .join("ClaudeDesktopProxy");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать папку запуска Claude: {e}"))?;
    #[cfg(target_os = "windows")]
    let (path, text) = (
        dir.join("claude-desktop-launch.ps1"),
        // Windows PowerShell 5.1 needs a BOM to read Russian messages correctly.
        format!("\u{feff}{}", LAUNCHER.trim_start_matches('\u{feff}')),
    );
    #[cfg(not(target_os = "windows"))]
    let (path, text) = (dir.join("claude-desktop-launch.sh"), LAUNCHER.to_string());
    std::fs::write(&path, text).map_err(|e| format!("Не удалось сохранить запуск Claude: {e}"))?;
    #[cfg(not(target_os = "windows"))]
    crate::macos::set_mode(&path, 0o755);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_install_matches_launcher_rules() {
        let store = r"C:\Program Files\WindowsApps\AnthropicPBC.Claude_1.2.0.0_x64__fnn82j28hfe8t\app\Claude.exe";
        let old = r"C:\Program Files\WindowsApps\AnthropicPBC.Claude_1.1.0.0_x64__fnn82j28hfe8t\app\claude.exe";
        assert!(same_install(store, true, store));
        assert!(same_install(store, true, old), "прежняя версия пакета Store");
        assert!(!same_install(store, true, r"C:\Users\u\.local\bin\claude.exe"), "Claude Code в терминале");
        let sq = r"C:\Users\u\AppData\Local\AnthropicClaude\app-0.9.3\claude.exe";
        assert!(same_install(sq, false, r"C:\Users\u\AppData\Local\AnthropicClaude\app-0.9.2\Claude.exe"));
        assert!(!same_install(sq, false, r"C:\Users\u\AppData\Local\Other\app-1\claude.exe"));
        assert!(!same_install(sq, false, old), "у установщика пакет Store не считается");
        assert!(is_already_running_error("Claude Desktop уже запущен. Полностью закройте его"));
        assert!(!is_already_running_error(RESTART_OFFER));
    }

    #[test]
    fn accepts_store_manifest_identity_and_legacy_installation() {
        let store: Installation = serde_json::from_str(r#"{"exe_path":"C:\\Program Files\\WindowsApps\\AnthropicPBC.Claude_2\\app\\claude.exe","app_id":"AnthropicPBC.Claude_fnn82j28hfe8t!Claude","kind":"msix"}"#).unwrap();
        assert_eq!(store.kind, LaunchKind::Msix);
        assert!(store.app_id.unwrap().ends_with("!Claude"));
        let legacy: Installation = serde_json::from_str(r#"{"exe_path":"C:\\Users\\Иван\\AppData\\Local\\AnthropicClaude\\claude.exe","app_id":null,"kind":"exe_flag"}"#).unwrap();
        assert_eq!(legacy.kind, LaunchKind::ExeFlag);
        assert!(legacy.app_id.is_none());
        assert!(serde_json::from_str::<Option<Installation>>("null")
            .unwrap()
            .is_none());
    }

    #[test]
    #[ignore = "read-only smoke test requiring a locally installed Claude Desktop"]
    fn detects_installed_desktop() {
        let installation = detect_fresh()
            .unwrap()
            .expect("Claude Desktop must be installed");
        assert!(PathBuf::from(&installation.exe_path).is_file());
        assert!(installation.exe_path.to_lowercase().ends_with("claude.exe"));
        if installation.kind == LaunchKind::Msix {
            assert!(installation.app_id.unwrap().contains("!Claude"));
        }
    }
}
