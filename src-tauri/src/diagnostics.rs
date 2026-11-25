//! «Отчёт для поддержки»: всё, что нужно, чтобы понять проблему человека
//! удалённо, - одним текстом, который он пересылает разработчику.
//!
//! Без секретов: адреса прокси - только host:port (логин и пароль никогда не
//! попадают в отчёт), чужие прокси-переменные - тоже без учётных данных.

use std::fmt::Write as _;
use std::time::Duration;

/// «http://user:pass@1.2.3.4:8000» → «1.2.3.4:8000».
fn host_port(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    rest.trim_end_matches('/').to_string()
}

/// Значение прокси-переменной без учётных данных.
fn mask_proxy_value(v: &str) -> String {
    match v.split_once("://") {
        Some((scheme, _)) => format!("{scheme}://{}", host_port(v)),
        None => host_port(v),
    }
}

/// Целиком, как его видит человек в программе.
pub fn report() -> String {
    build(false)
}

/// Версия для отправки разработчику: без списка сайтов, меток прокси,
/// корпоративных прокси и чужих доменов из ошибок моста. Видимый в программе
/// отчёт не меняется.
pub fn report_for_server() -> String {
    build(true)
}

/// Домен из встроенного списка сервисов (не личный, полезен для разбора).
pub(crate) fn is_service_domain(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_lowercase();
    crate::tasks::SERVICE_GROUPS
        .iter()
        .flat_map(|(_, d)| d.iter())
        .any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

/// Ошибка моста для сервера: домены вне списка сервисов - <host>.
fn redact_bridge_error(msg: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(назначение для |прямое соединение с )([^\s:]+)").expect("regex")
    });
    re.replace_all(msg, |c: &regex::Captures| {
        let host = &c[2];
        format!("{}{}", &c[1], if is_service_domain(host) { host } else { "<host>" })
    })
    .into_owned()
}

/// Чужой (корпоративный) прокси: адрес не отправляем. Свой мост виден как есть.
fn mask_corp(v: &str) -> String {
    let hp = host_port(v).to_lowercase();
    if hp.starts_with("127.0.0.1") || hp.starts_with("localhost") || hp.starts_with("[::1]") {
        mask_proxy_value(v)
    } else {
        "<proxy>".to_string()
    }
}

