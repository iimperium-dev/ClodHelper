//! Через мост ли на самом деле идёт приложение — по живым TCP-соединениям.
//!
//! Настройка «записана» ещё не значит «работает»: приложение могли открыть
//! мимо ярлыка, политику мог стереть macOS. Поэтому статус в «Приложениях»
//! считается по факту: один снимок `lsof` на все процессы, соединения
//! раскладываются по приложениям по имени процесса.
//!   - к 127.0.0.1:8889/8890/8891 — через мост;
//!   - наружу (не loopback) — мимо моста.

use serde::Serialize;

use crate::apps::AppProfile;

/// Входы моста: общий HTTP 8889, строгий Claude 8890, SOCKS 8891 и
/// закреплённые за прокси пула 8892–8896.
const BRIDGE_PORTS: std::ops::RangeInclusive<u16> = 8889..=8896;

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
pub struct AppTraffic {
    pub id: String,
    pub running: bool,
    /// Соединений через мост прямо сейчас.
    pub via_bridge: u32,
    /// Соединений наружу мимо моста.
    pub direct: u32,
    /// Браузер: включённое расширение, перехватившее прокси (FoxyProxy…).
    pub proxy_extension: Option<crate::browser_ext::ProxyExtension>,
}

/// Имена процессов, по которым узнаём приложение (префикс: «Claude» ловит и
/// «Claude Helper (Renderer)»). Общие рантаймы (node, git) не берём — они
/// принадлежат кому угодно.
fn traffic_names(p: &AppProfile) -> Vec<String> {
    match p.id.as_str() {
        "claude-desktop" => vec!["Claude".into()],
        "vscode" => vec!["Code".into(), "Electron".into()],
        // На Mac Codex живёт внутри ChatGPT.app.
        "codex" => vec!["ChatGPT".into(), "Codex".into()],
        "telegram" => vec!["Telegram".into()],
        _ => {
            // Своё приложение: имя бандла («WebStorm.app» → «WebStorm»).
            // Своё приложение: имя бандла/файла («WebStorm.app» → «WebStorm»,
            // «C:\\…\\chrome.exe» → «chrome») — разбор без учёта ОС.
            let from_path = p.exe_path.as_deref().and_then(|e| {
                let leaf = e.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next()?;
                let stem = leaf.strip_suffix(".app").or_else(|| leaf.strip_suffix(".exe")).unwrap_or(leaf);
                (!stem.is_empty()).then(|| stem.to_string())
            });
            from_path.into_iter().collect()
        }
    }
}

/// Имя процесса совпадает с именем приложения или начинается с него и
/// пробела/скобки («Claude Helper (Renderer)»). Без учёта регистра: на
/// Windows процессы — «claude.exe», «chrome.exe».
fn matches(command: &str, names: &[String]) -> bool {
    let c = command.to_ascii_lowercase();
    let c = c.strip_suffix(".exe").unwrap_or(&c);
    names.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        c == n || c.strip_prefix(n.as_str()).is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('('))
    })
}

/// Разобрать `lsof -Fcn`: «c<команда>» задаёт процесс, «n<лок>-><удал>» —
/// его соединения. Возвращает (команда, удалённый адрес).
fn parse_lsof(out: &str) -> Vec<(String, String)> {
    let mut cmd = String::new();
    let mut conns = Vec::new();
    for line in out.lines() {
        if let Some(c) = line.strip_prefix('c') {
            cmd = c.to_string();
        } else if let Some(n) = line.strip_prefix('n') {
            if let Some((_, remote)) = n.split_once("->") {
                conns.push((cmd.clone(), remote.to_string()));
            }
        }
    }
    conns
}

fn is_bridge(remote: &str) -> bool {
    let port = |host: &str| {
        remote
            .strip_prefix(host)
            .and_then(|p| p.parse::<u16>().ok())
            .is_some_and(|p| BRIDGE_PORTS.contains(&p))
    };
    port("127.0.0.1:") || port("[::1]:")
}

