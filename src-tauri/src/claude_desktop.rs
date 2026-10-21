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
/// macOS: exe_path — путь к бандлу `Claude.app`.
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

/// macOS: тот же лаунчер, что у ярлыка, — через stdin, без временных файлов.
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
/// claude-desktop-launch.ps1, но без PowerShell (Get-AppxPackage — секунды
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
    // Только известные папки установщика — Claude Code CLI из PATH не берём.
    let Some(local) = dirs::data_local_dir() else { return Ok(None) };
    for rel in ["AnthropicClaude", "Claude", "Programs\\claude-desktop", "Programs\\Claude", "Programs\\AnthropicClaude"] {
        let root = local.join(rel);
        let direct = root.join("Claude.exe");
        if direct.is_file() {
            return found(direct, None, LaunchKind::ExeFlag);
        }
        // Squirrel оставляет несколько app-* после обновлений — новейшая версия.
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
