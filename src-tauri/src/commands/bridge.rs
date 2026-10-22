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
    let found = crate::updater::check().await?;
    crate::update_notify::remember(&found);
    Ok(found)
}

/// Последнее найденное обновление (без запроса к GitHub) — для окна-уведомления.
#[tauri::command]
pub fn update_latest() -> Option<crate::updater::UpdateInfo> {
    crate::update_notify::latest()
}

/// Скачать, проверить SHA-256, поставить и перезапуститься. Ход —
/// событиями `update:progress` (окно обновления рисует по ним прогресс).
#[tauri::command]
pub async fn update_install(app: AppHandle, info: crate::updater::UpdateInfo) -> Result<(), String> {
    let emitter = app.clone();
    let progress = move |p: crate::updater::UpdateProgress| {
        let _ = emitter.emit("update:progress", p);
    };
    crate::updater::install(&info, &progress).await?;
    crate::update_notify::close_toast(&app);
    progress(crate::updater::UpdateProgress { stage: "restart", received: 0, total: 0 });
    // Пауза, чтобы «Перезапускаю…» успело показаться: иначе окно исчезает
    // посреди «Устанавливаю» и это выглядит как вылет.
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    // Новую версию поставит и запустит фоновый скрипт (macOS) или
    // установщик (Windows), когда мы выйдем.
    app.exit(0);
    Ok(())
}

/// Клик по уведомлению «Доступно обновление».
#[tauri::command]
pub fn update_toast_open(app: AppHandle) {
    crate::update_notify::open_from_toast(&app);
}

#[tauri::command]
pub fn update_toast_close(app: AppHandle) {
    crate::update_notify::close_toast(&app);
}

/// Версия для окна «Что нового», если приложение только что обновилось.
#[tauri::command]
pub fn whats_new_take() -> Result<Option<String>, String> {
    settings::take_whats_new(env!("CARGO_PKG_VERSION"))
}

/// Фирменный звук уведомления.
#[tauri::command]
pub fn play_chime() {
    crate::sound::chime();
}

/// «Отчёт для поддержки»: состояние программы текстом, без паролей.
#[tauri::command]
pub async fn diagnostics_report() -> Result<String, String> {
    tokio::task::spawn_blocking(crate::diagnostics::report)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
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
    // Мост уже умеет сайты, но задача зарегистрирована со старыми
    // аргументами (мост поставили поверх раньше, чем появились сайты) —
    // перерегистрировать: иначе браузер в режиме «только сайты» без сети.
    if !v.outdated && tokio::task::spawn_blocking(crate::tasks::sites_args_stale).await.unwrap_or(false) {
        let fixed = tokio::task::spawn_blocking(|| -> Result<(), String> {
            let url = settings::load().proxy_url.ok_or("прокси не настроен")?;
            crate::tasks::register(&url)?;
            crate::shim::restart()
        })
        .await;
        match fixed {
            Ok(Ok(())) => tracing::info!("мост перезапущен со входом сайтов"),
            other => tracing::warn!("не удалось включить вход сайтов: {other:?}"),
        }
        return;
    }
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
