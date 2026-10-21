//! Telegram через мост (macOS).
//!
//! Telegram для macOS (ru.keepcoder.Telegram) не читает HTTP_PROXY и не умеет
//! HTTP-прокси вовсе — только SOCKS5/MTProto. Поэтому мост (≥1.6) держит
//! SOCKS5-вход 127.0.0.1:8891, где всё идёт только через upstream.
//!
//! Записать прокси в настройки Telegram снаружи нельзя (они в его базе), но
//! есть штатная ссылка `tg://socks?server=…&port=…`: Telegram сам показывает
//! окно «Добавить прокси», пользователь жмёт «Подключить». То же понимает и
//! Telegram Desktop (Qt-версия).
//!
//! Факт «идёт ли через мост» берём не из настроек, а из живых соединений
//! процесса (lsof): есть сокеты к 127.0.0.1:8891 — через мост; есть сокеты
//! наружу мимо него — напрямую.

use serde::Serialize;

/// SOCKS5-вход моста для Telegram (см. bridge --socks-listen).
pub const SOCKS_PORT: u16 = 8891;

#[derive(Debug, Serialize, Default)]
pub struct TelegramStatus {
    /// Платформа поддерживает настройку (пока только macOS).
    pub supported: bool,
    /// Путь к Telegram.app / Telegram Desktop.app, если установлен.
    pub app_path: Option<String>,
    pub running: bool,
    /// Мост слушает SOCKS-вход (мост ≥1.6).
    pub socks_ready: bool,
    /// Соединений Telegram через мост прямо сейчас.
    pub via_bridge: u32,
    /// Соединений Telegram наружу мимо моста (не loopback).
    pub direct: u32,
}

#[cfg(not(target_os = "windows"))]
pub fn status() -> TelegramStatus {
    let app_path = crate::macos::find_app(&["Telegram.app", "Telegram Desktop.app"])
        .map(|p| p.to_string_lossy().to_string());
    let (running, via_bridge, direct) = connections();
    TelegramStatus {
        supported: true,
        app_path,
        running,
        socks_ready: socks_listening(),
        via_bridge,
        direct,
    }
}

/// Windows: Telegram Desktop, соединения — из общего снимка (netstat).
#[cfg(target_os = "windows")]
pub fn status() -> TelegramStatus {
    let app_path = crate::apps::find_telegram_exe().map(|p| p.to_string_lossy().to_string());
    let profile = crate::settings::load_with_presets()
        .app_profiles
        .into_iter()
        .filter(|p| p.id == crate::apps::TELEGRAM_ID)
        .collect::<Vec<_>>();
    let t = crate::traffic::snapshot(&profile).into_iter().next().unwrap_or_default();
    TelegramStatus {
        supported: app_path.is_some(),
        app_path,
        running: t.running,
        socks_ready: socks_listening(),
        via_bridge: t.via_bridge,
        direct: t.direct,
    }
}

/// Открыть в Telegram окно «Добавить прокси» с нашим SOCKS-входом.
#[cfg(not(target_os = "windows"))]
pub fn setup() -> Result<TelegramStatus, String> {
    let st = status();
    if st.app_path.is_none() {
        return Err("Telegram не найден в «Программах». Установите его и повторите.".into());
    }
    if !st.socks_ready {
        return Err(
            "Мост ещё не умеет SOCKS для Telegram — нажмите «Обновить мост» в Обзоре.".into(),
        );
    }
    // Защита от утечек по умолчанию: Telegram не пойдёт мимо прокси. Отказ
    // от пароля не мешает подключению — защиту можно включить позже.
    if let Ok(cur) = crate::leak_guard::status() {
        if let Ok(list) = crate::leak_guard::with_service(&cur, "telegram", true) {
            if !cur.enabled || cur.services != list {
                let _ = crate::leak_guard::set(&list);
            }
        }
    }
    let url = format!("tg://socks?server=127.0.0.1&port={SOCKS_PORT}");
    let ok = std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Не удалось открыть Telegram по ссылке tg://socks".into());
    }
    mark_prompted();
    Ok(st)
}