/// Дата-центры Telegram (официальные диапазоны MTProto). Для Telegram
/// «мимо моста» считаем ТОЛЬКО их: сам клиент напрямую проверяет
/// доступность сохранённых в нём прокси и ходит к служебным адресам —
/// это не переписка, и тревога «мимо моста» из-за них была ложной.
fn is_telegram_dc(remote: &str) -> bool {
    if let Some(v6) = remote.strip_prefix('[') {
        return ["2001:b28:f23d:", "2001:b28:f23f:", "2001:67c:4e8:", "2a0a:f280:"]
            .iter()
            .any(|p| v6.starts_with(p));
    }
    let Some(ip) = remote
        .rsplit_once(':')
        .and_then(|(h, _)| h.parse::<std::net::Ipv4Addr>().ok())
    else {
        return false;
    };
    let ip = u32::from(ip);
    let in_net = |net: [u8; 4], bits: u32| {
        let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
        ip & mask == u32::from(std::net::Ipv4Addr::from(net)) & mask
    };
    in_net([149, 154, 160, 0], 20)
        || in_net([91, 108, 0, 0], 16)
        || in_net([91, 105, 192, 0], 23)
        || in_net([95, 161, 64, 0], 20)
        || in_net([185, 76, 151, 0], 24)
}

/// Публичные DNS-резолверы (DNS-over-HTTPS / DNS-over-TLS). Движок Chromium
/// в Electron-приложениях (ChatGPT, VS Code…) ходит к ним напрямую, мимо
/// --proxy-server, — это поиск адресов, а не трафик к сервису: ни запросы,
/// ни IP пользователя до OpenAI/Anthropic так не доходят. Считать их «мимо
/// моста» значило бы вечное ложное «Частично мимо».
fn is_dns_resolver(remote: &str) -> bool {
    const RESOLVERS: &[&str] = &[
        // Cloudflare (chrome./mozilla.cloudflare-dns.com, 1.1.1.1)
        "172.64.41.3", "172.64.41.4", "162.159.61.3", "162.159.61.4",
        "104.16.248.249", "104.16.249.249", "1.1.1.1", "1.0.0.1",
        // Google (dns.google), Quad9, AdGuard, Яндекс DNS
        "8.8.8.8", "8.8.4.4", "9.9.9.9", "149.112.112.112",
        "94.140.14.14", "94.140.15.15", "77.88.8.8", "77.88.8.1",
    ];
    let Some((host, port)) = remote.rsplit_once(':') else {
        return false;
    };
    matches!(port, "443" | "853") && RESOLVERS.contains(&host)
}

fn is_loopback(remote: &str) -> bool {
    remote.starts_with("127.") || remote.starts_with("[::1]")
}

/// Свести снимок процессов и соединений к статусу каждого профиля.
fn summarize(
    profiles: &[AppProfile],
    running_cmds: &[String],
    conns: &[(String, String)],
) -> Vec<AppTraffic> {
    profiles
        .iter()
        .map(|p| {
            let names = traffic_names(p);
            let mut t = AppTraffic {
                id: p.id.clone(),
                running: !names.is_empty() && running_cmds.iter().any(|c| matches(c, &names)),
                ..Default::default()
            };
            let telegram = p.id == "telegram";
            for (cmd, remote) in conns {
                if !matches(cmd, &names) {
                    continue;
                }
                if is_bridge(remote) {
                    t.via_bridge += 1;
                } else if !is_loopback(remote)
                    && !is_dns_resolver(remote)
                    && (!telegram || is_telegram_dc(remote))
                {
                    t.direct += 1;
                }
            }
            t
        })
        .collect()
}

/// Включена ли у браузера политика «всегда через мост» (кэш 30 с: статус —
/// это запуск скрипта).
#[cfg(not(target_os = "windows"))]
fn browser_policy_on(app_path: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<HashMap<String, (Instant, bool)>>> = Mutex::new(None);
    let Some(domain) = crate::browser_policy::policy_domain(app_path) else {
        return false;
    };
    if let Ok(mut c) = CACHE.lock() {
        let map = c.get_or_insert_with(HashMap::new);
        if let Some((t, v)) = map.get(domain) {
            if t.elapsed() < Duration::from_secs(30) {
                return *v;
            }
        }
        let on = crate::browser_policy::status(domain).is_ok_and(|s| s.configured);
        map.insert(domain.to_string(), (Instant::now(), on));
        return on;
    }
    false
}

