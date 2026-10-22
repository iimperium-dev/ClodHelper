//! Центр уведомлений: обновления и рассылки разработчика.
//!
//! Рассылка без своего сервера: файл `notices.json` в репозитории (ветка
//! main). Приложение скачивает его при запуске и раз в несколько часов
//! (update_notify::spawn_watcher); новое — всплывает в углу экрана со звуком
//! и ложится в колокольчик. Кто был офлайн — увидит при следующем запуске:
//! список проверяется сразу при старте и кэшируется в настройках (виден и
//! без сети). Прочитанное помнится на этом компьютере.
//!
//! Формат файла — см. notices.json в корне репозитория.

use serde::{Deserialize, Serialize};

const URL: &str = "https://raw.githubusercontent.com/iimperium-dev/ClodHelper/main/notices.json";

/// Запись рассылки (как в notices.json).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// Уникальный и неизменный: по нему помнится «прочитано».
    pub id: String,
    /// YYYY-MM-DD — для сортировки и подписи.
    pub date: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Кнопка «Подробнее» — только https.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// "windows" / "macos"; пусто — всем.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// Только для версий в диапазоне (включительно).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_version: Option<String>,
}

#[derive(Deserialize)]
struct NoticesFile {
    #[serde(default)]
    notices: Vec<Notice>,
}

/// Элемент колокольчика для UI.
#[derive(Debug, Clone, Serialize)]
pub struct NoticeItem {
    pub id: String,
    /// "update" | "news".
    pub kind: &'static str,
    pub date: String,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    pub read: bool,
}

/// Подходит ли запись этой установке: платформа, версия, безопасная ссылка.
fn applies(n: &Notice, current: &str) -> bool {
    let os = if cfg!(target_os = "windows") { "windows" } else { "macos" };
    if !n.platforms.is_empty() && !n.platforms.iter().any(|p| p.eq_ignore_ascii_case(os)) {
        return false;
    }
    let cur = crate::updater::parse_ver(current);
    let cmp = |v: &Option<String>| v.as_deref().and_then(crate::updater::parse_ver);
    if let (Some(min), Some(c)) = (cmp(&n.min_version), cur) {
        if c < min {
            return false;
        }
    }
    if let (Some(max), Some(c)) = (cmp(&n.max_version), cur) {
        if c > max {
            return false;
        }
    }
    !n.id.trim().is_empty() && !n.title.trim().is_empty()
}

fn parse(text: &str, current: &str) -> Result<Vec<Notice>, String> {
    let f: NoticesFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("notices.json: {e}"))?;
    let mut v: Vec<Notice> = f
        .notices
        .into_iter()
        .filter(|n| applies(n, current))
        .map(|mut n| {
            // Ссылка только https — никаких file:/javascript: из файла.
            n.link = n.link.filter(|l| l.starts_with("https://"));
            n
        })
        .collect();
    v.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.id.cmp(&b.id)));
    v.truncate(50);
    Ok(v)
}

/// Скачать рассылки и обновить кэш. Возвращает новые (ещё не всплывавшие).
pub async fn fetch() -> Result<Vec<Notice>, String> {
    // Только отладочная сборка: рассылка из локального файла — проверить
    // колокольчик, не отправляя ничего настоящим пользователям.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CLOD_NOTICES_FILE") {
        let text = std::fs::read_to_string(path).map_err(|e| format!("CLOD_NOTICES_FILE: {e}"))?;
        let s = crate::settings::set_notices_cache(parse(&text, env!("CARGO_PKG_VERSION"))?)?;
        return Ok(s.notices_cache.iter().filter(|n| !s.notices_toasted.contains(&n.id)).cloned().collect());
    }
    // raw.githubusercontent.com у части провайдеров в России заблокирован:
    // не достали напрямую — берём через свой прокси (строгий вход моста).
    let text = match download(None).await {
        Ok(t) => t,
        Err(direct) => download(Some(&format!("http://127.0.0.1:{}", crate::claude_desktop::PROXY_PORT)))
            .await
            .map_err(|via| format!("рассылки: напрямую — {direct}; через мост — {via}"))?,
    };
    // Файла нет (404) — рассылок нет, это нормально.
    let list = match text {
        Some(t) => parse(&t, env!("CARGO_PKG_VERSION"))?,
        None => Vec::new(),
    };
    let s = crate::settings::set_notices_cache(list)?;
    Ok(s.notices_cache.iter().filter(|n| !s.notices_toasted.contains(&n.id)).cloned().collect())
}

