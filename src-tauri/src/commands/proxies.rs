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

/// Состояние одного прокси из пула: TCP-пинг (мс) и настоящий запрос через
/// него. Открытый порт ещё не значит «работает»: мёртвый у провайдера прокси
/// часто принимает соединение и молчит — пинг у него красивый.
#[derive(Debug, Serialize)]
pub struct ProxyPing {
    pub url: String,
    pub host: String,
    pub port: u16,
    /// TCP-connect, мс. None — порт не ответил.
    pub ms: Option<u64>,
    /// Запрос через прокси прошёл. None — не проверяли (порт не ответил /
    /// URL не разобрать), тогда судить по `ms`.
    pub alive: Option<bool>,
    /// Почему не работает — человеческой строкой.
    pub error: Option<String>,
    /// Резерв какого прокси пула (индекс); None — сам прокси пула.
    pub backup_of: Option<usize>,
}

/// Настоящий запрос через прокси: маленький ответ 204 без TLS (быстро и
/// дёшево по трафику). Любой ответ сайта — прокси работает.
async fn probe_through(url: &str) -> Result<(), String> {
    let proxy = reqwest::Proxy::all(url).map_err(|e| format!("Неверный адрес прокси: {e}"))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    let mut last = String::new();
    // Две цели: прокси провайдера иногда режет один конкретный хост.
    for target in ["http://cp.cloudflare.com/generate_204", "http://www.gstatic.com/generate_204"] {
        match client.get(target).send().await {
            Ok(r) if r.status().as_u16() == 407 => return Err("Прокси не принял логин или пароль".into()),
            Ok(r) if r.status().is_server_error() => last = format!("Прокси ответил ошибкой {}", r.status().as_u16()),
            Ok(_) => return Ok(()),
            // Молчит или не пускает — дело в самом прокси, вторая цель не
            // поможет, а ожидание удвоит.
            Err(e) if e.is_timeout() => return Err("Порт открыт, но прокси молчит — не отвечает на запросы".into()),
            Err(e) if e.is_connect() => return Err("Не удалось подключиться к прокси".into()),
            Err(e) => last = format!("Ошибка: {e}"),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Прокси, который принимает соединение и молчит (так выглядит мёртвый
    /// у провайдера сервер), — «не работает», хотя TCP-пинг у него есть.
    #[tokio::test]
    async fn silent_proxy_is_not_alive() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for s in l.incoming().flatten() {
                held.push(s); // держим открытым и ничего не отвечаем
            }
        });
        let t = std::time::Instant::now();
        let err = probe_through(&format!("http://127.0.0.1:{port}")).await.unwrap_err();
        assert!(err.contains("молчит"), "{err}");
        assert!(t.elapsed() < std::time::Duration::from_secs(12), "одна цель — не ждём дважды");
    }

    /// Прокси, отвечающий на запросы, — работает.
    #[tokio::test]
    async fn answering_proxy_is_alive() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
            }
        });
        probe_through(&format!("http://127.0.0.1:{port}")).await.unwrap();
    }
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

/// Резервы прокси пула: URL прокси → его резервы по порядку.
#[tauri::command]
pub fn proxy_backups_get() -> std::collections::HashMap<String, Vec<String>> {
    settings::load().proxy_backups
}

/// Задать резервы прокси (пустой список — убрать все). Список прокси моста
/// меняется — перерегистрация задачи (файл прокси) и перезапуск моста.
#[tauri::command]
pub async fn proxy_backups_set(
    parent: String,
    backups: Vec<String>,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    for u in &backups {
        proxy::parse(u)?;
    }
    let saved = settings::set_proxy_backups(parent, backups)?;
    let primary = saved.proxy_url.clone().unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        crate::tasks::write_rules_file()?;
        if let Err(e) = crate::tasks::register(&primary) {
            tracing::warn!("proxy_backups_set: перерегистрация задачи: {e}");
        }
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(settings::load().proxy_backups)
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
    // Пул и резервы — в порядке моста: индексы совпадают с /healthz.
    let s = settings::load();
    let pool = settings::effective_proxies(&s);
    let parent_of = |url: &str| {
        pool.iter()
            .position(|p| s.proxy_backups.get(p).is_some_and(|l| l.iter().any(|b| b == url)))
    };
    let all: Vec<(String, Option<usize>)> = settings::bridge_upstreams(&s)
        .into_iter()
        .map(|u| {
            let of = if pool.contains(&u) { None } else { parent_of(&u) };
            (u, of)
        })
        .collect();
    let handles: Vec<_> = all
        .into_iter()
        .map(|(url, backup_of)| {
            tokio::spawn(async move {
                let (host, port) = match proxy::parse(&url) {
                    Ok(c) => (c.host, c.port),
                    Err(e) => {
                        return ProxyPing { url, host: String::new(), port: 0, ms: None, alive: None, error: Some(e), backup_of }
                    }
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
                // Порт не ответил — дальше проверять нечего.
                let (alive, error) = if ms.is_none() {
                    (None, Some("Прокси не отвечает — порт закрыт или сервер выключен".to_string()))
                } else {
                    match probe_through(&url).await {
                        Ok(()) => (Some(true), None),
                        Err(e) => (Some(false), Some(e)),
                    }
                };
                ProxyPing { url, host, port, ms, alive, error, backup_of }
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