/// Снимок не чаще раза в 3 с: страница «Приложения» и окно настроек могут
/// спросить почти одновременно, а lsof по всем процессам — не бесплатный.
#[cfg(not(target_os = "windows"))]
pub fn snapshot(profiles: &[AppProfile]) -> Vec<AppTraffic> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Vec<String>, Vec<AppTraffic>)>> = Mutex::new(None);
    let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
    if let Ok(c) = CACHE.lock() {
        if let Some((t, k, v)) = c.as_ref() {
            if t.elapsed() < Duration::from_secs(3) && *k == ids {
                return v.clone();
            }
        }
    }
    let fresh = snapshot_uncached(profiles);
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), ids, fresh.clone()));
    }
    fresh
}

#[cfg(not(target_os = "windows"))]
fn snapshot_uncached(profiles: &[AppProfile]) -> Vec<AppTraffic> {
    use std::process::Command;
    // -c: только имя процесса (без пути), как его показывает lsof.
    let running: Vec<String> = Command::new("/bin/ps")
        .args(["-axc", "-o", "comm="])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    // +c 0: полное имя команды (по умолчанию lsof режет до 9 символов).
    let out = Command::new("/usr/sbin/lsof")
        .args(["+c", "0", "-nP", "-iTCP", "-sTCP:ESTABLISHED", "-Fcn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut t = summarize(profiles, &running, &parse_lsof(&out));
    for (traffic, p) in t.iter_mut().zip(profiles) {
        if let Some(path) = p.exe_path.as_deref() {
            // Политика «всегда через мост» главнее расширений — тогда
            // FoxyProxy и т.п. уже ничего не перехватывают.
            if !browser_policy_on(path) {
                traffic.proxy_extension = crate::browser_ext::proxy_extensions(path).into_iter().next();
            }
        }
    }
    t
}

/// Windows: те же правила, источник — `netstat -ano` (соединения с PID) и
/// `tasklist` (PID → имя образа). Кэш 3 с, как на macOS.
#[cfg(target_os = "windows")]
pub fn snapshot(profiles: &[AppProfile]) -> Vec<AppTraffic> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    static CACHE: Mutex<Option<(Instant, Vec<String>, Vec<AppTraffic>)>> = Mutex::new(None);
    let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
    if let Ok(c) = CACHE.lock() {
        if let Some((t, k, v)) = c.as_ref() {
            if t.elapsed() < Duration::from_secs(3) && *k == ids {
                return v.clone();
            }
        }
    }
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    };
    let names = parse_tasklist(&run("tasklist", &["/FO", "CSV", "/NH"]));
    let running: Vec<String> = names.values().cloned().collect();
    let conns: Vec<(String, String)> = parse_netstat(&run("netstat", &["-ano", "-p", "TCP"]))
        .into_iter()
        .filter_map(|(pid, remote)| names.get(&pid).map(|n| (n.clone(), remote)))
        .collect();
    let fresh = summarize(profiles, &running, &conns);
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), ids, fresh.clone()));
    }
    fresh
}

/// `tasklist /FO CSV /NH` → PID → имя образа.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_tasklist(out: &str) -> std::collections::HashMap<u32, String> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split("\",\"");
            let name = f.next()?.trim_start_matches('"').to_string();
            let pid = f.next()?.trim_matches('"').parse().ok()?;
            Some((pid, name))
        })
        .collect()
}

