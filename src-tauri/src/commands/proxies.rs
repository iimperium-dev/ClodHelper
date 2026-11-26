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
/// часто принимает соединение и молчит - пинг у него красивый.
#[derive(Debug, Serialize)]
pub struct ProxyPing {
    pub url: String,
    pub host: String,
    pub port: u16,
    /// TCP-connect, мс. None - порт не ответил.
    pub ms: Option<u64>,
    /// Запрос через прокси прошёл. None - не проверяли (порт не ответил /
    /// URL не разобрать), тогда судить по `ms`.
    pub alive: Option<bool>,
    /// Почему не работает - человеческой строкой.
    pub error: Option<String>,
    /// Резерв какого прокси пула (индекс); None - сам прокси пула.
    pub backup_of: Option<usize>,
}

/// Настоящий запрос через прокси: маленький ответ 204 без TLS (быстро и
/// дёшево по трафику). Любой ответ сайта - прокси работает.
async fn probe_through(url: &str) -> Result<(), String> {
    // SOCKS5: имя сайта пусть разрешает сам прокси (socks5h), как мост. Иначе
    // проверка зависела бы от DNS страны пользователя - в части стран он
    // подменяет или режет ответы, и рабочий прокси выглядел «мёртвым».
    let url = match url.strip_prefix("socks5://") {
        Some(rest) => format!("socks5h://{rest}"),
        None => url.to_string(),
    };
    let builder = || -> Result<reqwest::ClientBuilder, String> {
        let proxy = reqwest::Proxy::all(&url).map_err(|e| format!("Неверный адрес прокси: {e}"))?;
        Ok(reqwest::Client::builder()
            .proxy(proxy)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(8)))
    };
    // VPN включён - проверяем мимо него, как ходит мост. Мимо не пустил -
    // ещё раз обычным путём.
    let (b, bound) = crate::net_bypass::apply(builder()?, &proxy_host(&url));
    let r = probe_with(b.build().map_err(|e| format!("Внутренняя ошибка: {e}"))?).await;
    if bound && r.is_err() {
        let plain = probe_with(builder()?.build().map_err(|e| format!("Внутренняя ошибка: {e}"))?).await;
        if plain.is_ok() {
            crate::net_bypass::note_blocked();
        }
        return plain;
    }
    r
}

/// Хост прокси из адреса (для решения, обходить ли VPN).
fn proxy_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    let hostport = rest.split('/').next().unwrap_or(rest);
    match hostport.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
        _ => hostport.to_string(),
    }
}

async fn probe_with(client: reqwest::Client) -> Result<(), String> {
    let mut last = String::new();
    // Несколько целей: прокси (или сеть) иногда режет конкретный хост или
    // обычный http - тогда пробуем следующую, в том числе https (CONNECT).
    for target in [
        "http://cp.cloudflare.com/generate_204",
        "http://www.gstatic.com/generate_204",
        "https://www.google.com/generate_204",
    ] {
        match client.get(target).send().await {
            Ok(r) if r.status().as_u16() == 407 => return Err("Прокси не принял логин или пароль".into()),
            Ok(r) if r.status().is_server_error() => last = format!("Прокси ответил ошибкой {}", r.status().as_u16()),
            Ok(_) => return Ok(()),
            // Молчит - другие цели не помогут, а ожидание умножится. Если прокси
            // исправен (на сервере проверка прошла), так выглядит блокировка IP
            // сервера у провайдера: соединение открывается, данные пропадают.
            Err(e) if e.is_timeout() => return Err("Порт открыт, но прокси молчит. Скорее всего, ваш провайдер блокирует IP этого сервера: попросите у хостинга новый IP".into()),
            Err(e) if e.is_connect() => last = "Не удалось подключиться к прокси".into(),
            Err(e) => last = format!("Ошибка: {e}"),
        }
    }
    Err(last)
}

/// Замер скорости скачивания через прокси.
#[derive(Debug, Serialize)]
pub struct ProxySpeed {
    /// Мегабит в секунду.
    pub mbps: f64,
    /// Сколько скачали за замер, байт.
    pub bytes: u64,
    /// Сколько длился замер, с (от первого ответа до конца).
    pub secs: f64,
}

