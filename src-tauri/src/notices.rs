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
    /// Подпись кнопки со ссылкой; нет - «Подробнее».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_label: Option<String>,
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
    pub link_label: Option<String>,
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
            // Подпись кнопки: без пробелов по краям, не длиннее 40 символов.
            n.link_label = n
                .link_label
                .map(|l| l.trim().chars().take(40).collect::<String>())
                .filter(|l| !l.is_empty() && n.link.is_some());
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
        return Ok(cached(&s).into_iter().filter(|n| !s.notices_toasted.contains(&n.id)).collect());
    }
    // raw.githubusercontent.com у части провайдеров в России заблокирован:
    // не достали напрямую — берём через свой прокси (строгий вход моста).
    // Два источника: файл на GitHub и сервер рассылок. Один упал — берём
    // другой; ошибка — только если не получилось ничего. Сервер статистики
    // спрашиваем только при включённой статистике.
    let current = env!("CARGO_PKG_VERSION");
    let (server_on, au) = tokio::task::spawn_blocking(|| match crate::settings::load_checked() {
        Ok(s) => (crate::settings::usage_stats_on(&s), crate::settings::auto_update_state(&s).enabled),
        Err(_) => (false, false),
    })
    .await
    .unwrap_or((false, false));
    let server_fut = async {
        if server_on {
            Some(fetch_server(current, au).await)
        } else {
            None
        }
    };
    let (github, server) = tokio::join!(fetch_github(current), server_fut);
    if let (Err(a), false) = (&github, matches!(server, Some(Ok(_)))) {
        return Err(match &server {
            Some(Err(b)) => format!("рассылки: GitHub — {a}; сервер — {b}"),
            _ => format!("рассылки: GitHub — {a}"),
        });
    }
    // Кэш каждого источника свой: ответил — заменяется целиком, упал —
    // остаётся прежний (заново проверенный на пригодность этой версии).
    let s = crate::settings::update_notices_caches(|s| {
        combine_caches(&s.notices_cache, &s.notices_cache_server, github, server, current)
    })?;
    Ok(cached(&s).into_iter().filter(|n| !s.notices_toasted.contains(&n.id)).collect())
}

/// Новые кэши (GitHub, сервер). Источник ответил (Ok) — его кэш заменяется
/// целиком; упал (Err) или не спрашивался (None) — берём его прежний кэш,
/// заново пропущенный через applies(current).
fn combine_caches(
    prev_github: &[Notice],
    prev_server: &[Notice],
    github: Result<Vec<Notice>, String>,
    server: Option<Result<Vec<Notice>, String>>,
    current: &str,
) -> (Vec<Notice>, Vec<Notice>) {
    let keep = |prev: &[Notice]| -> Vec<Notice> { prev.iter().filter(|n| applies(n, current)).cloned().collect() };
    let gh = github.unwrap_or_else(|_| keep(prev_github));
    let srv = match server {
        Some(Ok(v)) => v,
        _ => keep(prev_server),
    };
    (gh, srv)
}

/// Все закэшированные рассылки (оба источника), новые сверху.
pub fn cached(s: &crate::settings::Settings) -> Vec<Notice> {
    merge(vec![s.notices_cache.clone(), s.notices_cache_server.clone()])
}

/// Файл рассылок на GitHub: напрямую, не вышло — через мост. 404 — пусто.
async fn fetch_github(current: &str) -> Result<Vec<Notice>, String> {
    let text = match download(None).await {
        Ok(t) => t,
        Err(direct) => download(Some(&format!("http://127.0.0.1:{}", crate::claude_desktop::PROXY_PORT)))
            .await
            .map_err(|via| format!("напрямую — {direct}; через мост — {via}"))?,
    };
    match text {
        Some(t) => parse(&t, current),
        None => Ok(Vec::new()),
    }
}

/// Рассылки с сервера: `GET /v1/notices?v=&os=`. 404 — пусто.
async fn fetch_server(current: &str, au: bool) -> Result<Vec<Notice>, String> {
    match crate::telemetry::get(&server_notices_query(current, au)).await? {
        Some(t) => parse_server(&t, current),
        None => Ok(Vec::new()),
    }
}

/// Запрос рассылок сервера: версия, система и режим автообновления (au=1|0).
fn server_notices_query(current: &str, au: bool) -> String {
    let os = if cfg!(target_os = "windows") { "windows" } else { "macos" };
    format!("/v1/notices?v={current}&os={os}&au={}", u8::from(au))
}

/// Ответ сервера: `{"notices":[...]}`, записи — как в notices.json.
/// Отдельная функция: формат могут уточнить.
fn parse_server(text: &str, current: &str) -> Result<Vec<Notice>, String> {
    parse(text, current).map_err(|e| e.replace("notices.json", "ответ сервера"))
}

