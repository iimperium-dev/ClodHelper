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

/// Удобный shortcut: только команда (`py` / `python` / `python3`) если есть.
pub fn detect_python_cmd() -> Option<String> {
    detect_python().command
}