/// Потолок замера: не больше стольких байт и секунд - прокси бывают с
/// оплатой за трафик, а для оценки скорости этого хватает.
const SPEED_MAX_BYTES: u64 = 25_000_000;
const SPEED_MAX_SECS: f64 = 8.0;

/// Откуда качать при замере: большие файлы на быстрых серверах; второй -
/// если первый недоступен через этот прокси.
const SPEED_TARGETS: &[&str] = &[
    "https://speed.cloudflare.com/__down?bytes=25000000",
    "https://proof.ovh.net/files/100Mb.dat",
];

/// Скачать через прокси до потолка и посчитать скорость. Время считаем от
/// ответа сервера: подключение и рукопожатие - это пинг, не скорость.
async fn speed_through(url: &str, targets: &[&str]) -> Result<ProxySpeed, String> {
    let url = match url.strip_prefix("socks5://") {
        Some(rest) => format!("socks5h://{rest}"),
        None => url.to_string(),
    };
    let proxy = reqwest::Proxy::all(&url).map_err(|e| format!("Неверный адрес прокси: {e}"))?;
    let b = reqwest::Client::builder().proxy(proxy).connect_timeout(std::time::Duration::from_secs(6));
    // Мимо VPN, если пинг не выяснил, что он туда не пускает.
    let (b, _) = crate::net_bypass::apply(b, &proxy_host(&url));
    let client = b.build().map_err(|e| format!("Внутренняя ошибка: {e}"))?;
    let mut last = "Не удалось начать замер".to_string();
    for target in targets {
        let resp = match tokio::time::timeout(std::time::Duration::from_secs(10), client.get(*target).send()).await {
            Err(_) => {
                last = "Прокси не отвечает на запрос".into();
                continue;
            }
            Ok(Err(e)) if e.is_connect() => {
                last = "Не удалось подключиться к прокси".into();
                continue;
            }
            Ok(Err(e)) => {
                last = format!("Ошибка: {e}");
                continue;
            }
            Ok(Ok(r)) => r,
        };
        if resp.status().as_u16() == 407 {
            return Err("Прокси не принял логин или пароль".into());
        }
        if !resp.status().is_success() {
            last = format!("Сервер замера ответил ошибкой {}", resp.status().as_u16());
            continue;
        }
        let mut resp = resp;
        let start = std::time::Instant::now();
        let mut bytes: u64 = 0;
        loop {
            let left = SPEED_MAX_SECS - start.elapsed().as_secs_f64();
            if left <= 0.0 || bytes >= SPEED_MAX_BYTES {
                break;
            }
            match tokio::time::timeout(std::time::Duration::from_secs_f64(left), resp.chunk()).await {
                Ok(Ok(Some(c))) => bytes += c.len() as u64,
                // Файл кончился, время вышло или связь оборвалась - считаем, что успели.
                _ => break,
            }
        }
        let secs = start.elapsed().as_secs_f64();
        if bytes == 0 {
            last = "Через прокси не пришло ни одного байта".into();
            continue;
        }
        let mbps = bytes as f64 * 8.0 / secs.max(0.001) / 1_000_000.0;
        return Ok(ProxySpeed { mbps, bytes, secs });
    }
    Err(last)
}

/// Замер скорости одного прокси (по кнопке - качает до 25 МБ).
#[tauri::command]
pub async fn proxy_speedtest(url: String) -> Result<ProxySpeed, String> {
    proxy::parse(&url)?;
    speed_through(&url, SPEED_TARGETS).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTP-прокси, отдающий на любой запрос 2 МБ, - скорость посчитана,
    /// байты сходятся.
    #[tokio::test]
    async fn speedtest_counts_bytes() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        const N: usize = 2_000_000;
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {N}\r\n\r\n").as_bytes());
                let _ = s.write_all(&vec![0u8; N]);
            }
        });
        let r = speed_through(&format!("http://127.0.0.1:{port}"), &["http://example.test/file"]).await.unwrap();
        assert_eq!(r.bytes, N as u64);
        assert!(r.mbps > 0.0);
    }

    /// Прокси с неверным паролем - понятная ошибка, а не «0 Мбит/с».
    #[tokio::test]
    async fn speedtest_reports_bad_auth() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n");
            }
        });
        let err = speed_through(&format!("http://127.0.0.1:{port}"), &["http://example.test/file"]).await.unwrap_err();
        assert!(err.contains("логин"), "{err}");
    }

    /// Прокси, который принимает соединение и молчит (так выглядит мёртвый
    /// у провайдера сервер), - «не работает», хотя TCP-пинг у него есть.
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
        assert!(t.elapsed() < std::time::Duration::from_secs(12), "одна цель - не ждём дважды");
    }

    /// Прокси, отвечающий на запросы, - работает.
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