/// Слить списки по id (первый источник главнее), новые сверху.
fn merge(lists: Vec<Vec<Notice>>) -> Vec<Notice> {
    let mut seen = std::collections::HashSet::new();
    let mut v: Vec<Notice> = lists.into_iter().flatten().filter(|n| seen.insert(n.id.clone())).collect();
    v.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.id.cmp(&b.id)));
    v.truncate(50);
    v
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
            link_label: None,
        });
    }
    for n in &cached(&s) {
        items.push(NoticeItem {
            id: n.id.clone(),
            kind: "news",
            date: n.date.clone(),
            title: n.title.clone(),
            body: n.body.clone(),
            link: n.link.clone(),
            link_label: n.link_label.clone(),
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
    fn merge_dedups_by_id_and_sorts() {
        let n = |id: &str, date: &str, title: &str| Notice {
            id: id.into(),
            date: date.into(),
            title: title.into(),
            body: String::new(),
            link: None,
            link_label: None,
            platforms: vec![],
            min_version: None,
            max_version: None,
        };
        let gh = vec![n("a", "2026-11-01", "из GitHub"), n("b", "2026-11-03", "b")];
        let srv = vec![n("a", "2026-11-01", "с сервера"), n("c", "2026-11-05", "c")];
        let m = merge(vec![gh, srv]);
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
        assert_eq!(m.iter().find(|x| x.id == "a").unwrap().title, "из GitHub", "дубль: первый источник");
        assert!(merge(vec![vec![], vec![]]).is_empty());
        let one = merge(vec![Vec::new(), vec![n("z", "2026-11-01", "z")]]);
        assert_eq!(one.len(), 1, "пустой источник не мешает другому");
        // Прежний кэш - младший источник: живёт, пока источник лежит.
        let cache = vec![n("old", "2026-10-01", "из кэша"), n("a", "2026-11-01", "устаревший a")];
        let m = merge(vec![vec![n("a", "2026-11-01", "свежий a")], Vec::new(), cache]);
        assert_eq!(m.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["a", "old"]);
        assert_eq!(m[0].title, "свежий a");
    }

    #[test]
    fn source_caches_are_independent() {
        let n = |id: &str, max: Option<&str>| Notice {
            id: id.into(),
            date: "2026-11-01".into(),
            title: id.into(),
            body: String::new(),
            link: None,
            link_label: None,
            platforms: vec![],
            min_version: None,
            max_version: max.map(str::to_string),
        };
        let prev_gh = vec![n("gh-old", None), n("gh-gone", Some("0.1.0"))];
        let prev_srv = vec![n("srv-old", None)];
        let ids = |v: &[Notice]| v.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
        // GitHub упал, сервер ответил: кэш GitHub - прежний (минус неподходящее), кэш сервера заменён целиком.
        let (gh, srv) = combine_caches(&prev_gh, &prev_srv, Err("нет сети".into()), Some(Ok(vec![n("srv-new", None)])), "0.8.3");
        assert_eq!(ids(&gh), vec!["gh-old"], "прежний кэш заново прошёл applies");
        assert_eq!(ids(&srv), vec!["srv-new"], "ответивший источник заменяется целиком");
        // Сервер упал, GitHub ответил пустым списком: пустой ответ тоже заменяет кэш.
        let (gh, srv) = combine_caches(&prev_gh, &prev_srv, Ok(vec![]), Some(Err("500".into())), "0.8.3");
        assert!(gh.is_empty());
        assert_eq!(ids(&srv), vec!["srv-old"]);
        // Сервер не спрашивали (статистика выключена): его кэш остаётся.
        let (_, srv) = combine_caches(&prev_gh, &prev_srv, Ok(vec![]), None, "0.8.3");
        assert_eq!(ids(&srv), vec!["srv-old"]);
        // Колокольчик видит оба источника.
        let s = crate::settings::Settings { notices_cache: prev_gh, notices_cache_server: prev_srv, ..Default::default() };
        assert_eq!(cached(&s).len(), 3);
    }

    #[test]
    fn link_label_is_optional_and_trimmed() {
        let v = parse(
            r#"{"notices":[
            {"id":"a","date":"2026-11-01","title":"A","link":"https://x.y","link_label":"  Что нового  "},
            {"id":"b","date":"2026-11-02","title":"B","link":"https://x.y"},
            {"id":"c","date":"2026-11-03","title":"C","link_label":"без ссылки"},
            {"id":"d","date":"2026-11-04","title":"D","link":"https://x.y","link_label":"   ","level":"info"}
        ]}"#,
            "0.8.3",
        )
        .unwrap();
        let get = |id: &str| v.iter().find(|n| n.id == id).unwrap().link_label.clone();
        assert_eq!(get("a").as_deref(), Some("Что нового"));
        assert_eq!(get("b"), None);
        assert_eq!(get("c"), None, "подпись без ссылки не нужна");
        assert_eq!(get("d"), None);
    }

    #[test]
    fn server_query_carries_auto_update_flag() {
        assert!(server_notices_query("0.9.1", true).ends_with("&au=1"));
        assert!(server_notices_query("0.9.1", false).ends_with("&au=0"));
        assert!(server_notices_query("0.9.1", true).starts_with("/v1/notices?v=0.9.1&os="));
    }

    #[test]
    fn server_notices_use_same_format() {
        let v = parse_server(r#"{"notices":[{"id":"s1","date":"2026-11-10","title":"С сервера","link":"https://x.y"}]}"#, "0.8.18").unwrap();
        assert_eq!(v.len(), 1);
        assert!(parse_server("{bad", "0.8.18").unwrap_err().contains("ответ сервера"));
    }

    #[test]
    fn only_recent_notices_pop_up() {
        let n = |date: &str| Notice {
            id: "x".into(),
            date: date.into(),
            title: "t".into(),
            body: String::new(),
            link: None,
            link_label: None,
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
