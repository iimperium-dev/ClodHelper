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
        let id = proxied.get("https://bash.ws/id").send().await.ok()?.text().await.ok()?;
        let id = id.trim().to_string();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        // Имена резолвит сторона прокси — так bash.ws видит её DNS.
        let pings = (1..=4).map(|i| proxied.get(format!("http://{i}.{id}.bash.ws")).send());
        let _ = futures_join_all(pings).await;
        let list: Vec<BashWsEntry> = proxied
            .get(format!("https://bash.ws/dnsleak/test/{id}?json"))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        Some(list)
    };
    let (home, list) = tokio::join!(home_fut, dns_fut);
    r.home = home;
    match list {
        Some(list) => {
            for e in list {
                let info = IpInfo { ip: e.ip, country: e.country_name, org: e.org };
                match e.kind.as_str() {
                    "ip" => r.exit = Some(info),
                    "dns" => r.dns.push(info),
                    _ => {}
                }
            }
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

/// Простой join_all без зависимости от futures.
async fn futures_join_all<F: std::future::Future>(futs: impl Iterator<Item = F>) -> Vec<F::Output> {
    let handles: Vec<_> = futs.collect();
    let mut out = Vec::with_capacity(handles.len());
    for f in handles {
        out.push(f.await);
    }
    out
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

    /// Живая проверка: cargo test live_privacy -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_privacy() {
        println!("{:#?}", check(Vec::new()).await);
    }
}
