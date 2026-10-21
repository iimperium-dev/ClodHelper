//! Окна (трей-попап).

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

// ─── Окна (для трей-попапа) ──────────────────────────────────────

/// Показать главное окно (кнопка «Открыть приложение» в трей-попапе).
#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    crate::show_main_window(&app);
}

/// Полный выход из GUI. Мост — отдельный процесс, он продолжает работать.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub async fn settings_set_proxy(url: String) -> Result<Settings, String> {
    // 1) Validate proxy first — fail fast.
    proxy::parse(&url)?;

    // 2) Persist to settings.json.
    let saved = settings::set_proxy_url(url.clone())?;

    // 3) Перерегистрировать задачу автозапуска: runner-vbs получит новый
    //    upstream-URL (заодно мигрирует легаси-установку со Startup .vbs).
    let url_for_task = url.clone();
    tokio::task::spawn_blocking(move || crate::tasks::register(&url_for_task))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    // 4) Restart shim so new URL applies immediately.
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;

    Ok(saved)
}