/// `netstat -ano -p TCP` → (PID, удалённый адрес) для ESTABLISHED.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_netstat(out: &str) -> Vec<(u32, String)> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 5 && f[0] == "TCP" && f[3] == "ESTABLISHED")
                .then(|| Some((f[4].parse().ok()?, f[2].to_string())))
                .flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::LaunchKind;

    fn profile(id: &str, exe: Option<&str>) -> AppProfile {
        AppProfile {
            id: id.into(),
            name: id.into(),
            kind: LaunchKind::ExeFlag,
            exe_path: exe.map(String::from),
            app_id: None,
            process_names: Vec::new(),
            enabled: false,
            builtin: true,
            hidden: false,
            proxy: None,
            full_proxy: None,
        }
    }

    #[test]
    fn dns_over_https_is_not_a_leak() {
        let profiles = vec![profile("codex", None)];
        let lsof = "p1\ncCodex (Service)\nf1\nn127.0.0.1:5000->127.0.0.1:8889\n\
                    f2\nn192.168.0.5:5001->172.64.41.3:443\n\
                    f3\nn192.168.0.5:5002->162.159.61.3:443\n";
        let t = summarize(&profiles, &["Codex (Service)".to_string()], &parse_lsof(lsof));
        assert_eq!((t[0].via_bridge, t[0].direct), (1, 0));
        assert!(!is_dns_resolver("172.64.41.3:80"));
        assert!(!is_dns_resolver("104.18.32.47:443"));
    }

    #[test]
    fn windows_netstat_and_tasklist() {
        let tl = "\"Claude.exe\",\"4120\",\"Console\",\"1\",\"150 000 K\"\r\n\"chrome.exe\",\"77\",\"Console\",\"1\",\"9 K\"\r\n";
        let ns = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  \
                  TCP    127.0.0.1:50001        127.0.0.1:8890         ESTABLISHED     4120\r\n  \
                  TCP    192.168.0.5:50002      142.250.1.1:443        ESTABLISHED     77\r\n  \
                  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       900\r\n";
        let names = parse_tasklist(tl);
        assert_eq!(names.get(&4120).map(String::as_str), Some("Claude.exe"));
        let conns: Vec<(String, String)> = parse_netstat(ns)
            .into_iter()
            .filter_map(|(pid, r)| names.get(&pid).map(|n| (n.clone(), r)))
            .collect();
        let profiles = vec![profile("claude-desktop", None), profile("custom-1", Some("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"))];
        let running: Vec<String> = names.values().cloned().collect();
        let t = summarize(&profiles, &running, &conns);
        assert_eq!((t[0].running, t[0].via_bridge, t[0].direct), (true, 1, 0));
        assert_eq!((t[1].running, t[1].via_bridge, t[1].direct), (true, 0, 1));
    }

    #[test]
    fn telegram_dc_ranges() {
        assert!(is_telegram_dc("149.154.167.51:443"));
        assert!(is_telegram_dc("91.108.56.130:443"));
        assert!(is_telegram_dc("[2001:b28:f23d:f001::a]:443"));
        assert!(!is_telegram_dc("194.221.250.50:443"));
        assert!(!is_telegram_dc("17.57.146.20:5223"));
    }

    #[test]
    fn splits_bridge_and_direct_per_app() {
        let lsof = "p1\ncClaude Helper\nf20\nn127.0.0.1:5000->127.0.0.1:8890\n\
                    f21\nn192.168.1.5:5001->160.79.104.10:443\n\
                    p2\ncTelegram\nf9\nn127.0.0.1:5002->127.0.0.1:8891\n\
                    f10\nn198.18.0.1:5006->194.221.250.50:443\n\
                    p3\ncClaudeBar\nf3\nn10.0.0.2:5003->1.1.1.1:443\n\
                    p4\ncCode Helper (Plugin)\nf4\nn127.0.0.1:5004->127.0.0.1:8889\n\
                    f5\nn127.0.0.1:5005->127.0.0.1:3000\n";
        let profiles = vec![
            profile("claude-desktop", None),
            profile("telegram", None),
            profile("vscode", None),
            profile("custom-1", Some("/Applications/WebStorm.app")),
        ];
        let running: Vec<String> = ["Claude", "Telegram", "Code Helper (Plugin)", "ClaudeBar"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let t = summarize(&profiles, &running, &parse_lsof(lsof));
        // ClaudeBar — другое приложение: префикс без пробела не считается.
        assert_eq!((t[0].running, t[0].via_bridge, t[0].direct), (true, 1, 1));
        assert_eq!((t[1].via_bridge, t[1].direct), (1, 0));
        // Локальный dev-сервер (127.0.0.1:3000) — не «мимо моста».
        assert_eq!((t[2].running, t[2].via_bridge, t[2].direct), (true, 1, 0));
        assert_eq!((t[3].running, t[3].via_bridge, t[3].direct), (false, 0, 0));
    }
}
