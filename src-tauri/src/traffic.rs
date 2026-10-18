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
            let from_path = p.exe_path.as_deref().and_then(|e| {
                std::path::Path::new(e.trim_end_matches('/'))
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            });
            from_path.into_iter().collect()
        }
    }
}

fn matches(command: &str, names: &[String]) -> bool {
    names.iter().any(|n| {
        command == n
            || command
                .strip_prefix(n.as_str())
                .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('('))
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
                } else if !is_loopback(remote) && (!telegram || is_telegram_dc(remote)) {
                    t.direct += 1;
                }
            }
            t
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
pub fn snapshot(profiles: &[AppProfile]) -> Vec<AppTraffic> {
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
    summarize(profiles, &running, &parse_lsof(&out))
}

/// Windows: статус по соединениям пока не считаем (там есть killswitch).
#[cfg(target_os = "windows")]
pub fn snapshot(_profiles: &[AppProfile]) -> Vec<AppTraffic> {
    Vec::new()
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
