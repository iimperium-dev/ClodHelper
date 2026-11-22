//! Купленный на сайте прокси: ссылка `clodhelper://add?key=<key>` со страницы
//! оплаты и запрос прокси у магазина по этому ключу.
//!
//! Ссылка сама ничего не добавляет: программа только запоминает ключ и
//! показывает окно «Добавить купленный прокси?». Добавляет человек кнопкой.
//!
//! Ключ - случайные 32 байта в base64url без `=` (ровно 43 символа). Любая
//! другая ссылка (другой хост, лишние параметры, кривой ключ) молча
//! игнорируется. Ключ и пароли прокси в журнал не пишутся.

use std::sync::Mutex;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::settings::{self, ShopMeta, UpsertItem};

/// Сервер магазина.
pub const SHOP_BASE: &str = "https://shop.clodhelper.com";
/// Кабинет покупателя на сайте: продление по ключу (ключ - после «#», на
/// сервер сайта не уходит).
pub const RENEW_BASE: &str = "https://clodhelper.com/buy/account/";
/// Событие главному окну: пришла ссылка, ключ ждёт в `shop_link_take`.
pub const LINK_EVENT: &str = "shop:link";
/// Сколько прокси берём из одного ответа (пул программы - до 5).
const MAX_PROXIES: usize = 5;

/// Ключ из последней ссылки, ещё не забранный окном. Окно может быть ещё не
/// загружено (холодный старт по ссылке), поэтому ключ ждёт здесь.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

// ---------------------------------------------------------------- разбор

/// Ключ магазина: base64url без выравнивания, ровно 32 байта.
pub fn valid_key(key: &str) -> bool {
    key.len() == 43
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        // Строгое декодирование: «хвостовые» биты последнего символа - нули,
        // то есть у одного ключа ровно одна запись.
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(key)
            .is_ok_and(|b| b.len() == 32)
}

/// Ключ из ссылки `clodhelper://add?key=<key>`, иначе None.
pub fn parse_link(link: &str) -> Option<String> {
    let u = url::Url::parse(link.trim()).ok()?;
    if u.scheme() != "clodhelper" {
        return None;
    }
    if !u.host_str().is_some_and(|h| h.eq_ignore_ascii_case("add")) {
        return None;
    }
    // Windows и браузеры иногда дописывают «/» перед «?».
    if !(u.path().is_empty() || u.path() == "/") {
        return None;
    }
    if !u.username().is_empty() || u.password().is_some() || u.port().is_some() || u.fragment().is_some() {
        return None;
    }
    let mut pairs = u.query_pairs();
    let (name, value) = pairs.next()?;
    if name != "key" || pairs.next().is_some() {
        return None;
    }
    let key = value.into_owned();
    valid_key(&key).then_some(key)
}

/// Ссылки от системы (холодный старт, вторая копия, macOS open URL):
/// годный ключ запомнить, показать окно и сказать ему об этом.
pub fn on_links<I, S>(app: &AppHandle, links: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let Some(key) = links.into_iter().find_map(|l| parse_link(l.as_ref())) else {
        tracing::info!("shop: ссылка не похожа на покупку, пропускаю");
        return;
    };
    if let Ok(mut g) = PENDING.lock() {
        *g = Some(key);
    }
    tracing::info!("shop: пришла ссылка на купленный прокси");
    crate::show_main_window(app);
    let _ = app.emit_to("main", LINK_EVENT, ());
}

/// Забрать ключ из последней ссылки (один раз).
#[tauri::command]
pub fn shop_link_take() -> Option<String> {
    PENDING.lock().ok().and_then(|mut g| g.take())
}

