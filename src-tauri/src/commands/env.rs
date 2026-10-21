//! Глобальные переменные прокси.

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

// ─── Глобальные env-переменные (главный выключатель) ────────────

/// Прочитать глобальные HTTP(S)_PROXY (User scope) — состояние тумблера.
#[tauri::command]
pub async fn global_proxy_env() -> Result<GlobalProxyEnv, String> {
    // Внутри спавн PowerShell — с главного потока это фриз окна.
    tokio::task::spawn_blocking(env_proxy::read)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Включить системное проксирование: HTTP(S)_PROXY → мост (User scope, без UAC).
/// Заодно best-effort чистим легаси-ключи из settings.json VS Code — при
/// глобальном env они стали лишними дублями.
#[tauri::command]
pub async fn set_global_proxy_env() -> Result<GlobalProxyEnv, String> {
    tokio::task::spawn_blocking(|| {
        env_proxy::set(&format!("http://127.0.0.1:{}", shim::LISTEN_PORT))?;
        let _ = crate::vscode_config::disable();
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(env_proxy::read())
}

/// Удалить глобальные HTTP(S)_PROXY из системы (User scope, без UAC).
#[tauri::command]
pub async fn clear_global_proxy_env() -> Result<GlobalProxyEnv, String> {
    tokio::task::spawn_blocking(env_proxy::clear)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(env_proxy::read())
}
