//! «Всегда через мост» для Chromium-браузеров (macOS): управляемая политика
//! ProxySettings + WebRtcIPHandling (resources/browser-proxy-policy.sh).
//! Главнее параметров запуска и расширений — браузер идёт через мост, даже
//! открытый из Dock. Запись — одна системная просьба пароля, как у Claude.

use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "windows"))]
const SCRIPT: &str = include_str!("../resources/browser-proxy-policy.sh");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserPolicyStatus {
    pub configured: bool,
    pub managed: bool,
    pub conflict: Option<String>,
    #[serde(default)]
    pub auto_restore: bool,
    #[serde(default)]
    pub server: String,
    pub notice: Option<String>,
}

#[derive(Deserialize)]
struct ScriptResult {
    ok: bool,
    status: Option<BrowserPolicyStatus>,
    error: Option<String>,
}

/// Домен управляемых настроек по имени бандла; None — браузер без политик.
pub fn policy_domain(app_path: &str) -> Option<&'static str> {
    let stem = std::path::Path::new(app_path.trim_end_matches('/')).file_stem()?.to_string_lossy();
    Some(match stem.as_ref() {
        "Google Chrome" => "com.google.Chrome",
        "Brave Browser" => "com.brave.Browser",
        "Microsoft Edge" => "com.microsoft.Edge",
        "Chromium" => "org.chromium.Chromium",
        _ => return None,
    })
}

fn parse(payload: &str) -> Result<BrowserPolicyStatus, String> {
    let r: ScriptResult = serde_json::from_str(payload.trim())
        .map_err(|e| format!("Некорректный ответ политики браузера: {e}"))?;
    if !r.ok {
        return Err(r.error.unwrap_or_else(|| "Политика браузера: ошибка".into()));
    }
    r.status.ok_or_else(|| "Политика браузера: нет состояния".into())
}

#[cfg(not(target_os = "windows"))]
pub fn status(domain: &str) -> Result<BrowserPolicyStatus, String> {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-s", "--", "Status", domain])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Политика браузера: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(SCRIPT.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("Политика браузера: {e}"))?;
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Включить/снять с правами администратора (одно системное окно пароля).
/// Enable кладёт копию скрипта в root-папку — её запускает служба
/// автовосстановления; пользовательский процесс подменить её не может.
#[cfg(not(target_os = "windows"))]
pub fn set(domain: &str, server: Option<&str>) -> Result<BrowserPolicyStatus, String> {
    use base64::Engine;
    if policy_domain_known(domain).is_none() {
        return Err("Неизвестный браузер".into());
    }
    let enc = base64::engine::general_purpose::STANDARD.encode(SCRIPT);
    let shell = match server {
        Some(srv) => {
            if !srv.starts_with("127.0.0.1:") || !srv[10..].chars().all(|c| c.is_ascii_digit()) {
                return Err("Ожидался вход моста 127.0.0.1:порт".into());
            }
            format!(
                "/bin/mkdir -p '{dir}' && /usr/sbin/chown root:wheel '{dir}' && /bin/chmod 755 '{dir}' && \
                 /bin/echo {enc} | /usr/bin/base64 -D > '{dir}/browser-proxy-policy.sh'; \
                 /bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Enable {domain} {srv}; exit 0",
                dir = "/Library/Application Support/ClodHelper",
            )
        }
        None => format!("/bin/echo {enc} | /usr/bin/base64 -D | /bin/sh -s -- Disable {domain}; exit 0"),
    };
    let apple = format!(
        "do shell script \"{shell}\" with prompt \"Clod Helper закрепляет прокси в браузере.\" with administrator privileges"
    );
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()
        .map_err(|e| format!("Не удалось запросить права администратора: {e}"))?;
    if String::from_utf8_lossy(&out.stderr).contains("-128") {
        return Err("Запрос пароля отменён".into());
    }
    let r = parse(&String::from_utf8_lossy(&out.stdout))?;
    // Не верим выводу root-процесса на слово — перечитываем фактическое состояние.
    let real = status(domain)?;
    if server.is_some() && !real.configured {
        return Err("Не удалось подтвердить политику браузера".into());
    }
    Ok(BrowserPolicyStatus { notice: r.notice, ..real })
}

fn policy_domain_known(domain: &str) -> Option<()> {
    matches!(
        domain,
        "com.google.Chrome" | "com.brave.Browser" | "com.microsoft.Edge" | "org.chromium.Chromium"
    )
    .then_some(())
}

#[cfg(target_os = "windows")]
pub fn status(_domain: &str) -> Result<BrowserPolicyStatus, String> {
    Ok(BrowserPolicyStatus::default())
}

#[cfg(target_os = "windows")]
pub fn set(_domain: &str, _server: Option<&str>) -> Result<BrowserPolicyStatus, String> {
    Err("Пока только для macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_and_parse() {
        assert_eq!(policy_domain("/Applications/Google Chrome.app"), Some("com.google.Chrome"));
        assert_eq!(policy_domain("/Applications/Telegram.app"), None);
        let ok = r#"{"ok":true,"status":{"configured":true,"managed":true,"conflict":null,"auto_restore":true,"server":"127.0.0.1:8890","notice":null},"error":null}"#;
        assert!(parse(ok).unwrap().configured);
        assert!(parse(r#"{"ok":false,"status":null,"error":"x"}"#).is_err());
    }

    /// Живой статус: cargo test live_browser_policy -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn live_browser_policy() {
        println!("chrome policy → {:?}", status("com.google.Chrome"));
    }
}
