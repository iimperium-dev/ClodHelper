//! «Сайты во всех браузерах»: системная автонастройка прокси (PAC) Windows.
//!
//! Вместо того чтобы подключать каждый браузер к мосту, Windows получает
//! адрес PAC-файла, который отдаёт сам мост (`/proxy.pac` на входе сайтов).
//! Chrome, Edge, Brave, Яндекс, Opera и Firefox («системные настройки
//! прокси») сами спрашивают его, куда вести каждый сайт: сайты из списка и
//! AI-сервисы — через мост, остальное — напрямую, мимо него. Ни прав
//! администратора, ни перезапуска браузеров: они следят за этими настройками.
//!
//! Пишем официальным API WinINet (per-connection options) — так же, как
//! «Параметры → Прокси»: значение AutoConfigURL в реестре без флагов в
//! DefaultConnectionSettings браузеры не читают. Прежние настройки (чужой
//! прокси, корпоративный PAC, автоопределение) сохраняются и возвращаются
//! при выключении.

use serde::{Deserialize, Serialize};

/// Адрес PAC на входе сайтов моста (tasks::SITES_PORT).
pub const PAC_BASE: &str = "http://127.0.0.1:8887/proxy.pac";

/// Системные настройки прокси (соединение по умолчанию, LAN).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyConf {
    /// PROXY_TYPE_*: 1 — напрямую, 2 — прокси, 4 — PAC, 8 — автоопределение.
    pub flags: u32,
    pub server: Option<String>,
    pub bypass: Option<String>,
    pub pac_url: Option<String>,
}

const DIRECT: u32 = 1;
const MANUAL: u32 = 2;
const AUTO_URL: u32 = 4;
const AUTO_DETECT: u32 = 8;

impl ProxyConf {
    /// Сейчас стоит наш PAC.
    pub fn is_ours(&self) -> bool {
        self.flags & AUTO_URL != 0 && self.pac_url.as_deref().is_some_and(|u| u.starts_with(PAC_BASE))
    }

    /// Чужая настройка, которую включение заменит, — человеку стоит знать.
    /// Автоопределение (оно стоит в Windows по умолчанию) — не в счёт.
    pub fn foreign(&self) -> Option<String> {
        if self.is_ours() {
            return None;
        }
        if self.flags & AUTO_URL != 0 {
            if let Some(u) = self.pac_url.as_deref().filter(|u| !u.is_empty()) {
                return Some(format!("сценарий настройки {u}"));
            }
        }
        if self.flags & MANUAL != 0 {
            if let Some(s) = self.server.as_deref().filter(|s| !s.is_empty()) {
                return Some(format!("прокси {s}"));
            }
        }
        None
    }

    /// Наш вариант: только PAC. Автоопределение (WPAD) выключаем — Windows и
    /// Chrome пробуют его раньше PAC, и каждый запуск ждали бы его тайм-аут.
    pub fn ours(url: &str) -> Self {
        Self { flags: DIRECT | AUTO_URL, server: None, bypass: None, pac_url: Some(url.to_string()) }
    }
}

/// Адрес PAC с меткой свежести: браузеры кэшируют сценарий, а новый адрес
/// заставляет перечитать его сразу (после правки сайтов, для «Подсмотреть»).
pub fn pac_url(peek: bool) -> String {
    let v = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if peek {
        format!("{PAC_BASE}?all=1&v={v}")
    } else {
        format!("{PAC_BASE}?v={v}")
    }
}

#[cfg(target_os = "windows")]
mod win {
    use super::ProxyConf;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinInet::{
        InternetQueryOptionW, InternetSetOptionW, INTERNET_OPTION_PER_CONNECTION_OPTION, INTERNET_OPTION_REFRESH,
        INTERNET_OPTION_SETTINGS_CHANGED, INTERNET_PER_CONN_AUTOCONFIG_URL, INTERNET_PER_CONN_FLAGS,
        INTERNET_PER_CONN_OPTIONW, INTERNET_PER_CONN_OPTION_LISTW, INTERNET_PER_CONN_PROXY_BYPASS,
        INTERNET_PER_CONN_PROXY_SERVER,
    };

