//! «Проверка приватности» одной кнопкой: то же, что ipleak, но с вердиктом.
//!
//!  - выход: внешний IP через строгий вход моста (что видят AI-сервисы);
//!  - дом: внешний IP напрямую (настоящий, от провайдера) — для сравнения;
//!  - DNS: какие резолверы видит сайт (bash.ws dnsleak через мост): утечка,
//!    если среди них DNS вашего провайдера;
//!  - приложения: кто сейчас идёт мимо моста (статус по соединениям) и
//!    закреплены ли браузеры политикой.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct IpInfo {
    pub ip: String,
    pub country: String,
    pub org: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PrivacyReport {
    /// IP, который видят сервисы (через мост).
    pub exit: Option<IpInfo>,
    /// Настоящий IP (напрямую). None — прямой выход недоступен.
    pub home: Option<IpInfo>,
    /// DNS-резолверы, видимые сайтом.
    pub dns: Vec<IpInfo>,
    /// Настоящий IP скрыт (выход ≠ дом).
    pub ip_hidden: bool,
    /// Среди DNS есть резолвер вашего провайдера.
    pub dns_leak: bool,
    /// Приложения, которые сейчас идут мимо моста (или чей прокси перехвачен).
    pub leaking_apps: Vec<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct BashWsEntry {
    ip: String,
    #[serde(default)]
    country_name: String,
    #[serde(default)]
    org: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct IpleakJson {
    ip: String,
    #[serde(default)]
    country_name: Option<String>,
    #[serde(default)]
    isp_name: Option<String>,
}

/// Утечка DNS: резолвер принадлежит вашему провайдеру (совпадает сеть) или
/// это сам ваш IP. Публичные DNS (Google, Cloudflare) провайдера не выдают.
pub fn dns_leaks(dns: &[IpInfo], home: Option<&IpInfo>) -> bool {
    let Some(home) = home else { return false };
    let norm = |s: &str| s.to_lowercase().replace([',', '.'], "");
    let home_org = norm(&home.org);
    dns.iter().any(|d| {
        d.ip == home.ip || (!home_org.is_empty() && !d.org.is_empty() && norm(&d.org) == home_org)
    })
}

pub async fn check(leaking_apps: Vec<String>) -> PrivacyReport {
    let mut r = PrivacyReport { leaking_apps, ..Default::default() };
    let proxied = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(crate::claude_policy::PROXY_URL).expect("valid proxy url"))
        .timeout(std::time::Duration::from_secs(15))
        .build();
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(6))
        .build();
    let (Ok(proxied), Ok(direct)) = (proxied, direct) else {
        r.error = Some("Не удалось подготовить проверку".into());
        return r;
    };

    // Настоящий IP — параллельно с DNS-тестом.
    let home_fut = async {
        let j: IpleakJson = direct.get("https://ipleak.net/json/").send().await.ok()?.json().await.ok()?;
        Some(IpInfo {
            ip: j.ip,
            country: j.country_name.unwrap_or_default(),
            org: j.isp_name.unwrap_or_default(),
        })
    };
    let dns_fut = async {
        // bash.ws — основной источник (как раньше). Если он не отдал id
        // (с ~ноября 2026 `/id` отдаёт индонезийскую версию сайта вместо id),
        // переходим на DNS-детектор ipleak.net.
        match bashws_dns(&proxied).await {
            Some(res) => Some(res),
            None => ipleak_dns(&proxied).await,
        }
    };
    let (home, dns_res) = tokio::join!(home_fut, dns_fut);
    r.home = home;
    match dns_res {
        Some((exit, dns)) => {
            r.exit = exit;
            r.dns = dns;
        }
        None => r.error = Some("Сервис проверки недоступен через прокси".into()),
    }
    r.ip_hidden = match (&r.exit, &r.home) {
        (Some(e), Some(h)) => e.ip != h.ip,
        (Some(_), None) => true,
        _ => false,
    };
    r.dns_leak = dns_leaks(&r.dns, r.home.as_ref());
    r
}

/// Выход и DNS-резолверы, как их видит сайт проверки.
type DnsResult = (Option<IpInfo>, Vec<IpInfo>);