// ---------------------------------------------------------------- магазин

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimProxy {
    /// `http://login:pass@ip:port` - то, что попадает в пул.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks_url: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Срок действия, ISO 8601.
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claim {
    pub proxies: Vec<ClaimProxy>,
    #[serde(default)]
    pub email_masked: Option<String>,
}

fn base() -> String {
    #[cfg(debug_assertions)]
    if let Some(u) = std::env::var("CLOD_SHOP_URL").ok().filter(|s| !s.trim().is_empty()) {
        return u.trim().trim_end_matches('/').to_string();
    }
    SHOP_BASE.to_string()
}

/// Разобрать ответ магазина: оставить только прокси, которые программа
/// умеет добавить. Ни одного - ошибка.
pub fn parse_claim(body: &str) -> Result<Claim, String> {
    let mut c: Claim =
        serde_json::from_str(body).map_err(|_| "Магазин ответил непонятно. Попробуйте позже".to_string())?;
    c.proxies.retain(|p| crate::proxy::parse(&p.url).is_ok());
    c.proxies.truncate(MAX_PROXIES);
    for p in &mut c.proxies {
        p.url = p.url.trim().to_string();
        // Неправильный SOCKS не мешает добавить основной адрес.
        if p.socks_url.as_deref().is_some_and(|s| crate::proxy::parse(s).is_err()) {
            p.socks_url = None;
        }
    }
    if c.proxies.is_empty() {
        return Err("В ответе магазина нет прокси, которые можно добавить".into());
    }
    Ok(c)
}

fn status_error(code: u16) -> String {
    match code {
        404 => "Покупка по этой ссылке не найдена. Откройте ссылку из письма или со страницы оплаты ещё раз".into(),
        410 => "Срок этого прокси уже закончился".into(),
        429 => "Слишком много попыток. Подождите минуту и попробуйте ещё раз".into(),
        _ => format!("Магазин сейчас не отвечает (код {code}). Попробуйте позже"),
    }
}

/// Подставной ответ для отладки без сервера: `CLOD_SHOP_FAKE=1`.
#[cfg(debug_assertions)]
fn fake_claim() -> Option<Claim> {
    std::env::var("CLOD_SHOP_FAKE").ok().filter(|v| v == "1")?;
    Some(Claim {
        proxies: vec![ClaimProxy {
            url: "http://buyer:secret@203.0.113.10:8000".into(),
            socks_url: Some("socks5://buyer:secret@203.0.113.10:1080".into()),
            country: Some("FI".into()),
            label: Some("Финляндия".into()),
            expires_at: Some("2026-12-22T00:00:00Z".into()),
        }],
        email_masked: Some("b***@example.com".into()),
    })
}

/// Купленные прокси по ключу из ссылки. Напрямую, а если соединиться не
/// удалось - через мост (как статистика).
#[tauri::command]
pub async fn shop_claim(key: String) -> Result<Claim, String> {
    if !valid_key(&key) {
        return Err("Ссылка повреждена. Откройте её ещё раз со страницы оплаты".into());
    }
    #[cfg(debug_assertions)]
    if let Some(c) = fake_claim() {
        return Ok(c);
    }
    let (status, body) = fetch(&key).await?;
    if status != 200 {
        tracing::warn!("shop: ответ магазина {status}");
        return Err(status_error(status));
    }
    parse_claim(&body)
}

/// GET /v1/claim/<key>: код ответа и тело. Ошибка - только «нет связи».
async fn fetch(key: &str) -> Result<(u16, String), String> {
    let url = format!("{}/v1/claim/{key}", base());
    crate::telemetry::with_fallback(|c| {
        let url = url.clone();
        async move {
            let r = c.get(url).header("Accept", "application/json").send().await?;
            let status = r.status().as_u16();
            Ok((status, r.text().await?))
        }
    })
    .await
    .map_err(|_| {
        // Текст ошибки reqwest содержит адрес с ключом - его не показываем.
        tracing::warn!("shop: магазин недоступен");
        "Нет связи с магазином. Проверьте интернет и попробуйте ещё раз".to_string()
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddItem {
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Ключ покупки: хранится у прокси для сверки срока и «Продлить».
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

fn meta_of(it: &AddItem) -> Option<ShopMeta> {
    let key = it.key.as_deref().filter(|k| valid_key(k))?;
    Some(ShopMeta {
        key: key.to_string(),
        expires_at: it.expires_at.clone(),
        expired: false,
        checked_at: Some(unix_now()),
    })
}

/// «Добавить» в окне покупки: прокси в пул (тот же адрес - обновить на
/// месте, см. `settings::upsert_pool`), затем мост с новым пулом.
/// `make_primary` - первый купленный становится основным. Ответ - пул после
/// записи.
#[tauri::command]
pub async fn shop_add(items: Vec<AddItem>, make_primary: bool) -> Result<Vec<String>, String> {
    if items.is_empty() {
        return Err("Нечего добавлять".into());
    }
    for it in &items {
        crate::proxy::parse(&it.url)?;
    }
    let list: Vec<UpsertItem> = items
        .iter()
        .map(|i| UpsertItem { url: i.url.trim().to_string(), label: i.label.clone(), meta: meta_of(i) })
        .collect();
    let saved = settings::with_settings(|s| settings::upsert_pool(s, &list, make_primary).map(|_| ()))?;
    crate::commands::restart_with_pool(saved.proxy_url.clone().unwrap_or_default()).await?;
    Ok(settings::effective_proxies(&settings::load()))
}

/// Мастер: купленный прокси подставлен в поле. Ключ и срок запоминаются
/// заранее - мастер сам запишет прокси в пул; не дойдёт - запись уберётся
/// при следующей смене пула.
#[tauri::command]
pub fn shop_remember(item: AddItem) -> Result<(), String> {
    crate::proxy::parse(&item.url)?;
    let Some(meta) = meta_of(&item) else { return Ok(()) };
    let url = item.url.trim().to_string();
    settings::with_settings(|s| {
        if let Some(l) = item.label.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            s.proxy_labels.entry(url.clone()).or_insert_with(|| l.to_string());
        }
        s.shop_proxies.insert(url, meta);
        Ok(())
    })
    .map(|_| ())
}

/// Что окну знать о прокси из магазина (без ключа).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShopInfo {
    pub expires_at: Option<String>,
    pub expired: bool,
}

/// Прокси из магазина: URL → срок. Ключ сюда не попадает.
#[tauri::command]
pub fn shop_meta_get() -> std::collections::HashMap<String, ShopInfo> {
    settings::load()
        .shop_proxies
        .into_iter()
        .map(|(u, m)| (u, ShopInfo { expires_at: m.expires_at, expired: m.expired }))
        .collect()
}

/// Страница продления в кабинете покупателя.
pub fn renew_url(key: &str) -> String {
    format!("{RENEW_BASE}#key={key}")
}

/// «Продлить»: кабинет покупателя в браузере. Ключ берётся из настроек и
/// окну не отдаётся.
#[tauri::command]
pub fn shop_renew(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let s = settings::load();
    let meta = s.shop_proxies.get(&url).ok_or("Этот прокси куплен не в магазине Clod Helper")?;
    app.opener()
        .open_url(renew_url(&meta.key), None::<&str>)
        .map_err(|_| "Не получилось открыть браузер".to_string())
}

// ---------------------------------------------------------------- сверка срока

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Раз в сутки на ключ.
const CHECK_EVERY: u64 = 24 * 3600;

/// Ключи, срок которых пора сверить с магазином.
pub fn keys_due(s: &settings::Settings, now: u64) -> Vec<String> {
    let mut keys: Vec<String> = s
        .shop_proxies
        .values()
        .filter(|m| m.checked_at.is_none_or(|t| now.saturating_sub(t) >= CHECK_EVERY))
        .map(|m| m.key.clone())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Ответ магазина по ключу - в данные прокси с этим ключом. `None` - нет
/// связи или непонятный ответ: попробуем позже.
pub fn apply_check(s: &mut settings::Settings, key: &str, status: u16, body: &str, now: u64) {
    let claim = (status == 200).then(|| parse_claim(body).ok()).flatten();
    if status == 200 && claim.is_none() {
        return;
    }
    for (url, m) in s.shop_proxies.iter_mut().filter(|(_, m)| m.key == key) {
        m.checked_at = Some(now);
        match status {
            200 => {
                let c = claim.as_ref().expect("200 с разобранным ответом");
                let addr = settings::proxy_addr_key(url);
                if let Some(p) = c.proxies.iter().find(|p| settings::proxy_addr_key(&p.url) == addr) {
                    m.expires_at = p.expires_at.clone();
                    m.expired = false;
                }
            }
            410 => m.expired = true,
            // 404 и прочее: срок не трогаем, спросим через сутки.
            _ => {}
        }
    }
}

/// Сверка срока прокси из магазина: раз в час смотрим, каким ключам пора
/// (раз в сутки на ключ).
pub fn spawn_expiry_check() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            let due = keys_due(&settings::load(), unix_now());
            for key in due {
                let Ok((status, body)) = fetch(&key).await else { continue };
                let _ = settings::with_settings_strict(|s| {
                    apply_check(s, &key, status, &body, unix_now());
                    Ok(())
                });
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA";

    #[test]
    fn key_is_strict_base64url_of_32_bytes() {
        assert_eq!(KEY.len(), 43);
        assert!(valid_key(KEY));
        assert!(valid_key(&"A".repeat(43)));
        assert!(valid_key(&"_-".repeat(21).chars().chain("A".chars()).collect::<String>()));
        // Длина
        assert!(!valid_key(&KEY[..42]));
        assert!(!valid_key(&format!("{KEY}A")));
        assert!(!valid_key(""));
        // Выравнивание и чужие алфавиты
        assert!(!valid_key(&format!("{}=", &KEY[..42])));
        assert!(!valid_key(&format!("+{}", &KEY[1..])));
        assert!(!valid_key(&format!("/{}", &KEY[1..])));
        assert!(!valid_key(&format!("%{}", &KEY[1..])));
        assert!(!valid_key(&format!("ж{}", &KEY[2..])));
        // Последний символ с ненулевыми лишними битами - не каноничная запись
        assert!(!valid_key(&format!("{}B", &KEY[..42])));
    }

    #[test]
    fn parses_good_links() {
        let k = Some(KEY.to_string());
        assert_eq!(parse_link(&format!("clodhelper://add?key={KEY}")), k);
        assert_eq!(parse_link(&format!("clodhelper://add/?key={KEY}")), k);
        assert_eq!(parse_link(&format!("CLODHELPER://ADD?key={KEY}")), k);
        assert_eq!(parse_link(&format!("  clodhelper://add?key={KEY}\n")), k);
    }

    #[test]
    fn ignores_other_links() {
        for l in [
            format!("clodhelper://remove?key={KEY}"),
            format!("clodhelper://add/x?key={KEY}"),
            format!("clodhelper://add?key={KEY}&proxy=http://evil:1"),
            format!("clodhelper://add?proxy=http://evil:1&key={KEY}"),
            format!("clodhelper://add?key={KEY}&key={KEY}"),
            format!("clodhelper://add?Key={KEY}"),
            format!("clodhelper://add?key={KEY}#x"),
            format!("clodhelper://add:80?key={KEY}"),
            format!("clodhelper://u:p@add?key={KEY}"),
            format!("clodhelper://add.evil.com?key={KEY}"),
            format!("https://add?key={KEY}"),
            format!("clodhelperx://add?key={KEY}"),
            "clodhelper://add?key=short".into(),
            "clodhelper://add?key=".into(),
            "clodhelper://add".into(),
            "clodhelper:add?key=x".into(),
            "".into(),
            "--tray".into(),
        ] {
            assert_eq!(parse_link(&l), None, "{l}");
        }
        // Ключ с %-кодированием «+» после декодирования не base64url
        assert_eq!(parse_link(&format!("clodhelper://add?key=%2B{}", &KEY[1..])), None);
    }

    #[test]
    fn take_returns_key_once() {
        *PENDING.lock().unwrap() = Some(KEY.into());
        assert_eq!(shop_link_take(), Some(KEY.to_string()));
        assert_eq!(shop_link_take(), None);
    }

    #[test]
    fn claim_keeps_only_usable_proxies() {
        let body = r#"{
            "proxies": [
                { "url": "http://login:pass@185.68.187.138:8000", "socks_url": "socks5://login:pass@185.68.187.138:1080",
                  "country": "FI", "label": "Финляндия", "expires_at": "2026-12-22T00:00:00Z", "extra": 1 },
                { "url": "ftp://x:y@1.2.3.4:21", "country": "DE" },
                { "url": "http://a:b@5.6.7.8:3128", "socks_url": "not a proxy" }
            ],
            "email_masked": "m***@mail.ru"
        }"#;
        let c = parse_claim(body).unwrap();
        assert_eq!(c.proxies.len(), 2);
        assert_eq!(c.proxies[0].country.as_deref(), Some("FI"));
        assert_eq!(c.proxies[0].expires_at.as_deref(), Some("2026-12-22T00:00:00Z"));
        assert_eq!(c.proxies[1].url, "http://a:b@5.6.7.8:3128");
        assert_eq!(c.proxies[1].socks_url, None);
        assert_eq!(c.proxies[1].country, None);
        assert_eq!(c.email_masked.as_deref(), Some("m***@mail.ru"));
    }

    #[test]
    fn claim_errors() {
        assert!(parse_claim("not json").is_err());
        assert!(parse_claim(r#"{"proxies": []}"#).is_err());
        assert!(parse_claim(r#"{"proxies": [{"url": "nonsense"}]}"#).is_err());
        let many: Vec<String> = (1..=7).map(|i| format!(r#"{{"url":"http://u:p@10.0.0.{i}:8000"}}"#)).collect();
        let c = parse_claim(&format!(r#"{{"proxies":[{}]}}"#, many.join(","))).unwrap();
        assert_eq!(c.proxies.len(), MAX_PROXIES);
    }

    #[test]
    fn status_messages_have_no_dashes() {
        for code in [404, 410, 429, 500] {
            let m = status_error(code);
            assert!(!m.contains('—') && !m.contains('–'), "{m}");
        }
    }

    fn shop_settings() -> settings::Settings {
        let mut s = settings::Settings::default();
        let url = "http://buyer:pw@45.142.7.19:8000".to_string();
        s.proxies = vec![url.clone()];
        s.shop_proxies.insert(
            url,
            ShopMeta { key: KEY.into(), expires_at: Some("2026-12-15T00:00:00Z".into()), expired: false, checked_at: Some(1_000) },
        );
        s
    }

    #[test]
    fn expiry_is_checked_once_a_day_per_key() {
        let s = shop_settings();
        assert!(keys_due(&s, 1_000 + 3600).is_empty());
        assert_eq!(keys_due(&s, 1_000 + CHECK_EVERY), vec![KEY.to_string()]);
        let mut never = s.clone();
        never.shop_proxies.values_mut().for_each(|m| m.checked_at = None);
        assert_eq!(keys_due(&never, 0), vec![KEY.to_string()]);
    }

    #[test]
    fn expiry_check_updates_term_and_expired_flag() {
        let url = "http://buyer:pw@45.142.7.19:8000";
        // Продлили: новый срок.
        let mut s = shop_settings();
        let body = r#"{"proxies":[{"url":"http://buyer:pw@45.142.7.19:8000","expires_at":"2027-01-15T00:00:00Z"}]}"#;
        apply_check(&mut s, KEY, 200, body, 5_000);
        let m = &s.shop_proxies[url];
        assert_eq!(m.expires_at.as_deref(), Some("2027-01-15T00:00:00Z"));
        assert_eq!(m.checked_at, Some(5_000));
        // Срок кончился.
        apply_check(&mut s, KEY, 410, "", 6_000);
        assert!(s.shop_proxies[url].expired);
        // Непонятный ответ - ничего не трогаем, спросим через час.
        let mut s = shop_settings();
        apply_check(&mut s, KEY, 200, "not json", 7_000);
        assert_eq!(s.shop_proxies[url].checked_at, Some(1_000));
        // Чужой ключ - не наш прокси.
        apply_check(&mut s, &"A".repeat(43), 410, "", 8_000);
        assert!(!s.shop_proxies[url].expired);
    }

    #[test]
    fn renew_link_carries_key_after_hash() {
        assert_eq!(renew_url(KEY), format!("https://clodhelper.com/buy/account/#key={KEY}"));
    }

    #[test]
    fn meta_only_for_valid_key() {
        let it = |key: Option<&str>| AddItem {
            url: "http://a:b@1.2.3.4:8000".into(),
            label: None,
            key: key.map(Into::into),
            expires_at: Some("2026-12-15T00:00:00Z".into()),
        };
        assert!(meta_of(&it(None)).is_none());
        assert!(meta_of(&it(Some("bad"))).is_none());
        let m = meta_of(&it(Some(KEY))).unwrap();
        assert_eq!(m.key, KEY);
        assert_eq!(m.expires_at.as_deref(), Some("2026-12-15T00:00:00Z"));
    }

    #[tokio::test]
    async fn claim_rejects_bad_key_without_network() {
        assert!(shop_claim("bad".into()).await.is_err());
    }
}
