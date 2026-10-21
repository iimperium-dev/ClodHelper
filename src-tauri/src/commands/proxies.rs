//! Пул прокси и назначения по сервисам.

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

// ─── Прокси-пул (до 5 upstream'ов + назначения по сервисам) ──────

/// TCP-пинг одного прокси из пула (мс). None = не подключились.
#[derive(Debug, Serialize)]
pub struct ProxyPing {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub ms: Option<u64>,
}

#[tauri::command]
pub fn proxies_get() -> Vec<String> {
    let s = settings::load();
    settings::effective_proxies(&s)
}

/// Заменить пул (1..=5; [0] — основной): валидация, сохранение, перерегистрация
/// задачи (vbs получает все --upstream) и перезапуск моста.
#[tauri::command]
pub async fn proxies_set(urls: Vec<String>) -> Result<Vec<String>, String> {
    for u in &urls {
        proxy::parse(u)?;
    }
    let saved = settings::set_proxies(urls)?;
    let primary = saved.proxy_url.clone().unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        // Не фатально: без задачи мост перезапустится с одним upstream (легаси).
        if let Err(e) = crate::tasks::register(&primary) {
            tracing::warn!("proxies_set: перерегистрация задачи: {e}");
        }
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(settings::effective_proxies(&settings::load()))
}

#[tauri::command]
pub fn proxy_assignments_get() -> std::collections::HashMap<String, usize> {
    settings::load().proxy_assignments
}

/// Имена (тэги) прокси: URL → имя.
#[tauri::command]
pub fn proxy_labels_get() -> std::collections::HashMap<String, String> {
    settings::load().proxy_labels
}

/// Задать/убрать имя прокси (пустое = убрать). Чисто косметика — мост не трогаем.
#[tauri::command]
pub fn proxy_label_set(
    url: String,
    label: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(settings::set_proxy_label(url, label)?.proxy_labels)
}

/// Назначения «сервис → прокси из пула»: сохранить, переписать rules,
/// перезапустить мост (rules читаются при старте).
#[tauri::command]
pub async fn proxy_assignments_set(
    assignments: std::collections::HashMap<String, usize>,
) -> Result<(), String> {
    settings::set_proxy_assignments(assignments)?;
    tokio::task::spawn_blocking(|| -> Result<(), String> {
        crate::tasks::write_rules_file()?;
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(())
}

/// TCP-пинг всего пула параллельно (лёгкий connect, не через интернет-запрос).
#[tauri::command]
pub async fn proxies_ping() -> Vec<ProxyPing> {
    let pool = settings::effective_proxies(&settings::load());
    let handles: Vec<_> = pool
        .into_iter()
        .map(|url| {
            tokio::spawn(async move {
                let (host, port) = match proxy::parse(&url) {
                    Ok(c) => (c.host, c.port),
                    Err(_) => return ProxyPing { url, host: String::new(), port: 0, ms: None },
                };
                let (h, p) = (host.clone(), port);
                // Три попытки, лучшая из удачных: один замер врёт — прокси,
                // на секунды переставший принимать новые соединения (бывает у
                // провайдеров), иначе висел бы красным «нет связи», хотя
                // мост через него работает. None — не ответил ни разу.
                let probe = tokio::task::spawn_blocking(move || {
                    use std::net::{TcpStream, ToSocketAddrs};
                    let addr = format!("{h}:{p}").to_socket_addrs().ok()?.next()?;
                    (0..3)
                        .filter_map(|_| {
                            let t = std::time::Instant::now();
                            TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(2500))
                                .ok()
                                .map(|_| t.elapsed().as_millis() as u64)
                        })
                        .min()
                });
                // DNS-резолв внутри блокирующей задачи своего таймаута не имеет —
                // ограничиваем всё целиком, иначе зависший резолвер держит
                // «пинг» в UI десятки секунд.
                let ms = match tokio::time::timeout(std::time::Duration::from_secs(9), probe).await {
                    Ok(Ok(ms)) => ms,
                    _ => None,
                };
                ProxyPing { url, host, port, ms }
            })
        })
        .collect();
    let mut out = Vec::new();
    for h in handles {
        if let Ok(p) = h.await {
            out.push(p);
        }
    }
    out
}
