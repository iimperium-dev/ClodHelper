//! Claude's native fixed proxy policy. This routes cooperating clients; it is
//! not a network boundary or a claim that all Claude traffic is protected.

#[cfg(not(target_os = "windows"))]
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::regval::RegValue;

pub const PROXY_URL: &str = "http://127.0.0.1:8890";
#[cfg(target_os = "windows")]
const POLICY_SCRIPT: &str = include_str!("../resources/claude-proxy-policy.ps1");
/// macOS: `/Library/Managed Preferences/com.anthropic.claudefordesktop.plist`.
#[cfg(not(target_os = "windows"))]
const POLICY_SCRIPT: &str = include_str!("../resources/claude-proxy-policy.sh");

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
    /// macOS: стоит служба, возвращающая настройку, если система её сотрёт.
    #[serde(default)]
    pub auto_restore: bool,
}

#[derive(Deserialize)]
struct PolicyResult {
    ok: bool,
    status: Option<ClaudePolicyStatus>,
    error: Option<String>,
}

/// Ключ политики и запись владения Helper (HKLM, как в claude-proxy-policy.ps1).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const POLICY_KEY: &str = r"SOFTWARE\Policies\Claude";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const OWNER_KEY: &str = r"SOFTWARE\ClodHelper\ClaudeProxyPolicy";

/// Reads the 64-bit machine policy and ownership record, without elevation —
/// straight from the registry: PowerShell costs seconds on every poll.
#[cfg(target_os = "windows")]
pub fn status() -> Result<ClaudePolicyStatus, String> {
    use crate::win::{hklm, value};
    let root = hklm();
    status_from(
        &value(&root, POLICY_KEY, "egressProxyUrl"),
        &value(&root, POLICY_KEY, "egressProxyPacUrl"),
        &value(&root, OWNER_KEY, "State"),
    )
    .map_err(|e| format!("Политика Claude: {e}"))
}

/// The same rules as Get-ClaudePolicyStatus in claude-proxy-policy.ps1 (the
/// script still does the elevated writes); messages are identical.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn status_from(fixed: &RegValue, pac: &RegValue, owner: &RegValue) -> Result<ClaudePolicyStatus, String> {
    let original = owned_original(owner)?;
    // PAC has precedence; even an empty value is a conflict.
    let pac_active = pac.exists();
    let ours = fixed.is_str(PROXY_URL);
    let conflict = if pac_active {
        Some("У Claude уже задана PAC-политика с приоритетом над фиксированным прокси. Она сохранена без изменений.")
    } else if original.as_ref().is_some_and(|o| !ours && fixed != o) {
        Some("Политика прокси Claude изменена вне Helper. Она сохранена без изменений.")
    } else if original.is_none() && fixed.exists() && !fixed.is_str("") {
        Some("У Claude уже задан фиксированный прокси, который не принадлежит Helper. Он сохранён без изменений.")
    } else {
        None
    };
    Ok(ClaudePolicyStatus {
        configured: ours && !pac_active,
        managed: original.is_some(),
        conflict: conflict.map(String::from),
        proxy_url: ours.then(|| PROXY_URL.to_string()),
        pac_configured: pac_active,
        notice: None,
        auto_restore: false,
    })
}

/// Original value from Helper's ownership record; None — no record. Only an
/// absent or empty original is valid: a damaged record is never trusted.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn owned_original(owner: &RegValue) -> Result<Option<RegValue>, String> {
    let json = match owner {
        RegValue::Absent => return Ok(None),
        RegValue::Other(_) => {
            return Err("Запись владения политикой Claude имеет неподдерживаемый тип реестра.".into())
        }
        RegValue::Str(s) => s,
    };
    let damaged = || "Запись владения политикой Claude повреждена; политика не изменена.".to_string();
    let state: serde_json::Value = serde_json::from_str(json).map_err(|_| damaged())?;
    let original = RegValue::from_backup(&state["original"]).ok_or_else(damaged)?;
    if state["owner"] != "ClodHelper"
        || state["version"].as_i64() != Some(1)
        || state["proxy_url"] != PROXY_URL
        || !(original == RegValue::Absent || original.is_str(""))
    {
        return Err(damaged());
    }
    Ok(Some(original))
}

