//! Claude's fixed proxy and the dedicated upstream-only bridge are routing
//! controls. They do not establish an OS boundary for arbitrary agent tools.
use serde::Serialize;

use crate::{claude_desktop::PROXY_PORT, claude_policy};

#[derive(Debug, Serialize)]
pub struct ClaudeProxyStatus {
    pub policy: claude_policy::ClaudePolicyStatus,
    pub bridge_ready: bool,
    pub bridge_error: Option<String>,
    /// Never infer enforcement from a proxy setting or a few firewall rules.
    pub network_enforced: bool,
}

pub fn validate_health(value: &serde_json::Value) -> Result<(), String> {
    let expected_listen = format!("127.0.0.1:{PROXY_PORT}");
    if value["service"] != "clod-bridge"
        || value["routing_policy"] != "upstream_only"
        || value["direct_allowed"] != false
        || value["strict_ready"] != true
        || value["mode"] != "all"
        || value["listen"] != expected_listen
        || value["port"] != PROXY_PORT
        || value["via_direct"] != 0
    {
        return Err("Мост Claude не подтвердил режим без прямых соединений. Обновите мост и проверьте HTTP-прокси.".into());
    }
    Ok(())
}

pub async fn check_bridge() -> Result<(), String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|_| "Не удалось проверить мост Claude".to_string())?;
    let response = client
        .get(format!("http://127.0.0.1:{PROXY_PORT}/healthz"))
        .send()
        .await
        .map_err(|_| format!("Мост Claude не отвечает на порту {PROXY_PORT}"))?
        .error_for_status()
        .map_err(|_| "Мост Claude вернул ошибку проверки".to_string())?;
    let health: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Порт Claude занят несовместимым мостом".to_string())?;
    validate_health(&health)
}

/// A supervisor process can exist before its worker has bound the socket.
pub async fn wait_for_bridge() -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        match check_bridge().await {
            Ok(()) => return Ok(()),
            Err(error) if tokio::time::Instant::now() >= deadline => return Err(error),
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }
}

pub async fn status() -> Result<ClaudeProxyStatus, String> {
    let policy = tokio::task::spawn_blocking(claude_policy::status)
        .await
        .map_err(|e| format!("Проверка настройки Claude: {e}"))??;
    let bridge_error = check_bridge().await.err();
    Ok(ClaudeProxyStatus {
        policy,
        bridge_ready: bridge_error.is_none(),
        bridge_error,
        network_enforced: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> serde_json::Value {
        serde_json::json!({"service":"clod-bridge", "routing_policy":"upstream_only",
            "direct_allowed":false, "strict_ready":true, "mode":"all",
            "listen":"127.0.0.1:8890", "port":8890, "via_direct":0})
    }

    #[test]
    fn refuses_legacy_shared_or_unconfigured_listener() {
        assert!(validate_health(&healthy()).is_ok());
        assert!(validate_health(&serde_json::json!({"status":"ok", "mode":"all"})).is_err());
        for (key, wrong) in [
            ("routing_policy", serde_json::json!("configured")),
            ("direct_allowed", serde_json::json!(true)),
            ("strict_ready", serde_json::json!(false)),
            ("mode", serde_json::json!("smart")),
            ("port", serde_json::json!(8889)),
            ("listen", serde_json::json!("0.0.0.0:8890")),
            ("via_direct", serde_json::json!(1)),
        ] {
            let mut value = healthy();
            value[key] = wrong;
            assert!(validate_health(&value).is_err(), "accepted invalid {key}");
        }
    }
}