/// Скачать notices.json (через прокси, если задан). None — файла нет (404).
async fn download(proxy: Option<&str>) -> Result<Option<String>, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("ClodHelper/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15));
    if let Some(p) = proxy {
        b = b.proxy(reqwest::Proxy::all(p).map_err(|e| e.to_string())?);
    }
    let resp = b.build().map_err(|e| e.to_string())?.get(URL).send().await.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    resp.error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Сегодняшняя дата UTC «YYYY-MM-DD» (как в notices.json), минус `days_ago`.
fn date_days_ago(days_ago: u64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil-from-days (H. Hinnant), эпоха 1970-01-01.
    let z = (secs / 86_400).saturating_sub(days_ago) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Всплывать в углу экрана — только свежим рассылкам: тот, кто поставил
/// программу через месяц, не должен получать старые новости всплывающим
/// окном (в колокольчике они остаются).
pub fn worth_toast(n: &Notice) -> bool {
    n.date.as_str() >= date_days_ago(7).as_str()
}

/// id элемента «вышло обновление» — свой на каждую версию.
pub fn update_id(version: &str) -> String {
    format!("update-{version}")
}

/// Всё для колокольчика: обновление (если есть) + рассылки, новые сверху.
pub fn list() -> Vec<NoticeItem> {
    let s = crate::settings::load();
    let mut items: Vec<NoticeItem> = Vec::new();
    if let Some(u) = crate::update_notify::latest() {
        let id = update_id(&u.version);
        items.push(NoticeItem {
            read: s.notices_read.contains(&id),
            id,
            kind: "update",
            date: String::new(),
            title: format!("Доступна версия {}", u.version),
            body: u.highlights.first().cloned().unwrap_or_else(|| "Исправления и улучшения".into()),
            link: None,
        });
    }
    for n in &s.notices_cache {
        items.push(NoticeItem {
            id: n.id.clone(),
            kind: "news",
            date: n.date.clone(),
            title: n.title.clone(),
            body: n.body.clone(),
            link: n.link.clone(),
            read: s.notices_read.contains(&n.id),
        });
    }
    items
}

pub fn unread() -> usize {
    list().iter().filter(|i| !i.read).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_platform_version_and_unsafe_links() {
        let text = r#"{"notices":[
            {"id":"a","date":"2026-10-20","title":"Всем","body":"x","link":"javascript:alert(1)"},
            {"id":"b","date":"2026-10-22","title":"Новее","link":"https://t.me/x"},
            {"id":"mac","date":"2026-10-21","title":"Только Mac","platforms":["macos"]},
            {"id":"win","date":"2026-10-21","title":"Только Windows","platforms":["Windows"]},
            {"id":"old","date":"2026-10-21","title":"Старым","max_version":"0.7.9"},
            {"id":"new","date":"2026-10-21","title":"Новым","min_version":"0.9.0"},
            {"id":"","date":"2026-10-21","title":"Без id"}
        ]}"#;
        let v = parse(text, "0.8.3").unwrap();
        let ids: Vec<&str> = v.iter().map(|n| n.id.as_str()).collect();
        let os_specific = if cfg!(target_os = "windows") { "win" } else { "mac" };
        assert_eq!(ids, vec!["b", os_specific, "a"], "новые сверху");
        assert_eq!(v.iter().find(|n| n.id == "a").unwrap().link, None, "не-https ссылку выкидываем");
        assert_eq!(v[0].link.as_deref(), Some("https://t.me/x"));
        assert!(parse("{}", "0.8.3").unwrap().is_empty());
        assert!(parse("{bad", "0.8.3").is_err());
    }

    #[test]
    fn only_recent_notices_pop_up() {
        let n = |date: &str| Notice {
            id: "x".into(),
            date: date.into(),
            title: "t".into(),
            body: String::new(),
            link: None,
            platforms: vec![],
            min_version: None,
            max_version: None,
        };
        let today = date_days_ago(0);
        assert_eq!(today.len(), 10);
        assert!(worth_toast(&n(&today)));
        assert!(worth_toast(&n(&date_days_ago(7))));
        assert!(!worth_toast(&n(&date_days_ago(8))));
        assert!(!worth_toast(&n("2020-01-01")));
        assert!(date_days_ago(1) < today);
    }

    /// Файл рассылок в репозитории читается тем же разбором, что в программе.
    #[test]
    fn repo_notices_file_is_valid() {
        let text = include_str!("../../notices.json");
        let all: NoticesFile = serde_json::from_str(text).expect("notices.json — не JSON");
        let mut ids: Vec<&str> = all.notices.iter().map(|n| n.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "id в notices.json должны быть уникальны");
        assert!(parse(text, env!("CARGO_PKG_VERSION")).is_ok());
    }
}
