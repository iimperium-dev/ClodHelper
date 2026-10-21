//! Автозапуск моста, здоровье, маршрутизация, обновление.

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

// ─── Автозапуск моста (Планировщик задач) ───────────────────────

/// Статус задачи автозапуска моста.
#[tauri::command]
pub async fn bridge_task_status() -> crate::tasks::BridgeTaskStatus {
    tokio::task::spawn_blocking(crate::tasks::status)
        .await
        .unwrap_or(crate::tasks::BridgeTaskStatus {
            registered: false,
            state: None,
        })
}

/// Зарегистрировать задачу автозапуска (использует proxy_url из settings).
/// Удаляет легаси .vbs из Startup после успешной регистрации.
#[tauri::command]
pub async fn bridge_task_register() -> Result<crate::tasks::BridgeTaskStatus, String> {
    let url = settings::load()
        .proxy_url
        .ok_or_else(|| "Прокси ещё не настроен — сначала сохраните его".to_string())?;
    tokio::task::spawn_blocking(move || -> Result<crate::tasks::BridgeTaskStatus, String> {
        crate::tasks::register(&url)?;
        // Повторная настройка должна сразу восстановить мост, а не только
        // записать задачу автозапуска до следующего входа в Windows.
        shim::start()?;
        Ok(crate::tasks::status())
    })
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

// ─── Мост: здоровье и маршрутизация ─────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeUpstreamHealth {
    pub url: String,
    pub healthy: bool,
    pub ok: u64,
    pub fail: u64,
    /// Отказов подряд (мост 1.5.0+; у старых — 0).
    #[serde(default)]
    pub fail_streak: u64,
}

/// Одна ошибка из кольцевого буфера моста: секунды-от-старта + текст.
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeErrorEntry {
    pub t: u64,
    pub msg: String,
}

/// Зеркало JSON от GET /healthz нового Rust-моста.
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeHealth {
    pub status: String,
    pub version: String,
    pub uptime_sec: u64,
    pub listen: String,
    pub mode: String,
    pub active: u64,
    pub total: u64,
    pub via_upstream: u64,
    pub via_direct: u64,
    pub errors: u64,
    pub upstreams: Vec<BridgeUpstreamHealth>,
    /// Последние ошибки моста (мост 1.1.0+; у старых версий поля нет).
    #[serde(default)]
    pub last_errors: Vec<BridgeErrorEntry>,
}

/// Установленный мост — старый python-exe (без /healthz и --rules)?
/// Проба `--version` кэшируется по mtime exe. По этому флагу UI показывает
/// «Обновить мост» — надёжнее, чем гадать по тексту ошибки healthz (старый
/// мост может и таймаутить запрос, и вернуть мусор — зависит от upstream).
#[tauri::command]
pub async fn bridge_exe_legacy() -> Result<bool, String> {
    tokio::task::spawn_blocking(crate::tasks::bridge_exe_is_legacy)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Версии моста: установленного и встроенного в приложение. Если встроенный
/// новее — UI предлагает «Обновить мост» (иначе после апдейта Clod Helper
/// пользователь навсегда остался бы на старом мосте: exe копируется только
/// мастером установки).
#[derive(Debug, Serialize)]
pub struct BridgeVersions {
    pub installed: Option<String>,
    pub bundled: Option<String>,
    pub outdated: bool,
}

#[tauri::command]
pub async fn bridge_versions(app: AppHandle) -> Result<BridgeVersions, String> {
    let bundled_path = resolve_resource(&app, shim::BRIDGE_EXE).ok();
    tokio::task::spawn_blocking(move || {
        let installed = shim::bridge_exe_path()
            .filter(|p| p.exists())
            .and_then(|p| crate::tasks::exe_version(&p));
        let bundled = bundled_path
            .filter(|p| p.exists())
            .and_then(|p| crate::tasks::exe_version(&p));
        let outdated = match (
            installed.as_deref().and_then(crate::tasks::parse_version),
            bundled.as_deref().and_then(crate::tasks::parse_version),
        ) {
            (Some(i), Some(b)) => b > i,
            _ => false,
        };
        BridgeVersions { installed, bundled, outdated }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}


// ─── Обновления ─────────────────────────────────────────────────

/// Есть ли новая версия Clod Helper (последний релиз GitHub).
#[tauri::command]
pub async fn update_check() -> Result<Option<crate::updater::UpdateInfo>, String> {
    crate::updater::check().await
}

/// Скачать, проверить SHA-256, поставить и перезапуститься.
#[tauri::command]
pub async fn update_install(app: AppHandle, info: crate::updater::UpdateInfo) -> Result<(), String> {
    crate::updater::install(&info).await?;
    // Новую версию поставит и запустит фоновый скрипт, когда мы выйдем.
    app.exit(0);
    Ok(())
}

/// При запуске: встроенный мост новее установленного → обновить сразу.
/// Без этого после обновления приложения мост оставался старым до ручного
/// «Обновить мост» (а новые возможности — SOCKS, закреплённые входы —
/// недоступны).
pub(crate) async fn auto_update_bridge(app: AppHandle) {
    if !crate::tasks::is_registered() {
        return;
    }
    // Правила новой версии (например, новые домены Claude) — сразу в мост.
    let rules_changed = tokio::task::spawn_blocking(crate::tasks::write_rules_file_if_changed)
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some_and(|(_, changed)| changed);
    let Ok(v) = bridge_versions(app.clone()).await else { return };
    if !v.outdated {
        if rules_changed {
            match tokio::task::spawn_blocking(crate::shim::restart).await {
                Ok(Ok(())) => tracing::info!("правила моста обновлены"),
                other => tracing::warn!("перезапуск моста с новыми правилами не удался: {other:?}"),
            }
        }
        return;
    }
    match bridge_update(app).await {
        Ok(_) => tracing::info!("мост обновлён автоматически: {:?} → {:?}", v.installed, v.bundled),
        Err(e) => tracing::warn!("автообновление моста не удалось: {e}"),
    }
}
