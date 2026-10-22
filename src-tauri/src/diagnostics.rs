//! «Отчёт для поддержки»: всё, что нужно, чтобы понять проблему человека
//! удалённо, — одним текстом, который он пересылает разработчику.
//!
//! Без секретов: адреса прокси — только host:port (логин и пароль никогда не
//! попадают в отчёт), чужие прокси-переменные — тоже без учётных данных.

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

pub fn report() -> String {
    let mut r = String::new();
    let s = crate::settings::load_with_presets();
    let _ = writeln!(r, "=== Clod Helper — отчёт для поддержки ===");
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
                        let _ = writeln!(r, "    [{}с] {}", e["t"], e["msg"].as_str().unwrap_or(""));
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
        let label = s.proxy_labels.get(u).map(|l| format!(" «{l}»")).unwrap_or_default();
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
                .map(|(k, v)| format!("{k}={}", mask_proxy_value(v)))
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
        pac.foreign.map(|f| format!(" чужая настройка: {}", mask_proxy_value(&f))).unwrap_or_default()
    );
    if let Ok(c) = crate::system_pac::read() {
        let _ = writeln!(
            r,
            "Системный прокси Windows: флаги={} PAC={} прокси={}",
            c.flags,
            c.pac_url.as_deref().map(|u| u.split('?').next().unwrap_or(u).to_string()).unwrap_or_else(|| "—".into()),
            c.server.as_deref().map(mask_proxy_value).unwrap_or_else(|| "—".into()),
        );
    }
    for site in &s.sites {
        let _ = writeln!(
            r,
            "{} {}: {} (прокси: {})",
            if site.enabled { "✓" } else { "✗" },
            site.name,
            site.domains.join(", "),
            site.proxy.map(|i| format!("#{i}")).unwrap_or_else(|| "авто".into())
        );
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
                        if st.configured { st.server.clone() } else { "нет".into() },
                        st.conflict.map(|c| format!(" (конфликт: {c})")).unwrap_or_default()
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
