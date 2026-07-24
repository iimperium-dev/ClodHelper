use serde::Serialize;
use std::process::Command;

#[derive(Debug, Serialize, Clone)]
pub struct NodeInfo {
    pub installed: bool,
    pub version: Option<String>,
    pub npm_prefix: Option<String>,
    pub prefix_in_program_files: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct PythonInfo {
    pub installed: bool,
    pub version: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CursorInfo {
    pub installed: bool,
    pub exe_path: Option<String>,
    /// Существует ли уже ярлык «Cursor (proxy)» на рабочем столе.
    pub shortcut_exists: bool,
}

/// Найти Cursor.exe в стандартных местах установки (per-user / system).
pub fn find_cursor_exe() -> Option<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            std::path::PathBuf::from(&local).join("Programs/cursor/Cursor.exe"),
        );
        candidates.push(
            std::path::PathBuf::from(&local).join("Programs/Cursor/Cursor.exe"),
        );
    }
    if let Ok(pf) = std::env::var("PROGRAMFILES") {
        candidates.push(std::path::PathBuf::from(&pf).join("Cursor/Cursor.exe"));
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(std::path::PathBuf::from(&pf86).join("Cursor/Cursor.exe"));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Путь к ярлыку «Cursor (proxy)» на рабочем столе пользователя.
/// Через known-folder (OneDrive-aware), не сырой USERPROFILE\Desktop.
pub fn cursor_shortcut_path() -> Option<std::path::PathBuf> {
    Some(crate::apps::desktop_dir()?.join("Cursor (proxy).lnk"))
}

pub fn detect_cursor() -> CursorInfo {
    let exe = find_cursor_exe();
    let shortcut_exists = cursor_shortcut_path()
        .map(|p| p.exists())
        .unwrap_or(false);
    CursorInfo {
        installed: exe.is_some(),
        exe_path: exe.map(|p| p.to_string_lossy().to_string()),
        shortcut_exists,
    }
}

fn run_silent(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn detect_node() -> NodeInfo {
    let version = run_silent("node", &["--version"]);
    if version.is_none() {
        return NodeInfo {
            installed: false,
            version: None,
            npm_prefix: None,
            prefix_in_program_files: false,
        };
    }
    let prefix = run_silent("npm", &["config", "get", "prefix"]);
    let in_pf = prefix
        .as_ref()
        .map(|p| p.to_lowercase().contains("program files"))
        .unwrap_or(false);
    NodeInfo {
        installed: true,
        version,
        npm_prefix: prefix,
        prefix_in_program_files: in_pf,
    }
}

pub fn detect_python() -> PythonInfo {
    // Windows: prefer `py` launcher, fall back to `python`.
    for cmd in &["py", "python", "python3"] {
        if let Some(ver) = run_silent(cmd, &["--version"]) {
            // MS Store stub returns no output; we'd already have skipped via empty.
            if ver.to_lowercase().starts_with("python") {
                return PythonInfo {
                    installed: true,
                    version: Some(ver),
                    command: Some(cmd.to_string()),
                };
            }
        }
    }
    PythonInfo {
        installed: false,
        version: None,
        command: None,
    }
}

// Примечание: detect_python_cmd удалён — мост стал самостоятельным exe,
// Python для автозапуска больше не нужен (detect_python остался для визарда).