    fn option(id: u32) -> INTERNET_PER_CONN_OPTIONW {
        // SAFETY: POD-структура WinInet; нули — валидное «пустое» значение.
        let mut o: INTERNET_PER_CONN_OPTIONW = unsafe { std::mem::zeroed() };
        o.dwOption = id;
        o
    }

    fn list(options: &mut [INTERNET_PER_CONN_OPTIONW]) -> INTERNET_PER_CONN_OPTION_LISTW {
        INTERNET_PER_CONN_OPTION_LISTW {
            dwSize: std::mem::size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
            // NULL — соединение по умолчанию (LAN), как в «Параметрах».
            pszConnection: std::ptr::null_mut(),
            dwOptionCount: options.len() as u32,
            dwOptionError: 0,
            pOptions: options.as_mut_ptr(),
        }
    }

    /// Строка, выделенная WinInet: прочитать и освободить (GlobalFree).
    unsafe fn take(p: *mut u16) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        GlobalFree(p as _);
        Some(s)
    }

    pub fn read() -> Result<ProxyConf, String> {
        let mut opts = [
            option(INTERNET_PER_CONN_FLAGS),
            option(INTERNET_PER_CONN_PROXY_SERVER),
            option(INTERNET_PER_CONN_PROXY_BYPASS),
            option(INTERNET_PER_CONN_AUTOCONFIG_URL),
        ];
        let mut l = list(&mut opts);
        let mut size = l.dwSize;
        // SAFETY: l и opts живы на время вызова; WinInet пишет в opts.
        let ok = unsafe {
            InternetQueryOptionW(std::ptr::null(), INTERNET_OPTION_PER_CONNECTION_OPTION, &mut l as *mut _ as _, &mut size)
        };
        if ok == 0 {
            return Err(format!("Не прочитать настройки прокси Windows: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: для FLAGS заполнен dwValue, для остальных — pszValue.
        unsafe {
            Ok(ProxyConf {
                flags: opts[0].Value.dwValue,
                server: take(opts[1].Value.pszValue),
                bypass: take(opts[2].Value.pszValue),
                pac_url: take(opts[3].Value.pszValue),
            })
        }
    }

    pub fn write(conf: &ProxyConf) -> Result<(), String> {
        let wide = |s: &Option<String>| -> Vec<u16> {
            s.as_deref().unwrap_or("").encode_utf16().chain(std::iter::once(0)).collect()
        };
        let (mut server, mut bypass, mut url) = (wide(&conf.server), wide(&conf.bypass), wide(&conf.pac_url));
        let mut opts = [
            option(INTERNET_PER_CONN_FLAGS),
            option(INTERNET_PER_CONN_PROXY_SERVER),
            option(INTERNET_PER_CONN_PROXY_BYPASS),
            option(INTERNET_PER_CONN_AUTOCONFIG_URL),
        ];
        opts[0].Value.dwValue = conf.flags;
        opts[1].Value.pszValue = server.as_mut_ptr();
        opts[2].Value.pszValue = bypass.as_mut_ptr();
        opts[3].Value.pszValue = url.as_mut_ptr();
        let l = list(&mut opts);
        // SAFETY: строки и opts живут до конца вызовов; WinInet копирует значения.
        unsafe {
            if InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_PER_CONNECTION_OPTION, &l as *const _ as _, l.dwSize) == 0 {
                return Err(format!("Не записать настройки прокси Windows: {}", std::io::Error::last_os_error()));
            }
            // Сообщить программам, что настройки изменились (браузеры перечитают).
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null(), 0);
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_REFRESH, std::ptr::null(), 0);
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub use win::{read, write};

#[cfg(not(target_os = "windows"))]
pub fn read() -> Result<ProxyConf, String> {
    Err("Пока только для Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn write(_: &ProxyConf) -> Result<(), String> {
    Err("Пока только для Windows".into())
}

/// Для UI: включено ли (намерение), стоит ли сейчас наш PAC, что чужое.
#[derive(Debug, Clone, Serialize)]
pub struct SystemPacStatus {
    pub supported: bool,
    pub enabled: bool,
    /// В Windows сейчас стоит наш PAC. enabled && !active — настройку
    /// сбросила другая программа (VPN, «Параметры»).
    pub active: bool,
    /// Чужая настройка прокси в системе (её заменит включение / она стоит
    /// вместо нашей).
    pub foreign: Option<String>,
}

pub fn status() -> SystemPacStatus {
    let enabled = crate::settings::load().system_pac;
    match read() {
        Ok(c) => SystemPacStatus { supported: true, enabled, active: c.is_ours(), foreign: c.foreign() },
        Err(_) => SystemPacStatus { supported: false, enabled, active: false, foreign: None },
    }
}

/// Включить: запомнить прежние настройки (если стоит не наш PAC) и поставить свой.
pub fn enable() -> Result<SystemPacStatus, String> {
    let cur = read()?;
    if !cur.is_ours() {
        crate::settings::set_system_pac_saved(Some(cur))?;
    }
    write(&ProxyConf::ours(&pac_url(false)))?;
    crate::settings::set_system_pac(true)?;
    Ok(status())
}

/// Выключить: вернуть то, что было до нас. Если наш PAC уже кто-то заменил —
/// его настройку не трогаем.
pub fn disable() -> Result<SystemPacStatus, String> {
    let saved = crate::settings::load().system_pac_saved;
    if read()?.is_ours() {
        let restore = saved.unwrap_or(ProxyConf { flags: DIRECT | AUTO_DETECT, ..Default::default() });
        write(&restore)?;
    }
    crate::settings::set_system_pac(false)?;
    crate::settings::set_system_pac_saved(None)?;
    Ok(status())
}

/// Идёт запись «Подсмотреть»: PAC ведёт через мост весь трафик браузеров.
static PEEK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Перечитать PAC браузерами (сайты изменились). Только если наш PAC стоит —
/// чужой не перезаписываем. Режим записи «Подсмотреть» сохраняется.
pub fn refresh() -> Result<(), String> {
    if !crate::settings::load().system_pac || !read()?.is_ours() {
        return Ok(());
    }
    write(&ProxyConf::ours(&pac_url(PEEK.load(std::sync::atomic::Ordering::Relaxed))))
}

/// Начать/закончить запись «Подсмотреть».
pub fn set_peek(on: bool) -> Result<(), String> {
    PEEK.store(on, std::sync::atomic::Ordering::Relaxed);
    refresh()
}

/// При запуске приложения: если наш PAC стоит, вернуть обычный режим (запись
/// «Подсмотреть» могла остаться от закрытого окна) и обновить метку свежести.
pub fn on_startup() {
    if let Err(e) = set_peek(false) {
        tracing::warn!("системный PAC при запуске: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ours_and_foreign() {
        let ours = ProxyConf::ours(&pac_url(false));
        assert!(ours.is_ours());
        assert_eq!(ours.foreign(), None);
        assert!(pac_url(true).contains("all=1"));
        let wpad_only = ProxyConf { flags: DIRECT | AUTO_DETECT, ..Default::default() };
        assert!(!wpad_only.is_ours());
        assert_eq!(wpad_only.foreign(), None, "автоопределение Windows по умолчанию — не чужой прокси");
        let manual = ProxyConf { flags: DIRECT | MANUAL, server: Some("127.0.0.1:2080".into()), ..Default::default() };
        assert_eq!(manual.foreign().as_deref(), Some("прокси 127.0.0.1:2080"));
        let corp = ProxyConf { flags: DIRECT | AUTO_URL, pac_url: Some("http://corp/wpad.dat".into()), ..Default::default() };
        assert!(corp.foreign().unwrap().contains("corp"));
        // Флаг ручного прокси без адреса — не чужая настройка.
        assert_eq!(ProxyConf { flags: DIRECT | MANUAL, ..Default::default() }.foreign(), None);
    }

    /// Живое чтение (ничего не меняет): cargo test live_system_proxy -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_system_proxy() {
        println!("{:?}", read());
    }
}
