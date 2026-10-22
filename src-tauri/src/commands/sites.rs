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

/// Браузер с расширениями, перехватывающими прокси: сайты в нём не работают.
#[derive(Debug, Serialize)]
pub struct BrowserProxyExtensions {
    pub name: String,
    pub path: String,
    pub extensions: Vec<crate::browser_ext::ProxyExtension>,
}

/// Для «Сайтов»: расширения-перехватчики во всех установленных Chromium-браузерах.
#[tauri::command]
pub async fn sites_proxy_extensions() -> Result<Vec<BrowserProxyExtensions>, String> {
    tokio::task::spawn_blocking(|| {
        crate::browser_ext::installed_browsers()
            .into_iter()
            .filter_map(|(name, path)| {
                let extensions = crate::browser_ext::proxy_extensions(&path);
                (!extensions.is_empty()).then_some(BrowserProxyExtensions { name, path, extensions })
            })
            .collect()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Открыть страницу расширения, чтобы его отключить. Путь — только из
/// списка найденных браузеров: произвольный exe по команде UI не запускаем.
#[tauri::command]
pub async fn sites_open_extension(path: String, ext_id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        if !crate::browser_ext::installed_browsers().iter().any(|(_, p)| *p == path) {
            return Err("Браузер не найден".to_string());
        }
        crate::browser_ext::open_extension_page(&path, &ext_id)
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// Браузер для режима «Только в выбранных»: PAC моста — его политикой.
#[derive(Debug, Serialize)]
pub struct ScopeBrowser {
    pub domain: &'static str,
    pub name: String,
    /// "pac" — сайты в этом браузере; "bridge" — весь трафик через мост
    /// («Всегда через мост» в Приложениях); "off"; "conflict" — чужая политика.
    pub state: &'static str,
    pub conflict: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SitesScope {
    /// Режим доступен (Windows).
    pub supported: bool,
    /// true — «Только в выбранных браузерах», false — «Во всех».
    pub selected: bool,
    pub browsers: Vec<ScopeBrowser>,
}

fn scope_browsers() -> Vec<ScopeBrowser> {
    let mut out: Vec<ScopeBrowser> = Vec::new();
    for (name, exe) in crate::browser_ext::installed_browsers() {
        let Some(domain) = crate::browser_policy::policy_domain(&exe) else { continue };
        if out.iter().any(|b| b.domain == domain) {
            continue;
        }
        let (state, conflict) = match crate::browser_policy::status(domain) {
            Ok(st) if st.configured && st.server == crate::browser_policy::PAC_SERVER => ("pac", None),
            Ok(st) if st.configured => ("bridge", None),
            Ok(st) if st.conflict.is_some() => ("conflict", st.conflict),
            Ok(_) => ("off", None),
            Err(e) => ("conflict", Some(e)),
        };
        out.push(ScopeBrowser { domain, name, state, conflict });
    }
    out
}

fn scope_now() -> SitesScope {
    SitesScope {
        supported: cfg!(target_os = "windows"),
        selected: settings::load().sites_scope_selected,
        browsers: scope_browsers(),
    }
}

#[tauri::command]
pub async fn sites_scope_get() -> Result<SitesScope, String> {
    tokio::task::spawn_blocking(scope_now).await.map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Переключить режим: «во всех» (системная автонастройка) или «только в
/// выбранных» (PAC моста политикой выбранных браузеров, остальные — вообще
/// без Clod Helper и без лишней проверки каждого запроса). Политики — одним
/// окном UAC; браузеры с «Всегда через мост» и с чужой политикой не трогаем.
#[tauri::command]
pub async fn sites_scope_set(selected: bool, browsers: Vec<String>) -> Result<SitesScope, String> {
    tokio::task::spawn_blocking(move || -> Result<SitesScope, String> {
        let current = scope_browsers();
        let pac_now: Vec<&str> = current.iter().filter(|b| b.state == "pac").map(|b| b.domain).collect();
        if selected {
            let enable: Vec<&str> = current
                .iter()
                .filter(|b| b.state == "off" && browsers.iter().any(|x| x == b.domain))
                .map(|b| b.domain)
                .collect();
            let disable: Vec<&str> = pac_now.iter().copied().filter(|d| !browsers.iter().any(|x| x == d)).collect();
            crate::browser_policy::apply_pac(&enable, &disable)?;
            if settings::load().system_pac {
                crate::system_pac::disable()?;
            }
        } else {
            crate::browser_policy::apply_pac(&[], &pac_now)?;
            crate::system_pac::enable()?;
        }
        settings::set_sites_scope_selected(selected)?;
        Ok(scope_now())
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))?
}

/// «Не трогать эти браузеры»: текущий список.
#[tauri::command]
pub fn sites_bypass_get() -> Vec<crate::browser_ext::BypassApp> {
    settings::load().sites_bypass
}

/// Кандидаты: установленные и запущенные Chromium-браузеры (кроме уже исключённых).
#[tauri::command]
pub async fn sites_bypass_candidates() -> Result<Vec<crate::browser_ext::BypassApp>, String> {
    tokio::task::spawn_blocking(|| {
        let taken = settings::load().sites_bypass;
        crate::browser_ext::bypass_candidates()
            .into_iter()
            .filter(|c| !taken.iter().any(|t| t.dir.eq_ignore_ascii_case(&c.dir)))
            .collect()
    })
    .await
    .map_err(|e| format!("Внутренняя ошибка: {e}"))
}

/// Программа по выбранному exe (кнопка «Выбрать программу…»).
#[tauri::command]
pub fn sites_bypass_from_exe(path: String) -> Result<crate::browser_ext::BypassApp, String> {
    let dir = crate::browser_ext::app_dir(&path).ok_or("Эту программу так не исключить — выберите exe внутри её папки")?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Программа".into());
    Ok(crate::browser_ext::BypassApp { name, dir, running: false })
}

/// Сохранить список и сразу отдать мосту (он перечитывает файл за ~2 с).
#[tauri::command]
pub async fn sites_bypass_set(apps: Vec<crate::browser_ext::BypassApp>) -> Result<Vec<crate::browser_ext::BypassApp>, String> {
    let apps: Vec<_> = apps
        .into_iter()
        .map(|mut a| {
            a.running = false;
            a
        })
        .collect();
    let saved = settings::set_sites_bypass(apps)?;
    tokio::task::spawn_blocking(crate::tasks::write_sites_file)
        .await
        .map_err(|e| format!("Внутренняя ошибка: {e}"))??;
    Ok(saved.sites_bypass)
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
/// `reset` — начать запись с чистого листа (мост ≥1.10.2 забывает прежние
/// хосты; старый параметр игнорирует).
#[tauri::command]
pub async fn sites_hosts(since: u64, reset: Option<bool>) -> Result<SeenHosts, String> {
    let reset = if reset.unwrap_or(false) { "&reset=1" } else { "" };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    client
        .get(format!("http://127.0.0.1:{}/hosts?since={since}{reset}", crate::tasks::SITES_PORT))
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

    /// Живой список (только чтение): cargo test live_scope -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_scope() {
        println!("{:#?}", super::scope_now());
    }

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
