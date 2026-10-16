//! Claude's native fixed proxy policy. This routes cooperating clients; it is
//! not a network boundary or a claim that all Claude traffic is protected.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const PROXY_URL: &str = "http://127.0.0.1:8890";
const POLICY_SCRIPT: &str = include_str!("../resources/claude-proxy-policy.ps1");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaudePolicyStatus {
    /// The registry policy matches Helper; this does not observe a running VM.
    pub configured: bool,
    /// Helper has the original-value backup and owns this policy operation.
    pub managed: bool,
    pub conflict: Option<String>,
    /// Unknown policy URLs are not returned, since they can contain credentials.
    pub proxy_url: Option<String>,
    pub pac_configured: bool,
    pub notice: Option<String>,
}

#[derive(Deserialize)]
struct PolicyResult {
    ok: bool,
    status: Option<ClaudePolicyStatus>,
    error: Option<String>,
}

/// Reads the 64-bit machine policy and ownership record, without elevation.
pub fn status() -> Result<ClaudePolicyStatus, String> {
    let mut command = powershell();
    command.arg("-Command").arg(format!(
        "& {{\n{}\n}} -Action Status",
        POLICY_SCRIPT.trim_start_matches('\u{feff}')
    ));
    let output = command
        .output()
        .map_err(|e| format!("Не удалось прочитать политику Claude: {e}"))?;
    let payload = String::from_utf8_lossy(&output.stdout);
    if payload.trim().is_empty() {
        return Err(format!(
            "Не удалось прочитать политику Claude: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let result = parse_result(&payload)?;
    if !output.status.success() {
        return Err("Проверка политики Claude завершилась с ошибкой".into());
    }
    Ok(result)
}

/// Explicit user action only: this can present a Windows UAC prompt.
/// Refuses an existing foreign fixed proxy or PAC policy before requesting UAC.
pub fn enable() -> Result<ClaudePolicyStatus, String> {
    let current = status()?;
    if let Some(conflict) = current.conflict.as_ref() {
        return Err(format!("Политика Claude: {conflict}"));
    }
    if current.configured && current.managed {
        return Ok(current);
    }
    mutate("Enable")
}

/// Restores only a value that still equals ours; an external edit is preserved.
/// Explicit user action only: this can present a Windows UAC prompt.
pub fn disable() -> Result<ClaudePolicyStatus, String> {
    let current = status()?;
    if !current.managed {
        return Ok(current);
    }
    mutate("Disable")
}

fn mutate(action: &str) -> Result<ClaudePolicyStatus, String> {
    let files = OperationFiles::create()?;
    // Windows PowerShell 5.1 reads UTF-8 script files reliably with a BOM.
    std::fs::write(
        &files.script,
        format!("\u{feff}{}", POLICY_SCRIPT.trim_start_matches('\u{feff}')),
    )
    .map_err(|e| format!("Не удалось подготовить политику Claude: {e}"))?;
    let wrapper = elevation_wrapper(action, &files.script, &files.result);
    let output = powershell()
        .arg("-Command")
        .arg(wrapper)
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = std::fs::read_to_string(&files.result).ok();
    let applied = parse_elevated_result(output.status.success(), &stdout, result.as_deref())?;
    // Do not trust just an elevated process exit or its result file. Re-read
    // actual HKLM state before reporting success to the UI.
    let mut observed = status()?;
    if action == "Enable"
        && (!observed.configured
            || !observed.managed
            || observed.conflict.is_some()
            || observed.proxy_url.as_deref() != Some(PROXY_URL))
    {
        return Err("Не удалось подтвердить применение политики прокси Claude".into());
    }
    if action == "Disable" && observed.managed {
        return Err("Не удалось подтвердить восстановление политики Claude".into());
    }
    observed.notice = applied.notice;
    Ok(observed)
}

fn parse_result(payload: &str) -> Result<ClaudePolicyStatus, String> {
    let result: PolicyResult = serde_json::from_str(payload.trim().trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Некорректный ответ проверки политики Claude: {e}"))?;
    if !result.ok {
        return Err(format!(
            "Политика Claude: {}",
            result.error.unwrap_or_else(|| "неизвестная ошибка".into())
        ));
    }
    result
        .status
        .ok_or_else(|| "Проверка политики Claude не вернула состояние".into())
}

fn parse_elevated_result(
    successful_exit: bool,
    stdout: &str,
    payload: Option<&str>,
) -> Result<ClaudePolicyStatus, String> {
    if stdout
        .lines()
        .any(|line| line.trim() == "CLOD_POLICY_UAC_CANCELLED")
    {
        return Err("Запрос прав администратора для политики Claude отменён".into());
    }
    // Error details from the operation remain useful even for a nonzero exit.
    if let Some(payload) = payload {
        let status = parse_result(payload)?;
        if successful_exit {
            return Ok(status);
        }
    }
    Err(if successful_exit {
        "Изменение политики Claude не вернуло подтверждение; состояние не подтверждено".into()
    } else {
        "Не удалось выполнить изменение политики Claude с правами администратора".into()
    })
}

fn elevation_wrapper(action: &str, script: &Path, result: &Path) -> String {
    // Windows paths cannot contain a quote. Quote paths inside the command line
    // and escape apostrophes for the surrounding PowerShell literal.
    let arguments = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" -Action {} -ResultPath \"{}\"",
        script.to_string_lossy(),
        action,
        result.to_string_lossy()
    )
    .replace('\'', "''");
    format!(
        "$ErrorActionPreference='Stop'; \
         try {{ \
           $p = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') \
             -Verb RunAs -WindowStyle Hidden -PassThru -Wait -ArgumentList '{arguments}'; \
           exit $p.ExitCode \
         }} catch {{ \
           $e = $_.Exception; \
           while ($null -ne $e) {{ \
             if ($e -is [System.ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq 1223) {{ \
               Write-Output 'CLOD_POLICY_UAC_CANCELLED'; exit 1 \
             }}; \
             $e = $e.InnerException \
           }}; \
           Write-Output 'CLOD_POLICY_ELEVATION_FAILED'; exit 1 \
         }}"
    )
}

fn powershell() -> Command {
    let executable = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let mut command = Command::new(executable);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

struct OperationFiles {
    directory: PathBuf,
    script: PathBuf,
    result: PathBuf,
}

impl OperationFiles {
    fn create() -> Result<Self, String> {
        let parent = dirs::config_dir()
            .ok_or_else(|| "Не найдена папка настроек Helper".to_string())?
            .join("Clod Helper");
        std::fs::create_dir_all(&parent)
            .map_err(|e| format!("Не удалось создать папку операции: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let directory = parent.join(format!("claude-policy-{}-{nonce}", std::process::id()));
        // Never reuse a stale script or result from an earlier operation.
        std::fs::create_dir(&directory)
            .map_err(|e| format!("Не удалось создать папку операции: {e}"))?;
        Ok(Self {
            script: directory.join("policy.ps1"),
            result: directory.join("result.json"),
            directory,
        })
    }
}

impl Drop for OperationFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script);
        let _ = std::fs::remove_file(&self.result);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUCCESS: &str = r#"{"ok":true,"status":{"configured":true,"managed":true,"conflict":null,"proxy_url":"http://127.0.0.1:8890","pac_configured":false,"notice":null},"error":null}"#;

    #[test]
    fn result_requires_valid_status_and_success() {
        assert_eq!(
            parse_result(SUCCESS).unwrap().proxy_url.as_deref(),
            Some(PROXY_URL)
        );
        assert!(parse_result(r#"{"ok":true,"status":null}"#).is_err());
        assert!(parse_result("invalid json").is_err());
        assert!(parse_result(r#"{"ok":false,"error":"PAC policy exists"}"#)
            .unwrap_err()
            .contains("PAC policy exists"));
    }

    #[test]
    fn elevation_denial_missing_result_and_failed_exit_never_report_success() {
        assert!(
            parse_elevated_result(false, "CLOD_POLICY_UAC_CANCELLED\r\n", None)
                .unwrap_err()
                .contains("отменён")
        );
        assert!(parse_elevated_result(true, "", None).is_err());
        assert!(parse_elevated_result(false, "", Some(SUCCESS)).is_err());
        assert!(
            parse_elevated_result(false, "", Some(r#"{"ok":false,"error":"write denied"}"#))
                .unwrap_err()
                .contains("write denied")
        );
        assert!(parse_elevated_result(true, "", Some(SUCCESS)).is_ok());
    }

    #[test]
    fn uac_wrapper_quotes_paths_and_hides_child_window() {
        let wrapper = elevation_wrapper(
            "Enable",
            Path::new("C:\\O'Brien\\policy.ps1"),
            Path::new("C:\\A B\\result.json"),
        );
        assert!(wrapper.contains("-Verb RunAs -WindowStyle Hidden"));
        assert!(wrapper.contains("-File \"C:\\O''Brien\\policy.ps1\""));
        assert!(wrapper.contains("-ResultPath \"C:\\A B\\result.json\""));
        assert!(wrapper.contains("NativeErrorCode -eq 1223"));
    }
}
