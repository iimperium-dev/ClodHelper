//! Сайты через прокси и «Подсмотреть».

use serde::Serialize;

use crate::settings::{self, SiteEntry};

/// Домен из того, что вставил пользователь: «https://www.YouTube.com/watch?v=…»
/// → «youtube.com». None — не похоже на домен.
pub(crate) fn normalize_domain(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let s = s.rsplit_once('@').map_or(s, |(_, host)| host);
    let s = s.trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.');
    // Порт отбрасываем (IPv6-литералы сайтам не нужны).
    let s = s.split(':').next().unwrap_or("");
    // Кириллические домены — в punycode (мост сравнивает с CONNECT-хостом).
    let host = url::Url::parse(&format!("http://{s}/")).ok()?.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let label_ok = |l: &str| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    (host.contains('.') && host.len() <= 253 && host.split('.').all(label_ok)).then_some(host)
}

#[tauri::command]
pub fn sites_get() -> Vec<SiteEntry> {
    settings::load().sites
}

/// Сохранить сайты и переписать bridge-sites.json. Мост перечитывает файл
/// сам за ~2 с — без перезапуска, стримы Claude не рвутся.
#[tauri::command]
pub async fn sites_set(sites: Vec<SiteEntry>) -> Result<Vec<SiteEntry>, String> {
    let mut clean = Vec::with_capacity(sites.len());
    for mut site in sites {
        let mut domains: Vec<String> = Vec::new();
        for d in &site.domains {
            let n = normalize_domain(d).ok_or_else(|| format!("«{d}» не похоже на адрес сайта"))?;
            if !domains.contains(&n) {
                domains.push(n);
            }
        }
        if domains.is_empty() {
            return Err(format!("У сайта «{}» нет адресов", site.name));
        }
        site.name = site.name.trim().to_string();
        if site.name.is_empty() {
            site.name = domains[0].clone();
        }
        site.domains = domains;
        clean.push(site);
    }
    let saved = settings::set_sites(clean)?;
    tokio::task::spawn_blocking(crate::tasks::write_sites_file)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    // Мост перечитает файл за ~2 с — после этого PAC с новой меткой, чтобы
    // браузеры запросили его заново. Фоном: окно не ждёт.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        if let Err(e) = crate::system_pac::refresh() {
            tracing::warn!("обновление системного PAC: {e}");
        }
    });
    Ok(saved.sites)
}

/// «Сайты во всех браузерах»: состояние системной автонастройки прокси.
#[tauri::command]
pub async fn system_pac_status() -> Result<crate::system_pac::SystemPacStatus, String> {
    tokio::task::spawn_blocking(crate::system_pac::status)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

#[tauri::command]
pub async fn system_pac_set(enable: bool) -> Result<crate::system_pac::SystemPacStatus, String> {
    tokio::task::spawn_blocking(move || {
        if enable {
            crate::system_pac::enable()
        } else {
            crate::system_pac::disable()
        }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Запись «Подсмотреть»: на её время PAC ведёт через мост весь трафик
/// браузеров — иначе мост не увидит адресов, которых ещё нет в списке.
#[tauri::command]
pub async fn sites_peek(active: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || crate::system_pac::set_peek(active))
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

#[tauri::command]
pub fn sites_normalize(input: String) -> Option<String> {
    normalize_domain(&input)
}

/// Браузер для раздела «Сайты»: как он сейчас ходит через мост.
#[derive(Debug, Serialize)]
pub struct SitesBrowser {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// "sites" — только AI и сайты (вход 8887); "all" — весь трафик через
    /// прокси; "ai" — мост старый, только AI-сайты на общем входе.
    pub mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SitesStatus {
    /// Установленный мост понимает сайты (≥1.9).
    pub supported: bool,
    pub browsers: Vec<SitesBrowser>,
}

#[tauri::command]
pub async fn sites_status() -> Result<SitesStatus, String> {
    tokio::task::spawn_blocking(|| {
        let s = settings::load_with_presets();
        let statuses = crate::apps::statuses(&s.app_profiles);
        let browsers = s
            .app_profiles
            .iter()
            .filter(|p| !p.hidden && crate::apps::is_chromium_browser(p))
            .map(|p| SitesBrowser {
                id: p.id.clone(),
                name: p.name.clone(),
                installed: statuses.iter().any(|st| st.id == p.id && st.installed),
                mode: crate::apps::browser_mode(p),
            })
            .collect();
        // По факту: вход сайтов слушается, а не только exe новый.
        let supported = crate::apps::sites_listener_available() && crate::tasks::sites_listener_up();
        SitesStatus { supported, browsers }
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Хост, к которому ходил браузер через вход сайтов.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct SeenHost {
    pub host: String,
    pub count: u64,
    pub first_ms: u64,
    pub last_ms: u64,
    /// "proxy" | "direct".
    pub via: String,
    pub site: Option<String>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct SeenHosts {
    pub now_ms: u64,
    pub hosts: Vec<SeenHost>,
}

/// «Подсмотреть»: хосты со входа сайтов, к которым ходили с момента `since`
/// (миллисекунды по часам моста — их отдаёт тот же ответ в now_ms).
#[tauri::command]
pub async fn sites_hosts(since: u64) -> Result<SeenHosts, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    client
        .get(format!("http://127.0.0.1:{}/hosts?since={since}", crate::tasks::SITES_PORT))
        .send()
        .await
        .map_err(|_| "Вход сайтов моста не отвечает — обновите мост или перезапустите его".to_string())?
        .json::<SeenHosts>()
        .await
        .map_err(|e| format!("Не удалось разобрать ответ моста: {e}"))
}

#[cfg(test)]
mod tests {
    use super::normalize_domain;

    #[test]
    fn domains_from_pasted_urls() {
        for (input, want) in [
            ("https://www.YouTube.com/watch?v=1", Some("youtube.com")),
            ("youtube.com", Some("youtube.com")),
            ("  m.youtube.com:443 ", Some("m.youtube.com")),
            ("*.googlevideo.com", Some("googlevideo.com")),
            ("http://user:pw@x.com/", Some("x.com")),
            ("кто.рф", Some("xn--j1ail.xn--p1ai")),
            ("localhost", None),
            ("", None),
            ("bad domain.com", None),
        ] {
            assert_eq!(normalize_domain(input).as_deref(), want, "{input}");
        }
    }
}
