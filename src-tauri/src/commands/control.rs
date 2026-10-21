//! Управление мостом (старт/стоп/тест) и настройки.

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

// ─── Shim control (Dashboard) ────────────────────────────────────

#[tauri::command]
pub async fn shim_status() -> Result<ShimStatus, String> {
    // Скан процессов sysinfo — сотни миллисекунд; с главного потока — фриз.
    tokio::task::spawn_blocking(shim::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub async fn shim_start() -> Result<(), String> {
    // Запуск через .vbs — синхронный shell, обернём в blocking чтобы не
    // блокировать tokio executor.
    tokio::task::spawn_blocking(shim::start)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_stop() -> Result<usize, String> {
    tokio::task::spawn_blocking(shim::stop)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_restart() -> Result<(), String> {
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn shim_test() -> ShimTestResult {
    shim::test().await
}

#[tauri::command]
pub fn shim_script_path() -> Option<String> {
    // Сигнал «мост установлен» для App.tsx (Dashboard vs Wizard).
    // Новый маркер — clod-bridge.exe; легаси — local-proxy.py (установки,
    // где exe ещё не появился).
    shim::bridge_exe_path()
        .filter(|p| p.exists())
        .or_else(|| shim::shim_script_path().filter(|p| p.exists()))
        .map(|p| p.to_string_lossy().to_string())
}

// ─── Settings ────────────────────────────────────────────────────

#[tauri::command]
pub fn settings_get() -> Settings {
    settings::load()
}
