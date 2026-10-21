//! Автозапуск приложения.

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

// ─── Автозапуск приложения ──────────────────────────────────────

#[tauri::command]
pub async fn autostart_get(app: AppHandle) -> Result<crate::autostart::AutostartState, String> {
    tokio::task::spawn_blocking(move || crate::autostart::get(&app))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub async fn autostart_set(
    app: AppHandle,
    enabled: bool,
) -> Result<crate::autostart::AutostartState, String> {
    tokio::task::spawn_blocking(move || crate::autostart::set(&app, enabled))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Живые счётчики моста. Ошибка = мост не отвечает ИЛИ это старый python-мост
/// без /healthz (UI предложит «Обновить мост»).
#[tauri::command]
pub async fn bridge_health() -> Result<BridgeHealth, String> {
    // Один клиент на всё время жизни: UI спрашивает здоровье каждые 5с, а
    // сборка reqwest-клиента (TLS-конфиг, пул) — не бесплатная.
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = match CLIENT.get() {
        Some(c) => c,
        None => {
            let c = reqwest::Client::builder()
                .no_proxy() // сами к себе — не через системный прокси
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .map_err(|e| format!("client: {e}"))?;
            CLIENT.get_or_init(|| c)
        }
    };
    let resp = client
        .get(format!("http://127.0.0.1:{}/healthz", shim::LISTEN_PORT))
        .send()
        .await
        .map_err(|e| format!("мост не отвечает: {e}"))?;
    resp.json::<BridgeHealth>()
        .await
        .map_err(|e| format!("не удалось разобрать healthz: {e}"))
}

/// Переключить режим маршрутизации моста ("smart" | "all"): сохранить в
/// settings, переписать rules-файл (+vbs при известном upstream), перезапустить.
#[tauri::command]
pub async fn bridge_set_route_mode(mode: String) -> Result<(), String> {
    if mode != "smart" && mode != "all" {
        return Err("Режим должен быть smart или all".into());
    }
    settings::set_route_mode(mode)?;
    // Задача уже могла быть зарегистрирована без --rules (старый формат) —
    // полная перерегистрация гарантирует актуальные аргументы runner-vbs.
    if let Some(url) = settings::load().proxy_url {
        tokio::task::spawn_blocking(move || crate::tasks::register(&url))
            .await
            .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    } else {
        tokio::task::spawn_blocking(crate::tasks::write_rules_file)
            .await
            .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    }
    tokio::task::spawn_blocking(shim::restart)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(())
}

/// Обновить установленный мост из ресурсов приложения: stop → copy → start.
/// Нужен для миграции живых установок со старого python-моста на Rust-мост
/// (новый exe попадает в resources при апдейте Clod Helper).
#[tauri::command]
pub async fn bridge_update(app: AppHandle) -> Result<String, String> {
    let src = resolve_resource(&app, shim::BRIDGE_EXE)?;
    if !src.exists() {
        return Err(format!("В ресурсах приложения нет {}", shim::BRIDGE_EXE));
    }
    let dst = shim::bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let _ = shim::stop(); // работающий exe залочен Windows — сначала стоп
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
        }
        // Страховка отката: старый exe сохраняем рядом (.bak) — если новый
        // мост у пользователя поведёт себя хуже, можно вернуть вручную.
        if dst.exists() {
            let mut bak = dst.clone().into_os_string();
            bak.push(".bak");
            let _ = std::fs::copy(&dst, bak);
        }
        // Файл может освобождаться не мгновенно после kill — пара ретраев.
        let mut last_err = String::new();
        for _ in 0..3 {
            match shim::install_bridge_binary(&src, &dst) {
                Ok(_) => {
                    // Перерегистрировать задачу: новый Rust-мост понимает
                    // --rules, vbs должен его получить (иначе smart-режим не
                    // применится). Не фатально — легаси-vbs тоже рабочий.
                    if let Some(url) = settings::load().proxy_url {
                        if let Err(e) = crate::tasks::register(&url) {
                            tracing::warn!("bridge_update: перерегистрация задачи не удалась: {e}");
                        }
                    }
                    shim::start()?;
                    return Ok(format!("Мост обновлён: {}", dst.display()));
                }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        // Копирование не удалось (exe залочен антивирусом и т.п.) — старый exe
        // на месте, но мост уже остановлен shim::stop(). Обязательно поднимаем
        // обратно, иначе прокси лежит намертво до ручного «Запустить».
        if let Err(e) = shim::start() {
            tracing::warn!("bridge_update: откат-запуск старого моста не удался: {e}");
        }
        Err(format!("Не удалось скопировать мост: {last_err} (старый мост перезапущен)"))
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}