/// DNS-тест bash.ws: id → пинги поддоменов → JSON с выходом и резолверами.
async fn bashws_dns(proxied: &reqwest::Client) -> Option<DnsResult> {
    let id = proxied.get("https://bash.ws/id").send().await.ok()?.text().await.ok()?;
    let id = id.trim().to_string();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    // Имена резолвит сторона прокси — так bash.ws видит её DNS. Все
    // четыре разом: по очереди зависший прокси растягивал проверку на минуту.
    let pings: Vec<_> = (1..=4)
        .map(|i| {
            let req = proxied
                .get(format!("http://{i}.{id}.bash.ws"))
                .timeout(std::time::Duration::from_secs(6));
            tokio::spawn(req.send())
        })
        .collect();
    for ping in pings {
        let _ = ping.await;
    }
    let list: Vec<BashWsEntry> = proxied
        .get(format!("https://bash.ws/dnsleak/test/{id}?json"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let mut exit = None;
    let mut dns = Vec::new();
    for e in list {
        let info = IpInfo { ip: e.ip, country: e.country_name, org: e.org };
        match e.kind.as_str() {
            "ip" => exit = Some(info),
            "dns" => dns.push(info),
            _ => {}
        }
    }
    Some((exit, dns))
}

/// Ответ `https://<40 hex>.ipleak.net/dnsdetection/`:
/// `{"session": "...", "ip": {"172.217.44.22": 1}}` — резолверы, которые
/// спросили авторитетный DNS ipleak об этом случайном имени.
#[derive(Deserialize)]
struct IpleakDnsDetection {
    #[serde(default)]
    ip: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Случайная метка из 40 hex-символов — формат, который ждёт ipleak.
fn random_label() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(48);
    for i in 0..3u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out.truncate(40);
    out
}

/// IP резолверов из ответа dnsdetection (только валидные IP, без дублей).
fn parse_dnsdetection(body: &str) -> Option<Vec<String>> {
    let j: IpleakDnsDetection = serde_json::from_str(body).ok()?;
    Some(
        j.ip.into_keys()
            .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
            .collect(),
    )
}

/// Запасной DNS-тест через ipleak.net: несколько случайных имён (каждое
/// резолвит сторона прокси), плюс выходной IP и провайдер каждого резолвера.
async fn ipleak_dns(proxied: &reqwest::Client) -> Option<DnsResult> {
    let short = std::time::Duration::from_secs(6);
    let probes: Vec<_> = (0..4)
        .map(|_| {
            let req = proxied
                .get(format!("https://{}.ipleak.net/dnsdetection/", random_label()))
                .timeout(short);
            tokio::spawn(async move { req.send().await.ok()?.text().await.ok() })
        })
        .collect();
    let exit_fut = async {
        let j: IpleakJson = proxied.get("https://ipleak.net/json/").send().await.ok()?.json().await.ok()?;
        Some(IpInfo {
            ip: j.ip,
            country: j.country_name.unwrap_or_default(),
            org: j.isp_name.unwrap_or_default(),
        })
    };
    let probes_fut = async {
        let mut ips: Vec<String> = Vec::new();
        let mut answered = false;
        for p in probes {
            if let Ok(Some(body)) = p.await {
                if let Some(list) = parse_dnsdetection(&body) {
                    answered = true;
                    for ip in list {
                        if !ips.contains(&ip) {
                            ips.push(ip);
                        }
                    }
                }
            }
        }
        answered.then_some(ips)
    };
    let (exit, ips) = tokio::join!(exit_fut, probes_fut);
    let ips = ips?;
    // Провайдер резолвера нужен для вердикта об утечке; не узнали — пусто.
    let lookups: Vec<_> = ips
        .into_iter()
        .map(|ip| {
            let req = proxied.get(format!("https://ipleak.net/json/{ip}")).timeout(short);
            tokio::spawn(async move {
                let j: Option<IpleakJson> = async { req.send().await.ok()?.json().await.ok() }.await;
                match j {
                    Some(j) => IpInfo {
                        ip,
                        country: j.country_name.unwrap_or_default(),
                        org: j.isp_name.unwrap_or_default(),
                    },
                    None => IpInfo { ip, country: String::new(), org: String::new() },
                }
            })
        })
        .collect();
    let mut dns = Vec::new();
    for l in lookups {
        if let Ok(info) = l.await {
            dns.push(info);
        }
    }
    Some((exit, dns))
}

/// Проверка целиком не дольше `limit`: «Проверить» не должна крутиться
/// бесконечно, даже если прокси принимает соединения и молчит.
pub async fn check_within(leaking_apps: Vec<String>, limit: std::time::Duration) -> PrivacyReport {
    match tokio::time::timeout(limit, check(leaking_apps.clone())).await {
        Ok(report) => report,
        Err(_) => PrivacyReport {
            leaking_apps,
            error: Some(format!(
                "Проверка не уложилась в {} с — прокси отвечает слишком медленно",
                limit.as_secs()
            )),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(ip: &str, org: &str) -> IpInfo {
        IpInfo { ip: ip.into(), country: String::new(), org: org.into() }
    }

    #[test]
    fn dns_leak_detection() {
        let home = ip("95.174.127.216", "Consumer Internet Cooperative PG-19");
        assert!(!dns_leaks(&[ip("172.253.11.17", "Google LLC")], Some(&home)));
        assert!(dns_leaks(&[ip("95.174.0.1", "Consumer Internet Cooperative PG-19")], Some(&home)));
        assert!(dns_leaks(&[ip("95.174.127.216", "")], Some(&home)));
        assert!(!dns_leaks(&[ip("1.1.1.1", "")], None));
    }

    #[test]
    fn ipleak_dnsdetection_parsing() {
        let body = r#"{"session":"bcb41042ddf5454db05b5b97ec656da6e1e9e236","ip":{"172.217.44.22":1,"8.8.4.4":3,"junk":1}}"#;
        let mut ips = parse_dnsdetection(body).unwrap();
        ips.sort();
        assert_eq!(ips, vec!["172.217.44.22".to_string(), "8.8.4.4".to_string()]);
        assert_eq!(parse_dnsdetection(r#"{"session":"x","ip":{}}"#), Some(vec![]));
        assert_eq!(parse_dnsdetection("<!DOCTYPE HTML><html lang=\"id\">"), None);
    }

    #[test]
    fn random_label_is_40_hex() {
        let a = random_label();
        assert_eq!(a.len(), 40);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_label());
    }

    /// Живая проверка: cargo test live_privacy -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_privacy() {
        println!("{:#?}", check(Vec::new()).await);
    }
}