/// Заменить пул (1..=5; [0] - основной): валидация, сохранение, перерегистрация
/// задачи (vbs получает все --upstream) и перезапуск моста.
#[tauri::command]
pub async fn proxies_set(urls: Vec<String>) -> Result<Vec<String>, String> {
    for u in &urls {
        proxy::parse(u)?;
    }
    let empty = urls.is_empty();
    let saved = settings::set_proxies(urls)?;
    let primary = saved.proxy_url.clone().unwrap_or_default();
    if empty {
        // Без прокси мост не работает: останавливаем и снимаем с автозапуска.
        // Приложения через мост остаются без сети - напрямую не выпускаем.
        tokio::task::spawn_blocking(|| {
            let _ = shim::stop();
            crate::tasks::unregister();
            let _ = crate::tasks::write_rules_file();
        })
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?;
        return Ok(Vec::new());
    }
    restart_with_pool(primary).await?;
    Ok(settings::effective_proxies(&settings::load()))
}

/// Пул поменялся: перерегистрация задачи (vbs получает все --upstream) и
/// перезапуск моста.
pub(crate) async fn restart_with_pool(primary: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        // Не фатально: без задачи мост перезапустится с одним upstream (легаси).
        if let Err(e) = crate::tasks::register(&primary) {
            tracing::warn!("proxies_set: перерегистрация задачи: {e}");
        }
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Заменить прокси на новый (обычно - тот же сервер с новым логином или
/// паролем). Имя, резервы, назначения и сайты остаются за ним. Мост
/// перезапускается с новым списком.
#[tauri::command]
pub async fn proxy_replace(old: String, new: String) -> Result<Vec<String>, String> {
    proxy::parse(&new)?;
    let saved = settings::replace_proxy(&old, new)?;
    let primary = saved.proxy_url.clone().unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        crate::tasks::write_rules_file()?;
        if let Err(e) = crate::tasks::register(&primary) {
            tracing::warn!("proxy_replace: перерегистрация задачи: {e}");
        }
        shim::restart()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(settings::effective_proxies(&settings::load()))
}

/// Включён ли VPN рядом с мостом (и запущен ли SOTA VPN).
#[tauri::command]
pub async fn vpn_status() -> crate::vpn::VpnStatus {
    tokio::task::spawn_blocking(crate::vpn::status).await.unwrap_or_default()
}

/// «Добавить исключения в Happ»: открыть Happ ссылкой с правилом «к прокси -
/// напрямую». Из Rust, а не из окна: окну разрешены только веб-ссылки.
#[tauri::command]
pub fn vpn_happ_add(app: AppHandle, hosts: Vec<String>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if hosts.iter().all(|h| h.trim().is_empty()) {
        return Err("Нет адресов прокси".into());
    }
    app.opener()
        .open_url(crate::vpn::happ_routing_link(&hosts), None::<&str>)
        .map_err(|_| "Не получилось открыть Happ. Он установлен?".to_string())
}

/// Резервы прокси пула: URL прокси → его резервы по порядку.
#[tauri::command]
pub fn proxy_backups_get() -> std::collections::HashMap<String, Vec<String>> {
    settings::load().proxy_backups
}

/// Задать резервы прокси (пустой список - убрать все). Список прокси моста
/// меняется - перерегистрация задачи (файл прокси) и перезапуск моста.
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

/// Блокируется ли телеметрия Claude (тумблер в настройках Claude Desktop).
#[tauri::command]
pub fn claude_telemetry_get() -> bool {
    settings::claude_telemetry_blocked(&settings::load())
}

/// Включить/выключить блокировку: мост перечитывает правила сам, без
/// перезапуска (ответы Claude не обрываются).
#[tauri::command]
pub async fn claude_telemetry_set(on: bool) -> Result<bool, String> {
    settings::set_claude_telemetry_blocked(on)?;
    tokio::task::spawn_blocking(crate::tasks::write_rules_file)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(on)
}

/// Блокируется ли реклама Google (тумблер на экране «Сайты»).
#[tauri::command]
pub fn google_ads_get() -> bool {
    settings::google_ads_blocked(&settings::load())
}

/// Включить/выключить блокировку рекламы Google. Мост перечитывает правила
/// сам; системный PAC обновляем, чтобы браузеры взяли новый список доменов.
#[tauri::command]
pub async fn google_ads_set(on: bool) -> Result<bool, String> {
    settings::set_google_ads_blocked(on)?;
    tokio::task::spawn_blocking(crate::tasks::write_rules_file)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    std::thread::spawn(|| {
        // Мост перечитывает правила раз в 2 с - PAC после этого.
        std::thread::sleep(std::time::Duration::from_millis(2500));
        if let Err(e) = crate::system_pac::refresh() {
            tracing::warn!("обновление системного PAC: {e}");
        }
    });
    Ok(on)
}

/// Имена (тэги) прокси: URL → имя.
#[tauri::command]
pub fn proxy_labels_get() -> std::collections::HashMap<String, String> {
    settings::load().proxy_labels
}

/// Задать/убрать имя прокси (пустое = убрать). Чисто косметика - мост не трогаем.
#[tauri::command]
pub fn proxy_label_set(
    url: String,
    label: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(settings::set_proxy_label(url, label)?.proxy_labels)
}

/// Прокси (URL пула и резервов), о поломке которых не предупреждаем.
#[tauri::command]
pub fn proxy_alerts_muted_get() -> Vec<String> {
    settings::muted_alert_urls(&settings::load())
}

/// Скрыть или вернуть предупреждение «не отвечает» о прокси. Мост и
/// проверки не трогаем: прокси остаётся в пуле и проверяется как раньше.
#[tauri::command]
pub fn proxy_alert_mute(url: String, mute: bool) -> Result<Vec<String>, String> {
    let s = settings::with_settings(|s| settings::set_alert_muted(s, &url, mute))?;
    Ok(settings::muted_alert_urls(&s))
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
    // Пул и резервы - в порядке моста: индексы совпадают с /healthz.
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
                // Три попытки, лучшая из удачных: один замер врёт - прокси,
                // на секунды переставший принимать новые соединения (бывает у
                // провайдеров), иначе висел бы красным «нет связи», хотя
                // мост через него работает. None - не ответил ни разу.
                let probe = tokio::task::spawn_blocking(move || {
                    use std::net::ToSocketAddrs;
                    let addr = format!("{h}:{p}").to_socket_addrs().ok()?.next()?;
                    (0..3)
                        .filter_map(|_| {
                            let t = std::time::Instant::now();
                            crate::net_bypass::connect_timeout(&addr, std::time::Duration::from_millis(2500))
                                .ok()
                                .map(|_| t.elapsed().as_millis() as u64)
                        })
                        .min()
                });
                // DNS-резолв внутри блокирующей задачи своего таймаута не имеет -
                // ограничиваем всё целиком, иначе зависший резолвер держит
                // «пинг» в UI десятки секунд.
                let ms = match tokio::time::timeout(std::time::Duration::from_secs(9), probe).await {
                    Ok(Ok(ms)) => ms,
                    _ => None,
                };
                // Порт не ответил - дальше проверять нечего.
                let (alive, error) = if ms.is_none() {
                    (None, Some("Прокси не отвечает - порт закрыт или сервер выключен".to_string()))
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
    // Скрытое предупреждение о прокси, который снова заработал, снимаем:
    // сломается ещё раз - человек увидит это как новую поломку.
    let alive: Vec<String> = out.iter().filter(|p| p.alive == Some(true)).map(|p| p.url.clone()).collect();
    if !alive.is_empty() && !s.muted_proxy_alerts.is_empty() {
        let _ = settings::with_settings(|s| {
            settings::unmute_alive(s, &alive);
            Ok(())
        });
    }
    out
}