fn build(redact: bool) -> String {
    let mut r = String::new();
    let s = crate::settings::load_with_presets();
    let _ = writeln!(r, "=== Clod Helper - отчёт для поддержки ===");
    let _ = writeln!(r, "Версия: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        r,
        "ОС: {} ({})",
        sysinfo::System::long_os_version().unwrap_or_default(),
        std::env::consts::ARCH
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(r, "Время (unix): {now}");
    let _ = writeln!(r, "Автообновление: {}", if s.auto_update == Some(true) { "включено" } else { "выключено" });
    if let Some(w) = crate::auto_update::last_wait_line() {
        let _ = writeln!(r, "Тихая установка: {w}");
    }

    // ── Мост ──
    let _ = writeln!(r, "\n--- Мост ---");
    let exe = crate::shim::bridge_exe_path().filter(|p| p.exists());
    let _ = writeln!(
        r,
        "Установлен: {}",
        exe.as_deref().and_then(crate::tasks::exe_version).unwrap_or_else(|| "нет / не запускается".into())
    );
    let st = crate::shim::status();
    let _ = writeln!(r, "Процесс: {}", if st.running { "работает" } else { "НЕ работает" });
    let task = crate::tasks::status();
    let _ = writeln!(r, "Автозапуск: {}", if task.registered { task.state.unwrap_or_default() } else { "не настроен".into() });
    let ports = [
        (crate::shim::LISTEN_PORT, "общий"),
        (crate::claude_desktop::PROXY_PORT, "Claude"),
        (crate::tasks::SITES_PORT, "сайты"),
    ];
    let mut errors_shown = false;
    for (port, name) in ports {
        match crate::monitor::fetch_healthz_port(port, Duration::from_millis(500)) {
            Some(h) => {
                let _ = writeln!(
                    r,
                    "Вход {port} ({name}): v{} режим={} uptime={}с всего={} через прокси={} напрямую={} ошибок={} активных={}",
                    h["version"].as_str().unwrap_or("?"),
                    h["mode"].as_str().unwrap_or("?"),
                    h["uptime_sec"],
                    h["total"],
                    h["via_upstream"],
                    h["via_direct"],
                    h["errors"],
                    h["active"],
                );
                if port == crate::shim::LISTEN_PORT {
                    for u in h["upstreams"].as_array().into_iter().flatten() {
                        let _ = writeln!(
                            r,
                            "  прокси {}: здоров={} ok={} fail={} подряд={} задержка={}мс",
                            host_port(u["url"].as_str().unwrap_or("")),
                            u["healthy"],
                            u["ok"],
                            u["fail"],
                            u["fail_streak"],
                            u["latency_ms"],
                        );
                    }
                }
                if let Some(errs) = h["last_errors"].as_array().filter(|e| !e.is_empty()) {
                    errors_shown = true;
                    let _ = writeln!(r, "  последние ошибки входа {port}:");
                    for e in errs.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
                        let msg = e["msg"].as_str().unwrap_or("");
                        let msg = if redact { redact_bridge_error(msg) } else { msg.to_string() };
                        let _ = writeln!(r, "    [{}с] {}", e["t"], msg);
                    }
                }
            }
            None => {
                let _ = writeln!(r, "Вход {port} ({name}): НЕ отвечает");
            }
        }
    }
    if !errors_shown {
        let _ = writeln!(r, "Ошибок в журнале моста нет");
    }

    // ── Прокси и маршрутизация ──
    let _ = writeln!(r, "\n--- Прокси ---");
    let pool = crate::settings::effective_proxies(&s);
    for (i, u) in pool.iter().enumerate() {
        let label = if redact {
            String::new()
        } else {
            s.proxy_labels.get(u).map(|l| format!(" «{l}»")).unwrap_or_default()
        };
        let _ = writeln!(r, "#{i}{label}: {}", host_port(u));
    }
    if pool.is_empty() {
        let _ = writeln!(r, "пул пуст");
    }
    let _ = writeln!(r, "Режим общего входа: {}", s.route_mode.as_deref().unwrap_or("all"));
    if !s.proxy_assignments.is_empty() {
        let mut a: Vec<_> = s.proxy_assignments.iter().collect();
        a.sort();
        let _ = writeln!(r, "Закрепления: {a:?}");
    }
    let env = crate::env_proxy::read();
    let _ = writeln!(
        r,
        "Переменные прокси: {}",
        if !env.present {
            "нет".to_string()
        } else if env.points_to_bridge {
            "на мост".to_string()
        } else {
            env.values
                .iter()
                .map(|(k, v)| format!("{k}={}", if redact { mask_corp(v) } else { mask_proxy_value(v) }))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );

    // ── Сайты ──
    let _ = writeln!(r, "\n--- Сайты ---");
    let pac = crate::system_pac::status();
    let _ = writeln!(
        r,
        "«Во всех браузерах»: поддержка={} включено={} стоит в системе={}{}",
        pac.supported,
        pac.enabled,
        pac.active,
        pac.foreign
            .map(|f| format!(" чужая настройка: {}", if redact { mask_corp(&f) } else { mask_proxy_value(&f) }))
            .unwrap_or_default()
    );
    if let Ok(c) = crate::system_pac::read() {
        let _ = writeln!(
            r,
            "Системный прокси {}: флаги={} PAC={} прокси={}",
            if cfg!(target_os = "macos") { "macOS" } else { "Windows" },
            c.flags,
            c.pac_url
                .as_deref()
                .map(|u| if redact { "<pac>".to_string() } else { u.split('?').next().unwrap_or(u).to_string() })
                .unwrap_or_else(|| "-".into()),
            c.server
                .as_deref()
                .map(|v| if redact { mask_corp(v) } else { mask_proxy_value(v) })
                .unwrap_or_else(|| "-".into()),
        );
    }
    // На сервер список сайтов не уходит: имена и домены личные. Только число.
    for site in s.sites.iter().filter(|_| !redact) {
        let _ = writeln!(
            r,
            "{} {}: {} (прокси: {})",
            if site.enabled { "✓" } else { "✗" },
            site.name,
            site.domains.join(", "),
            site.proxy.map(|i| format!("#{i}")).unwrap_or_else(|| "авто".into())
        );
    }
    if redact && !s.sites.is_empty() {
        let _ = writeln!(r, "Сайтов: {}", s.sites.len());
    }
    if s.sites.is_empty() {
        let _ = writeln!(r, "сайтов нет");
    }

    // ── Браузеры и приложения ──
    let _ = writeln!(r, "\n--- Приложения ---");
    for p in s.app_profiles.iter().filter(|p| !p.hidden) {
        let browser = crate::apps::is_chromium_browser(p);
        let mode = if browser { format!(" браузер: {}", crate::apps::browser_mode(p)) } else { String::new() };
        let policy = if browser {
            p.exe_path
                .as_deref()
                .and_then(crate::browser_policy::policy_domain)
                .and_then(|d| crate::browser_policy::status(d).ok())
                .map(|st| {
                    format!(
                        " политика={}{}",
                        if st.configured {
                            if redact { mask_corp(&st.server) } else { st.server.clone() }
                        } else {
                            "нет".into()
                        },
                        st.conflict
                            .map(|c| format!(" (конфликт: {})", if redact { "<proxy>".to_string() } else { c }))
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let _ = writeln!(
            r,
            "{} [{}]{}{}{}",
            p.name,
            p.id,
            if p.enabled { "" } else { " выкл" },
            mode,
            policy
        );
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_report_redacts_bridge_domains() {
        assert_eq!(
            redact_bridge_error("назначение для mybank.ru указывает на прокси вне пула"),
            "назначение для <host> указывает на прокси вне пула"
        );
        assert_eq!(redact_bridge_error("прямое соединение с shop.example.org:443 не удалось"), "прямое соединение с <host>:443 не удалось");
        assert_eq!(
            redact_bridge_error("прямое соединение с api.anthropic.com:443 не удалось"),
            "прямое соединение с api.anthropic.com:443 не удалось",
            "домены сервисов полезны для разбора"
        );
        assert_eq!(redact_bridge_error("назначение для claude.ai указывает"), "назначение для claude.ai указывает");
        assert_eq!(redact_bridge_error("прямое соединение с notclaude.ai:443"), "прямое соединение с <host>:443");
        assert_eq!(redact_bridge_error("другая ошибка"), "другая ошибка");
    }

    #[test]
    fn corporate_proxy_is_masked_for_server() {
        assert_eq!(mask_corp("http://corp.local:3128"), "<proxy>");
        assert_eq!(mask_corp("proxy.company.com:8080"), "<proxy>");
        assert_eq!(mask_corp("http://127.0.0.1:8889"), "http://127.0.0.1:8889");
    }

    #[test]
    fn secrets_never_leak() {
        assert_eq!(host_port("http://user:secret@1.2.3.4:8000"), "1.2.3.4:8000");
        assert_eq!(host_port("socks5://a:b@h.example:1080/"), "h.example:1080");
        assert_eq!(mask_proxy_value("http://u:p@corp:3128"), "http://corp:3128");
        assert_eq!(mask_proxy_value("127.0.0.1:8889"), "127.0.0.1:8889");
    }

    /// Живой отчёт (только чтение): cargo test live_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_report() {
        println!("{}", report());
    }
}
