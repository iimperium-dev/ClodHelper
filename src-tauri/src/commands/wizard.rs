//! Мастер установки одним вызовом.

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

// ─── Новый мастер (3 шага): установка одним вызовом ──────────────

/// Результат установки нового мастера.
#[derive(Debug, Serialize)]
pub struct WizardInstallResult {
    /// Реальная проверка соединения через мост (не «зелёный pill по флагу»).
    pub test: shim::ShimTestResult,
    /// Включилось ли системное проксирование (env). false — не фатально,
    /// тумблер есть в Обзоре.
    pub env_on: bool,
}

/// Установка для нового мастера: мост из ресурсов → настройки + задача
/// автозапуска + старт → системное проксирование → пресеты приложений →
/// честная проверка соединения. Прогресс — события "wizard:step"
/// ("<фаза>:start|ok|err"). Легаси PS-скрипты (python-мост, npm, режимы
/// установки) в этом пути не участвуют — всё делают существующие Rust-пути.
#[tauri::command]
pub async fn wizard_install(
    app: AppHandle,
    proxy_url: String,
) -> Result<WizardInstallResult, String> {
    let step = |phase: &str, state: &str| {
        let _ = app.emit("wizard:step", format!("{phase}:{state}"));
    };
    proxy::parse(&proxy_url)?;

    // 1) Мост: копия из ресурсов в %LOCALAPPDATA%\ClaudeDesktopProxy.
    step("bridge", "start");
    let src = resolve_resource(&app, shim::BRIDGE_EXE)?;
    let dst = shim::bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    let copied = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let _ = shim::stop(); // переустановка поверх живого моста: exe залочен
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
        let mut last_err = String::new();
        for _ in 0..3 {
            match shim::install_bridge_binary(&src, &dst) {
                Ok(_) => {
                    last_err.clear();
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
            }
        }
        if last_err.is_empty() {
            Ok(())
        } else {
            Err(format!("Не удалось установить мост: {last_err}"))
        }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    if let Err(e) = copied {
        step("bridge", "err");
        return Err(e);
    }
    step("bridge", "ok");

    // 2) Настройки + задача автозапуска + запуск моста.
    step("task", "start");
    let url = proxy_url.clone();
    let started = tokio::task::spawn_blocking(move || -> Result<(), String> {
        settings::set_proxy_url(url.clone())?;
        crate::tasks::register(&url)?;
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    if let Err(e) = started {
        step("task", "err");
        return Err(e);
    }
    step("task", "ok");

    // 3) Системное проксирование — главный выключатель новой модели.
    //    Неудача не фатальна: тумблер есть в Обзоре.
    step("env", "start");
    let env_on = tokio::task::spawn_blocking(|| {
        env_proxy::set(&format!("http://127.0.0.1:{}", shim::LISTEN_PORT)).is_ok()
    })
    .await
    .unwrap_or(false);
    step("env", if env_on { "ok" } else { "err" });

    // 4) Пресеты профилей приложений — чтобы раздел «Приложения» был готов.
    step("presets", "start");
    let _ = tokio::task::spawn_blocking(settings::load_with_presets).await;
    step("presets", "ok");

    // 5) Claude Desktop (macOS): штатный прокси Claude — одна системная
    //    просьба пароля прямо в мастере, а не поиск кнопки потом. Отмена или
    //    конфликт не фатальны: в «Приложениях» будет «Подключить».
    #[cfg(not(target_os = "windows"))]
    {
        let has_claude = tokio::task::spawn_blocking(|| crate::claude_desktop::detect().is_some())
            .await
            .unwrap_or(false);
        if has_claude {
            step("claude", "start");
            let configured = tokio::task::spawn_blocking(crate::claude_policy::status)
                .await
                .ok()
                .and_then(|r| r.ok())
                .is_some_and(|s| s.configured);
            let ok = configured || claude_proxy_enable(app.clone()).await.is_ok();
            step("claude", if ok { "ok" } else { "err" });
        } else {
            step("claude", "skip");
        }
    }

    // 6) Telegram (macOS): мост уже слушает SOCKS — открываем окно «Добавить
    //    прокси». Нет Telegram — шаг пропускается. Не фатально.
    #[cfg(not(target_os = "windows"))]
    {
        step("telegram", "start");
        let installed = tokio::task::spawn_blocking(|| {
            let st = crate::telegram::status();
            if st.app_path.is_some() && st.socks_ready && st.via_bridge == 0 {
                let _ = crate::telegram::setup();
            }
            st.app_path.is_some()
        })
        .await
        .unwrap_or(false);
        step("telegram", if installed { "ok" } else { "skip" });
    }

    // 7) Честная проверка: реальный запрос через мост, не зелёный pill.
    step("test", "start");
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let test = shim::test().await;
    step("test", if test.ok { "ok" } else { "err" });

    Ok(WizardInstallResult { test, env_on })
}
