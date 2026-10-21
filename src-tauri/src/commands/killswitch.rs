//! Killswitch (Windows-файрвол).

#![allow(unused_imports)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::apps::{self, AppProfile, AppProfileStatus, ShortcutTarget};
use crate::env_proxy::{self, GlobalProxyEnv};
use crate::firewall::{self, KillswitchStatus};
use crate::proxy::{self, ProxyCheckResult, ProxyConfig};
use crate::settings::{self, Settings};
use crate::shim::{self, ShimStatus, ShimTestResult};
use crate::system::{self, CursorInfo, NodeInfo, PythonInfo};

use super::*;

// ─── Killswitch (firewall) ──────────────────────────────────────

/// Фактическое состояние killswitch (читает Windows Firewall, без UAC).
#[tauri::command]
pub async fn killswitch_status() -> KillswitchStatus {
    tokio::task::spawn_blocking(firewall::status)
        .await
        .unwrap_or(KillswitchStatus {
            active: false,
            rule_count: 0,
            blocked_processes: Vec::new(),
        })
}

/// Включить killswitch: собрать пути процессов включённых профилей и
/// поставить outbound-block правила (один UAC-промпт).
#[tauri::command]
pub async fn killswitch_enable() -> Result<KillswitchStatus, String> {
    let s = settings::load_with_presets();
    let paths = tokio::task::spawn_blocking({
        let profiles = s.app_profiles.clone();
        move || apps::resolve_process_paths(&profiles)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;

    if paths.is_empty() {
        return Err(
            "Не нашёл процессов для блокировки. Включите приложение (VS Code / Codex) \
             и убедитесь что оно установлено."
                .into(),
        );
    }

    tokio::task::spawn_blocking(move || firewall::enable(&paths))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    let _ = settings::set_killswitch(true);
    Ok(tokio::task::spawn_blocking(firewall::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?)
}

/// Выключить killswitch: снять все наши правила (один UAC-промпт).
#[tauri::command]
pub async fn killswitch_disable() -> Result<KillswitchStatus, String> {
    tokio::task::spawn_blocking(firewall::disable)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    let _ = settings::set_killswitch(false);
    Ok(tokio::task::spawn_blocking(firewall::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?)
}

/// «Защита от утечек» (pf, macOS): текущее состояние и живая проверка.
#[tauri::command]
pub async fn leak_guard_status() -> Result<crate::leak_guard::LeakGuardStatus, String> {
    tokio::task::spawn_blocking(crate::leak_guard::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Включить для выбранных сервисов (пустой список — снять). Пароль Mac.
#[tauri::command]
pub async fn leak_guard_set(services: Vec<String>) -> Result<crate::leak_guard::LeakGuardStatus, String> {
    let list = crate::leak_guard::normalize(&services)?;
    tokio::task::spawn_blocking(move || crate::leak_guard::set(&list))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}