/// Explicit user action only: this can present a Windows UAC prompt.
/// Refuses an existing foreign fixed proxy or PAC policy before requesting UAC.
pub fn enable() -> Result<ClaudePolicyStatus, String> {
    let current = status()?;
    if let Some(conflict) = current.conflict.as_ref() {
        return Err(format!("Политика Claude: {conflict}"));
    }
    // macOS: уже настроенным без службы автовосстановления повторный Enable
    // её доставит (политика при этом не меняется — значение уже наше).
    if current.configured && current.managed && (current.auto_restore || cfg!(target_os = "windows")) {
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
    let applied = elevate(action)?;
    // Do not trust just an elevated process exit or its result. Re-read the
    // actual policy state (HKLM / Managed Preferences) before reporting success.
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

/// Run the policy operation with administrator rights (one UAC prompt).
/// Enable включает в том же окне UAC и защиту от утечек для Claude (как на
/// Mac): из России Claude напрямую не работает, прямой выход только выдаёт IP.
#[cfg(target_os = "windows")]
fn elevate(action: &str) -> Result<ClaudePolicyStatus, String> {
    let arguments = format!("-Action {action}");
    let guard = (action == "Enable")
        .then(crate::leak_guard::status)
        .and_then(Result::ok)
        .and_then(|st| {
            let list = crate::leak_guard::with_service(&st, "claude", true).ok()?;
            // Уже включена с тем же набором — второй раз правила не трогаем.
            (!st.enabled || st.services != list).then_some(list)
        });
    let outcome = match guard {
        Some(list) => crate::elevate::run(
            "claude-policy",
            &[
                ("operation.ps1", &enable_with_guard(&list)),
                ("policy.ps1", POLICY_SCRIPT),
                ("leak-guard.ps1", crate::leak_guard::SCRIPT),
            ],
            &arguments,
        )?,
        None => crate::elevate::run("claude-policy", &[("policy.ps1", POLICY_SCRIPT)], &arguments)?,
    };
    parse_elevated_result(&outcome)
}

/// Главный скрипт «Подключить»: сначала защита от утечек (молча — её сбой не
/// мешает политике), затем политика: её итог и код выхода — итог операции.
/// `services` уже проверен leak_guard::normalize: только claude/telegram.
#[cfg(target_os = "windows")]
fn enable_with_guard(services: &str) -> String {
    format!(
        "param([string]$Action, [string]$ResultPath)\n\
         try {{ & (Join-Path $PSScriptRoot 'leak-guard.ps1') -Action Enable -Services '{services}' \
         -ResultPath (Join-Path $PSScriptRoot 'guard.json') *> $null }} catch {{ }}\n\
         & (Join-Path $PSScriptRoot 'policy.ps1') -Action $Action -ResultPath $ResultPath\n\
         exit $LASTEXITCODE\n"
    )
}

/// macOS: the shell script reads Managed Preferences without elevation.
#[cfg(not(target_os = "windows"))]
pub fn status() -> Result<ClaudePolicyStatus, String> {
    use std::io::Write;
    let mut child = Command::new("/bin/sh")
        .args(["-s", "--", "Status"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось прочитать политику Claude: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(POLICY_SCRIPT.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Не удалось прочитать политику Claude: {e}"))?;
    parse_result(&String::from_utf8_lossy(&output.stdout))
}

/// macOS: one system password prompt via `do shell script … with
/// administrator privileges`. The script travels as base64 inside the command
/// (no root-executed file a user process could swap) and always exits 0, so
/// its JSON reaches stdout even on failure — osascript drops stdout of a
/// failing command.
#[cfg(not(target_os = "windows"))]
fn elevate(action: &str) -> Result<ClaudePolicyStatus, String> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(POLICY_SCRIPT);
    // Enable кладёт копию скрипта в root-папку — её запускает служба
    // автовосстановления (LaunchDaemon). Пишет root в root-owned каталог:
    // пользовательский процесс подменить копию не может.
    let shell = if action == "Enable" {
        // В том же окне пароля — защита от утечек для Claude (по умолчанию):
        // из России Claude напрямую не работает, прямой выход только выдаёт IP.
        let guard = crate::leak_guard::status()
            .ok()
            .and_then(|st| crate::leak_guard::with_service(&st, "claude", true).ok())
            .map(|list| format!("{}; ", crate::leak_guard::shell_enable(&list)))
            .unwrap_or_default();
        format!(
            "/bin/mkdir -p '{dir}' && /usr/sbin/chown root:wheel '{dir}' && /bin/chmod 755 '{dir}' && \
             /bin/echo {encoded} | /usr/bin/base64 -D > '{dir}/claude-proxy-policy.sh'; \
             {guard}/bin/echo {encoded} | /usr/bin/base64 -D | /bin/sh -s -- {action}; exit 0",
            dir = "/Library/Application Support/ClodHelper",
        )
    } else {
        format!("/bin/echo {encoded} | /usr/bin/base64 -D | /bin/sh -s -- {action}; exit 0")
    };
    let apple = format!(
        "do shell script \"{shell}\" with prompt \"Clod Helper подключает Claude через мост и включает защиту от утечек.\" with administrator privileges"
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("-128") {
        return Err("Запрос прав администратора для политики Claude отменён".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || stdout.trim().is_empty() {
        return Err(format!(
            "Не удалось выполнить изменение политики Claude с правами администратора: {}",
            stderr.trim()
        ));
    }
    parse_result(&stdout)
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

#[cfg(target_os = "windows")]
fn parse_elevated_result(outcome: &crate::elevate::Outcome) -> Result<ClaudePolicyStatus, String> {
    if outcome.cancelled {
        return Err("Запрос прав администратора для политики Claude отменён".into());
    }
    // Error details from the operation remain useful even for a nonzero exit.
    if let Some(payload) = outcome.result.as_deref() {
        let status = parse_result(payload)?;
        if outcome.success {
            return Ok(status);
        }
    }
    Err(if outcome.success {
        "Изменение политики Claude не вернуло подтверждение; состояние не подтверждено".into()
    } else {
        "Не удалось выполнить изменение политики Claude с правами администратора".into()
    })
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

    /// Статус из реестра — те же случаи, что в tests/claude-proxy-policy.Tests.ps1.
    #[test]
    fn registry_status_matches_policy_script_rules() {
        use RegValue::{Absent, Other, Str};
        let s = |v: &str| Str(v.to_string());
        let owner = |original: &str| {
            s(&format!(
                r#"{{"owner":"ClodHelper","version":1,"proxy_url":"{PROXY_URL}","original":{original}}}"#
            ))
        };
        let absent = r#"{"exists":false,"kind":null,"data":null}"#;
        let empty = r#"{"exists":true,"kind":"String","data":""}"#;

        let st = status_from(&Absent, &Absent, &Absent).unwrap();
        assert!(!st.configured && !st.managed && st.conflict.is_none() && st.proxy_url.is_none());

        let st = status_from(&s(PROXY_URL), &Absent, &owner(absent)).unwrap();
        assert!(st.configured && st.managed && st.conflict.is_none());
        assert_eq!(st.proxy_url.as_deref(), Some(PROXY_URL));

        // Наше значение без записи владения: скрипт Windows считает его чужим.
        let st = status_from(&s(PROXY_URL), &Absent, &Absent).unwrap();
        assert!(st.configured && !st.managed && st.conflict.is_some());

        // Чужие прокси и PAC — конфликт; пустая строка — нет.
        assert!(status_from(&s("http://corp:3128"), &Absent, &Absent).unwrap().conflict.unwrap().contains("не принадлежит Helper"));
        assert!(status_from(&Other("DWord".into()), &Absent, &Absent).unwrap().conflict.is_some());
        assert!(status_from(&s(""), &Absent, &Absent).unwrap().conflict.is_none());
        let pac = status_from(&s(PROXY_URL), &s(""), &owner(absent)).unwrap();
        assert!(pac.pac_configured && !pac.configured && pac.conflict.unwrap().contains("PAC"));

        // Изменено вне Helper после включения — конфликт; откат к исходному — нет.
        assert!(status_from(&s("http://other:1"), &Absent, &owner(absent)).unwrap().conflict.unwrap().contains("изменена вне"));
        assert!(status_from(&Absent, &Absent, &owner(absent)).unwrap().conflict.is_none());
        assert!(status_from(&s(""), &Absent, &owner(empty)).unwrap().conflict.is_none());

        // Повреждённая запись владения никогда не принимается.
        for bad in [
            s("not json"),
            s(r#"{"owner":"Someone","version":1,"proxy_url":"http://127.0.0.1:8890","original":{"exists":false}}"#),
            owner(r#"{"exists":true,"kind":"String","data":"http://corp:3128"}"#),
            owner(r#"{"exists":"yes"}"#),
            s(r#"{"owner":"ClodHelper","version":1,"proxy_url":"http://127.0.0.1:8890"}"#),
        ] {
            assert!(status_from(&Absent, &Absent, &bad).unwrap_err().contains("повреждена"));
        }
        assert!(status_from(&Absent, &Absent, &Other("DWord".into())).unwrap_err().contains("неподдерживаемый тип"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn elevation_denial_missing_result_and_failed_exit_never_report_success() {
        use crate::elevate::Outcome;
        let out = |cancelled, success, result: Option<&str>| Outcome {
            cancelled,
            success,
            result: result.map(String::from),
        };
        assert!(parse_elevated_result(&out(true, false, None))
            .unwrap_err()
            .contains("отменён"));
        assert!(parse_elevated_result(&out(false, true, None)).is_err());
        assert!(parse_elevated_result(&out(false, false, Some(SUCCESS))).is_err());
        assert!(
            parse_elevated_result(&out(false, false, Some(r#"{"ok":false,"error":"write denied"}"#)))
                .unwrap_err()
                .contains("write denied")
        );
        assert!(parse_elevated_result(&out(false, true, Some(SUCCESS))).is_ok());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn enable_runs_leak_guard_before_policy_in_one_operation() {
        let op = enable_with_guard("claude,telegram");
        let guard = op.find("leak-guard.ps1").unwrap();
        let policy = op.find("'policy.ps1') -Action $Action").unwrap();
        assert!(guard < policy, "сначала защита, потом политика");
        assert!(op.contains("-Services 'claude,telegram'"));
        // Итог операции — итог политики, а не защиты.
        assert!(op.contains("-ResultPath $ResultPath"));
        assert!(op.trim_end().ends_with("exit $LASTEXITCODE"));
    }
}