/// Запомнить, что окно прокси Telegram уже показывали.
fn mark_prompted() {
    let mut s = crate::settings::load();
    if !s.telegram_prompted {
        s.telegram_prompted = true;
        let _ = crate::settings::save(&s);
    }
}

/// Автонастройка: один раз, когда Telegram впервые замечен идущим напрямую
/// при готовом SOCKS-входе, сами открываем его окно «Добавить прокси». Telegram
/// не позволяет менять прокси снаружи — нажатие «Подключить» остаётся за
/// пользователем, но искать настройку ему не нужно. Дальше не навязываемся.
pub fn auto_prompt_once() {
    if crate::settings::load().telegram_prompted {
        return;
    }
    let st = status();
    if st.app_path.is_none() || !st.running || !st.socks_ready || st.via_bridge > 0 || st.direct == 0
    {
        return;
    }
    if setup().is_ok() {
        tracing::info!("Telegram идёт мимо моста — показал окно «Добавить прокси»");
    }
}

/// Windows: окно «Добавить прокси» Telegram Desktop по той же ссылке.
#[cfg(target_os = "windows")]
pub fn setup() -> Result<TelegramStatus, String> {
    use std::os::windows::process::CommandExt;
    let st = status();
    if st.app_path.is_none() {
        return Err("Telegram Desktop не найден. Установите его и повторите.".into());
    }
    if !st.socks_ready {
        return Err("Мост ещё не умеет SOCKS для Telegram — обновите мост в Обзоре.".into());
    }
    let url = format!("tg://socks?server=127.0.0.1&port={SOCKS_PORT}");
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .creation_flags(0x0800_0000)
        .status()
        .map_err(|e| format!("Не удалось открыть Telegram: {e}"))?;
    mark_prompted();
    Ok(st)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn socks_listening() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], SOCKS_PORT));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

/// (запущен?, соединений через мост, соединений мимо моста) по lsof.
#[cfg(not(target_os = "windows"))]
fn connections() -> (bool, u32, u32) {
    let running = std::process::Command::new("/usr/bin/pgrep")
        .args(["-x", "Telegram"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !running {
        return (false, 0, 0);
    }
    // -a: И между -c и -i; -Fn: только поле адреса («n127.0.0.1:5->127.0.0.1:8891»).
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", "-c", "Telegram", "-iTCP", "-sTCP:ESTABLISHED", "-Fn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let (via, direct) = count_connections(&out);
    (true, via, direct)
}

/// Разбор `lsof -Fn`: строки «n<локальный>-><удалённый>».
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn count_connections(lsof_fn: &str) -> (u32, u32) {
    let bridge = format!("127.0.0.1:{SOCKS_PORT}");
    let (mut via, mut direct) = (0, 0);
    for line in lsof_fn.lines() {
        let Some(remote) = line.strip_prefix('n').and_then(|l| l.split_once("->")).map(|(_, r)| r)
        else {
            continue;
        };
        if remote == bridge {
            via += 1;
        } else if !remote.starts_with("127.") && !remote.starts_with("[::1]") {
            direct += 1;
        }
    }
    (via, direct)
}

#[cfg(test)]
mod tests {
    use super::count_connections;

    #[test]
    fn counts_bridge_and_direct_connections() {
        let sample = "p10302\nf12\nn127.0.0.1:52001->127.0.0.1:8891\nf13\n\
                      n127.0.0.1:52002->127.0.0.1:8891\nf14\n\
                      n192.168.1.5:52100->149.154.167.51:443\nf15\n\
                      n127.0.0.1:52200->127.0.0.1:8889\n";
        assert_eq!(count_connections(sample), (2, 1));
        assert_eq!(count_connections(""), (0, 0));
    }
}
