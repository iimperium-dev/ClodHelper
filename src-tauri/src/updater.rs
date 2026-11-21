//! Обновление Clod Helper в один клик (без подписи Apple и Microsoft).
//!
//! Проверка - по манифесту `latest.json` последнего релиза GitHub
//! (iimperium-dev/ClodHelper): ссылка `releases/latest/download/…` идёт
//! через сайт и CDN, а не через API - без лимита «60 запросов в час на IP».
//! Манифест подписан ключом разработчика (ed25519, `latest.json.sig`;
//! подписывает scripts/publish-release.mjs на его машине - на GitHub
//! закрытого ключа нет). Подпись не сошлась - обновление не ставим: так не
//! пройдёт подмена файлов релиза (утёкший токен, взлом аккаунта, зеркало).
//! Подпись берётся по тегу из манифеста; не сошлась - один повтор, где и
//! манифест, и подпись запрашиваются по тегу (кэш CDN мог отдать файлы
//! разных релизов).
//! В манифесте - SHA-256 установщиков; скачанный файл сверяется с ним:
//!   - macOS: `*_universal.dmg` - новая версия встаёт на место текущей,
//!     старая уходит в ~/Library/Application Support/ClodHelper-backup
//!     (одна последняя копия);
//!   - Windows: `*_x64-setup.exe` (NSIS) - тихая установка поверх текущей
//!     в профиль пользователя (без UAC), новая версия стартует сама.
//!
//! Сеть: напрямую (без прокси из окружения - программа сама ставит
//! HTTPS_PROXY на мост); через мост 127.0.0.1:PROXY_PORT - только если прямое
//! соединение не установилось. Скачивание - потоком в файл `.part` с суммой
//! на лету, тайм-аутом на простой, докачкой через Range и тремя попытками.
//!
//! Необязательные поля подписанного манифеста: `rollout` (0-100),
//! `paused`, `min_version`, `force` - см. scripts/lib/controls.mjs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const REPO: &str = "iimperium-dev/ClodHelper";

/// Открытые ключи подписи манифеста (ed25519, base64 сырых 32 байт).
/// Несколько - на время смены ключа: новый добавить сюда заранее, выпустить
/// версию, и только потом подписывать новым.
const UPDATE_KEYS: &[&str] = &[
    // Основной.
    "dkN/Vz8GRDntm9Ra94VI+slYMi3kgsIv2rbZoB8hjow=",
    // Резервный, закрытая часть офлайн у владельца.
    "KfvP06ngR5AN35ratGxb2HRcXyJFTlCsNi0o1w7ae0k=",
];

const MANIFEST: &str = "latest.json";

/// Манифест старше этого срока игнорируется (подписанный, но давний).
const MAX_MANIFEST_AGE_SECS: u64 = 400 * 86_400;
/// Соединение с сервером не установилось за это время - ошибка соединения.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Манифест и подпись - маленькие файлы: общий предел на запрос.
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Скачивание: столько можно не получать данные, прежде чем считать обрыв.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Паузы перед 2-й и 3-й попытками скачивания.
const RETRY_PAUSES: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(15)];

/// Платформа этой сборки в манифесте.
#[cfg(not(target_os = "windows"))]
const PLATFORM: &str = "darwin-universal";
#[cfg(target_os = "windows")]
const PLATFORM: &str = "windows-x86_64";

fn full_rollout() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// Установщик для этой ОС (.dmg или -setup.exe).
    pub url: String,
    /// SHA-256 установщика из подписанного манифеста.
    #[serde(default)]
    pub sha256: String,
    /// Пункты «Что нового» этой версии.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Срочное исправление (манифест `force`).
    #[serde(default)]
    pub force: bool,
    /// Установка слишком старая для автоматического обновления (манифест
    /// `min_version`): показываем ручную установку со ссылкой `page`.
    #[serde(default)]
    pub manual_only: bool,
    /// Страница релиза на GitHub - для ручной установки.
    #[serde(default)]
    pub page: String,
    /// Доля установок, которым предложена версия (манифест `rollout`).
    #[serde(default = "full_rollout")]
    pub rollout: u32,
    /// Обновление поставится само (автообновление включено и условия не
    /// исключают тихую установку): по этому полю интерфейс обещает «поставится
    /// само». Считает бэкенд (auto_update::handles_silently).
    #[serde(default)]
    pub handles_silently: bool,
}

/// Ход установки для UI: событие `update:progress`.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    /// download → verify → install → restart.
    pub stage: &'static str,
    pub received: u64,
    /// Размер установщика; 0 - сервер не сообщил.
    pub total: u64,
}

/// Манифест обновления (latest.json; собирает CI, подписывает разработчик).
#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    tag: String,
    /// ISO-дата сборки; старше MAX_MANIFEST_AGE_SECS - манифест не берём.
    #[serde(default)]
    pub_date: Option<String>,
    /// Время подписи (unix-секунды), ставит publish-release.mjs. Программа
    /// хранит последний принятый и отвергает меньший (откат к старому манифесту).
    #[serde(default)]
    seq: Option<u64>,
    /// Доля установок 0-100; нет поля - 100.
    #[serde(default)]
    rollout: Option<u32>,
    /// Остановлено: никто не ставит и не получает уведомлений.
    #[serde(default)]
    paused: bool,
    /// Установки старше этой версии не ставят сами.
    #[serde(default)]
    min_version: Option<String>,
    /// Срочное исправление: ставить, даже если «Что нового» для ОС пусто.
    #[serde(default)]
    force: bool,
    #[serde(default)]
    highlights: Vec<String>,
    platforms: std::collections::HashMap<String, ManifestFile>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    /// Имя файла в релизе (как его назвал GitHub - пробелы стали точками).
    file: String,
    sha256: String,
    /// «Что нового» именно для этой ОС (manifest ≥0.8.11). Пустой список -
    /// в релизе для этой ОС ничего нового: обновление не предлагаем (если нет
    /// `force`), люди не получают уведомление о версии, которая им ничего не
    /// даёт. Нет поля - старый манифест: берём общий список.
    #[serde(default)]
    highlights: Option<Vec<String>>,
}

/// Кто спрашивает: фоновая проверка уважает раскатку, ручная - нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Background,
    Manual,
}

/// "0.6.0" / "v0.6.1" → (0, 6, 1).
pub fn parse_ver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

// ---------------------------------------------------------------- сеть

fn bridge_proxy() -> String {
    format!("http://127.0.0.1:{}", crate::claude_desktop::PROXY_PORT)
}

/// Клиент. Прокси из окружения не берём (программа сама ставит HTTPS_PROXY
/// на мост): либо напрямую, либо явно через мост. `total` - общий предел на
/// запрос (для больших файлов не задаётся: там тайм-аут на простой).
fn build_client(proxy: Option<&str>, total: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("ClodHelper/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .no_proxy();
    if let Some(t) = total {
        b = b.timeout(t);
    }
    if let Some(p) = proxy {
        b = b.proxy(reqwest::Proxy::all(p).map_err(|e| format!("Обновления: {e}"))?);
    }
    b.build().map_err(|e| format!("Обновления: {e}"))
}

struct FetchErr {
    /// Не удалось соединиться (или вышел тайм-аут): запрос до сервера не
    /// дошёл, можно пробовать мост.
    connect: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    not_found: bool,
    msg: String,
}

async fn fetch_once(proxy: Option<&str>, url: &str) -> Result<Vec<u8>, FetchErr> {
    let err = |connect: bool, msg: String| FetchErr { connect, not_found: false, msg };
    let c = build_client(proxy, Some(MANIFEST_TIMEOUT)).map_err(|m| err(false, m))?;
    let r = c
        .get(url)
        .send()
        .await
        .map_err(|e| err(e.is_connect() || e.is_timeout(), format!("нет связи с GitHub ({e})")))?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(FetchErr { connect: false, not_found: true, msg: "в релизе нет подписанного манифеста".into() });
    }
    let r = r.error_for_status().map_err(|e| err(false, format!("GitHub: {e}")))?;
    r.bytes().await.map(|b| b.to_vec()).map_err(|e| err(e.is_timeout(), format!("GitHub: {e}")))
}

/// GET небольшого файла: напрямую, не соединились - через мост.
async fn get_small(bridge: &str, url: &str) -> Result<Vec<u8>, FetchErr> {
    match fetch_once(None, url).await {
        Ok(v) => Ok(v),
        Err(direct) if direct.connect => fetch_once(Some(bridge), url).await.map_err(|via| FetchErr {
            msg: format!("напрямую: {}; через мост: {}", direct.msg, via.msg),
            ..via
        }),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- манифест

/// Есть ли версия новее текущей. None - уже последняя (или остановлена,
/// или ещё не дошла до этой установки по раскатке).
pub async fn check(mode: Mode) -> Result<Option<UpdateInfo>, String> {
    #[cfg(debug_assertions)]
    if let Some(info) = debug_override() {
        return Ok(Some(info));
    }
    let m = fetch_manifest().await.map_err(|e| format!("Не удалось проверить обновления: {e}"))?;
    let stored = crate::settings::load().update_seq;
    if !seq_ok(m.seq, stored) {
        return Err("Не удалось проверить обновления: сервер отдал устаревший манифест, попробуйте позже".into());
    }
    if let Some(n) = m.seq {
        let _ = crate::settings::set_update_seq(n);
    }
    let ctx = Ctx { mode, number: crate::auto_update::rollout_number(), now: crate::auto_update::now_secs() };
    let mut found = update_from(m, env!("CARGO_PKG_VERSION"), PLATFORM, &ctx);
    if let Some(info) = found.as_mut() {
        info.handles_silently = crate::auto_update::handles_silently(&crate::settings::load(), info, ctx.now);
    }
    Ok(found)
}

/// Только отладочная сборка: «обновление» с локального сервера для живой
/// проверки установки. CLOD_UPDATE_URL - установщик, CLOD_UPDATE_SHA - его
/// SHA-256, CLOD_UPDATE_VERSION - версия (по умолчанию 99.0.0).
#[cfg(debug_assertions)]
fn debug_override() -> Option<UpdateInfo> {
    let url = std::env::var("CLOD_UPDATE_URL").ok()?;
    let sha = std::env::var("CLOD_UPDATE_SHA").ok()?.trim().to_lowercase();
    let version = std::env::var("CLOD_UPDATE_VERSION").unwrap_or_else(|_| "99.0.0".into());
    Some(UpdateInfo {
        version: version.clone(),
        notes: String::new(),
        url,
        sha256: sha,
        highlights: vec!["Проверка установки".into()],
        force: false,
        manual_only: false,
        page: format!("https://github.com/{REPO}/releases/tag/v{version}"),
        rollout: 100,
        handles_silently: false,
    })
}

/// Страница релиза версии `version` (ручная установка).
pub fn release_page(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

fn releases_base() -> String {
    format!("https://github.com/{REPO}/releases")
}

fn tag_ok(tag: &str) -> bool {
    let Some(v) = tag.strip_prefix('v') else { return false };
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Манифест последнего релиза - только с верной подписью.
async fn fetch_manifest() -> Result<Manifest, String> {
    fetch_manifest_from(&bridge_proxy(), &releases_base(), UPDATE_KEYS).await
}

/// Подпись - по тегу из манифеста (а не по `latest`: кэш CDN мог отдать файлы
/// разных релизов). Не сошлась - один повтор: и манифест, и подпись по тегу.
async fn fetch_manifest_from(bridge: &str, base: &str, keys: &[&str]) -> Result<Manifest, String> {
    let nf = |e: FetchErr| e.msg;
    let body = get_small(bridge, &format!("{base}/latest/download/{MANIFEST}")).await.map_err(nf)?;
    // Тег из ещё не проверенного манифеста - только для адреса, и только
    // строго вида vX.Y.Z.
    #[derive(Deserialize)]
    struct TagOnly {
        tag: String,
    }
    let tag = serde_json::from_slice::<TagOnly>(&body)
        .map_err(|e| format!("манифест не читается: {e}"))?
        .tag;
    if !tag_ok(&tag) {
        return Err("в манифесте странный тег - не ставлю".into());
    }
    let sig = get_small(bridge, &format!("{base}/download/{tag}/{MANIFEST}.sig")).await.map_err(nf)?;
    let (body, signed_tag) = match verify_manifest(&body, &sig, keys) {
        Ok(()) => (body, tag),
        Err(_) => {
            let by_tag = format!("{base}/download/{tag}");
            let body = get_small(bridge, &format!("{by_tag}/{MANIFEST}")).await.map_err(nf)?;
            let sig = get_small(bridge, &format!("{by_tag}/{MANIFEST}.sig")).await.map_err(nf)?;
            verify_manifest(&body, &sig, keys)?;
            (body, tag)
        }
    };
    let m: Manifest = serde_json::from_slice(&body).map_err(|e| format!("манифест не читается: {e}"))?;
    if m.tag != signed_tag {
        return Err("тег подписанного манифеста не совпал с запрошенным - не ставлю".into());
    }
    Ok(m)
}

/// Монотонность манифеста: `seq` не меньше сохранённого. Манифест без `seq`
/// принимается, только пока ничего не сохранено (совместимость со старыми).
fn seq_ok(seq: Option<u64>, stored: Option<u64>) -> bool {
    match (seq, stored) {
        (Some(n), Some(s)) => n >= s,
        (None, Some(_)) => false,
        _ => true,
    }
}

/// Подпись манифеста (base64 64 байт ed25519) - одним из наших ключей.
fn verify_manifest(body: &[u8], sig: &[u8], keys: &[&str]) -> Result<(), String> {
    use base64::Engine;
    use ed25519_dalek::{Signature, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let bad = || "Подпись обновления не сошлась - не ставлю (файлы релиза могли подменить)".to_string();
    let sig = b64.decode(String::from_utf8_lossy(sig).trim()).map_err(|_| bad())?;
    let sig = Signature::from_slice(&sig).map_err(|_| bad())?;
    let ok = keys.iter().any(|k| {
        let Ok(raw) = b64.decode(k) else { return false };
        let Ok(raw): Result<[u8; 32], _> = raw.try_into() else { return false };
        VerifyingKey::from_bytes(&raw).is_ok_and(|vk| vk.verify_strict(body, &sig).is_ok())
    });
    if ok { Ok(()) } else { Err(bad()) }
}

/// Контекст решения «предлагать ли»: кто спрашивает, номер раскатки, время.
struct Ctx {
    mode: Mode,
    number: u8,
    now: u64,
}

/// Дней от 1970-01-01 до гражданской даты (H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026-11-21T10:00:00.000Z" → unix-секунды начала этого дня (UTC).
fn iso_day_secs(s: &str) -> Option<u64> {
    let day = s.get(..10)?;
    let mut it = day.split('-');
    let (y, m, d) = (it.next()?.parse::<i64>().ok()?, it.next()?.parse::<i64>().ok()?, it.next()?.parse::<i64>().ok()?);
    if !(1970..=3000).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    u64::try_from(days_from_civil(y, m, d)).ok().map(|n| n * 86_400)
}

/// Манифест слишком старый. Нет даты или она не читается - не отбрасываем.
fn manifest_stale(pub_date: Option<&str>, now: u64) -> bool {
    pub_date.and_then(iso_day_secs).is_some_and(|t| now.saturating_sub(t) > MAX_MANIFEST_AGE_SECS)
}

/// Предложение по управляющим полям манифеста.
#[derive(Debug, PartialEq, Eq)]
enum Offer {
    /// Не предлагать (остановлено или не дошло по раскатке).
    Hidden,
    /// Только ручная установка (слишком старая версия).
    ManualOnly,
    Normal,
}

fn offer_for(m: &Manifest, current: &str, ctx: &Ctx) -> Offer {
    if m.paused {
        return Offer::Hidden;
    }
    let rollout = m.rollout.unwrap_or(100);
    if ctx.mode == Mode::Background && !crate::auto_update::rollout_passes(rollout, ctx.number) {
        return Offer::Hidden;
    }
    let too_old = m.min_version.as_deref().and_then(parse_ver).zip(parse_ver(current)).is_some_and(|(min, cur)| cur < min);
    if too_old { Offer::ManualOnly } else { Offer::Normal }
}

/// Обновление из проверенного манифеста: новее `current`, со своим
/// установщиком. Ссылка - на файл того же релиза.
fn update_from(m: Manifest, current: &str, platform: &str, ctx: &Ctx) -> Option<UpdateInfo> {
    if parse_ver(&m.version)? <= parse_ver(current)? {
        return None;
    }
    // Тег и версия подписаны вместе - должны совпадать.
    if m.tag.trim_start_matches('v') != m.version {
        return None;
    }
    if manifest_stale(m.pub_date.as_deref(), ctx.now) {
        return None;
    }
    let offer = offer_for(&m, current, ctx);
    if offer == Offer::Hidden {
        return None;
    }
    let f = m.platforms.get(platform)?;
    let empty_for_os = f.highlights.as_ref().is_some_and(|h| h.is_empty());
    if empty_for_os && !m.force {
        return None;
    }
    let sha = f.sha256.to_lowercase();
    let safe_name = !f.file.is_empty() && !f.file.contains(['/', '\\']) && !f.file.contains("..");
    if !safe_name || sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut highlights: Vec<String> = f.highlights.clone().unwrap_or_else(|| m.highlights.clone());
    if highlights.is_empty() {
        highlights = m.highlights.clone();
    }
    if highlights.is_empty() && m.force {
        highlights.push("Срочное исправление".into());
    }
    Some(UpdateInfo {
        url: format!("https://github.com/{REPO}/releases/download/{}/{}", m.tag, f.file),
        page: format!("https://github.com/{REPO}/releases/tag/{}", m.tag),
        version: m.version,
        notes: String::new(),
        sha256: sha,
        highlights: highlights.into_iter().take(12).collect(),
        force: m.force,
        manual_only: offer == Offer::ManualOnly,
        rollout: m.rollout.unwrap_or(100).min(100),
        handles_silently: false,
    })
}

// ---------------------------------------------------------------- скачивание

struct DlConfig {
    idle: Duration,
    pauses: Vec<Duration>,
}

impl Default for DlConfig {
    fn default() -> Self {
        Self { idle: IDLE_TIMEOUT, pauses: RETRY_PAUSES.to_vec() }
    }
}

/// Итог одной попытки скачивания.
#[derive(Debug)]
enum Attempt {
    Done,
    /// Не удалось соединиться: пробуем мост (попытка не тратится).
    Bridge(String),
    /// Обрыв, простой, 5xx: повтор после паузы, докачка с места обрыва.
    Retry(String),
    /// Повторять бессмысленно (404, повреждённый файл, диск).
    Fatal(String),
}

/// `bytes 100-999/1000` → (100, Some(1000)).
fn content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

async fn hash_existing(path: PathBuf) -> Option<sha2::Sha256> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    tokio::task::spawn_blocking(move || {
        let mut f = std::fs::File::open(path).ok()?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf).ok()?;
            if n == 0 {
                return Some(h);
            }
            h.update(&buf[..n]);
        }
    })
    .await
    .ok()
    .flatten()
}

/// Одна попытка: продолжает `part`, если он есть и сервер принимает Range.
async fn attempt(
    proxy: Option<&str>,
    url: &str,
    part: &Path,
    want: &str,
    cfg: &DlConfig,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Attempt {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let c = match build_client(proxy, None) {
        Ok(c) => c,
        Err(e) => return Attempt::Fatal(e),
    };
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut req = c.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = match tokio::time::timeout(cfg.idle, req.send()).await {
        Err(_) => return Attempt::Retry(format!("сервер не отвечает {} с", cfg.idle.as_secs())),
        Ok(Err(e)) if e.is_connect() => return Attempt::Bridge(e.to_string()),
        Ok(Err(e)) => return Attempt::Retry(e.to_string()),
        Ok(Ok(r)) => r,
    };
    let status = resp.status().as_u16();
    let (resumed, total) = match status {
        206 => {
            let cr = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(content_range);
            match cr {
                Some((start, total)) if start == have && have > 0 => (true, total.unwrap_or(0)),
                _ => {
                    let _ = std::fs::remove_file(part);
                    return Attempt::Retry("сервер вернул не тот кусок файла, качаю заново".into());
                }
            }
        }
        // Сервер не знает Range (или файла ещё нет): с начала.
        200 => (false, resp.content_length().unwrap_or(0)),
        416 => {
            // Файл уже докачан раньше, но не был принят?
            if have > 0 && sha256_file(part).is_ok_and(|s| s == want) {
                return Attempt::Done;
            }
            let _ = std::fs::remove_file(part);
            return Attempt::Retry("сервер не принял докачку, качаю заново".into());
        }
        401 | 403 | 404 | 410 => return Attempt::Fatal(format!("Загрузка: сервер ответил {status}")),
        408 | 429 | 500..=599 => return Attempt::Retry(format!("сервер ответил {status}")),
        _ => return Attempt::Fatal(format!("Загрузка: сервер ответил {status}")),
    };
    let mut hasher = if resumed {
        match hash_existing(part.to_path_buf()).await {
            Some(h) => h,
            None => return Attempt::Fatal("Не удалось прочитать начатый файл обновления".into()),
        }
    } else {
        Sha256::new()
    };
    let mut opts = tokio::fs::OpenOptions::new();
    opts.create(true).write(true);
    if resumed {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    let mut file = match opts.open(part).await {
        Ok(f) => f,
        Err(e) => return Attempt::Fatal(format!("Не удалось сохранить установщик: {e}")),
    };
    let base = if resumed { have } else { 0 };
    let mut received = base;
    progress(received, total);
    let mut last = std::time::Instant::now();
    loop {
        match tokio::time::timeout(cfg.idle, resp.chunk()).await {
            Err(_) => {
                let _ = file.flush().await;
                return Attempt::Retry(format!("нет данных {} с", cfg.idle.as_secs()));
            }
            Ok(Err(e)) => {
                let _ = file.flush().await;
                return Attempt::Retry(e.to_string());
            }
            Ok(Ok(None)) => break,
            Ok(Ok(Some(chunk))) => {
                hasher.update(&chunk);
                if let Err(e) = file.write_all(&chunk).await {
                    return Attempt::Fatal(format!("Не удалось записать файл обновления: {e}"));
                }
                received += chunk.len() as u64;
                // Не чаще ~12 раз в секунду: событие на каждый кусок в 16 КБ
                // забивало бы IPC без пользы для глаза.
                if last.elapsed() >= Duration::from_millis(80) {
                    last = std::time::Instant::now();
                    progress(received, total);
                }
            }
        }
    }
    if let Err(e) = file.flush().await {
        return Attempt::Fatal(format!("Не удалось записать файл обновления: {e}"));
    }
    drop(file);
    if total > 0 && received != total {
        return Attempt::Retry(format!("обрыв: получено {received} из {total} байт"));
    }
    progress(received, total.max(received));
    if format!("{:x}", hasher.finalize()) != want {
        let _ = std::fs::remove_file(part);
        let msg = "Файл обновления повреждён (контрольная сумма не совпала) - не ставлю".to_string();
        // После докачки мог испортиться старый кусок: один раз с нуля.
        return if resumed { Attempt::Retry(msg) } else { Attempt::Fatal(msg) };
    }
    Attempt::Done
}

/// Скачать `url` в `dest` через `dest.part`: докачка, повторы, мост при
/// недоступности прямого соединения. Сумма проверяется на лету.
async fn download_file(
    bridge: &str,
    url: &str,
    dest: &Path,
    want: &str,
    cfg: &DlConfig,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(), String> {
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut via_bridge = false;
    let mut retries = 0usize;
    loop {
        match attempt(via_bridge.then_some(bridge), url, &part, want, cfg, progress).await {
            Attempt::Done => {
                let _ = std::fs::remove_file(dest);
                std::fs::rename(&part, dest).map_err(|e| format!("Не удалось сохранить установщик: {e}"))?;
                return Ok(());
            }
            Attempt::Bridge(msg) if !via_bridge => {
                tracing::info!("обновление: прямое соединение не вышло ({msg}), иду через мост");
                via_bridge = true;
            }
            Attempt::Bridge(msg) | Attempt::Retry(msg) => {
                if retries >= cfg.pauses.len() {
                    return Err(format!("Загрузка: {msg}"));
                }
                tracing::info!("обновление: {msg}; повтор через {} с", cfg.pauses[retries].as_secs());
                tokio::time::sleep(cfg.pauses[retries]).await;
                retries += 1;
            }
            Attempt::Fatal(msg) => return Err(msg),
        }
    }
}

// ---------------------------------------------------------------- кэш и папки

/// Кэш обновлений: …/ru.iimperium.clodhelper/update.
fn cache_root() -> Result<PathBuf, String> {
    Ok(dirs::cache_dir().ok_or("Нет папки кэша")?.join("ru.iimperium.clodhelper").join("update"))
}

/// Случайные 128 бит в hex: каждый RandomState берёт новые ключи у ОС.
pub(crate) fn random_hex() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(std::process::id() as u64);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Свежая папка для запуска установщика: имя случайное, создаётся только что
/// (create_dir, а не create_dir_all на существующую): подложить файлы заранее
/// в неё нельзя.
fn fresh_run_dir(root: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    for _ in 0..8 {
        let dir = root.join(format!("run-{}", random_hex()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Не удалось создать папку обновления: {e}")),
        }
    }
    Err("Не удалось создать папку обновления".into())
}

/// Копия файла с проверкой суммы по ходу копирования: запускается именно то,
/// что проверено (кэш лежит в папке, куда пишет пользователь).
fn copy_verified(src: &Path, dst: &Path, want: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let mut r = std::fs::File::open(src).map_err(|e| format!("Не удалось открыть установщик: {e}"))?;
    let mut w = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| format!("Не удалось сохранить установщик: {e}"))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        w.write_all(&buf[..n]).map_err(|e| format!("Не удалось сохранить установщик: {e}"))?;
    }
    drop(w);
    if format!("{:x}", h.finalize()) != want {
        let _ = std::fs::remove_file(dst);
        return Err("Файл обновления повреждён (контрольная сумма не совпала) - не ставлю".into());
    }
    Ok(())
}

/// В кэше оставляем только папку нужной версии; следы старых запусков
/// установщика (run-…) старше часа убираем.
fn prune_cache(root: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == keep {
            continue;
        }
        if name.starts_with("run-") {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d > Duration::from_secs(3600));
            if !old {
                continue;
            }
        }
        let p = e.path();
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
}

/// Скачивание идёт одно за раз: фоновая закачка и ручная установка той же
/// версии не должны писать в один файл.
static DL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Манифест ещё раз, и только он решает, что ставить: адресу и сумме из окна
/// не доверяем (и ту же версию, что видел пользователь).
async fn trusted_for(info: &UpdateInfo, mode: Mode) -> Result<UpdateInfo, String> {
    check(mode)
        .await?
        .filter(|t| t.version == info.version)
        .ok_or_else(|| "Обновление на сервере изменилось - проверьте ещё раз".to_string())
}

/// Скачать установщик (если ещё не скачан) в кэш и проверить его SHA-256.
/// Возвращает путь к проверенному файлу и доверенные данные обновления.
/// Качаем кусками и сообщаем ход в `progress`: без этого окно минутами
/// крутило спиннер, и обновление выглядело зависшим.
pub async fn fetch_installer(
    info: &UpdateInfo,
    mode: Mode,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
) -> Result<(PathBuf, UpdateInfo), String> {
    let trusted = trusted_for(info, mode).await?;
    if trusted.manual_only {
        return Err("Эта версия ставится вручную: скачайте установщик на странице релиза".into());
    }
    let _dl = DL_LOCK.lock().await;
    let root = cache_root()?;
    let dir = root.join(&trusted.version);
    prune_cache(&root, &trusted.version);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку обновления: {e}"))?;
    let file = urlencoding_decode(trusted.url.rsplit('/').next().unwrap_or_default());
    if file.is_empty() || file.contains(['/', '\\']) || file.contains("..") {
        return Err("Адрес обновления не годится".into());
    }
    let dest = dir.join(&file);
    let want = trusted.sha256.clone();
    // Уже скачан и сходится - ничего не качаем.
    if dest.exists() {
        let ok = {
            let d = dest.clone();
            tokio::task::spawn_blocking(move || sha256_file(&d).ok()).await.ok().flatten()
        };
        if ok.as_deref() == Some(want.as_str()) {
            progress(UpdateProgress { stage: "verify", received: 0, total: 0 });
            return Ok((dest, trusted));
        }
        let _ = std::fs::remove_file(&dest);
    }
    let report = |received: u64, total: u64| progress(UpdateProgress { stage: "download", received, total });
    download_file(&bridge_proxy(), &trusted.url, &dest, &want, &DlConfig::default(), &report).await?;
    progress(UpdateProgress { stage: "verify", received: 0, total: 0 });
    Ok((dest, trusted))
}

// ---------------------------------------------------------------- блокировка

static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Одновременно идёт одна установка обновления (ручная или тихая).
pub struct InstallLock;

impl InstallLock {
    pub fn acquire() -> Result<Self, String> {
        if INSTALLING.swap(true, Ordering::SeqCst) {
            Err(ALREADY.into())
        } else {
            Ok(Self)
        }
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

pub const ALREADY: &str = "Обновление уже устанавливается";
/// Тихая установка отложена: перед самым запуском человек вернулся к
/// компьютеру или открыл окно. Это не сбой: неудача не записывается.
pub const POSTPONED: &str = "Установка отложена";

pub fn is_installing() -> bool {
    INSTALLING.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------- установка

/// Скачать, проверить, поставить и перезапуститься (после вызова приложение
/// должно завершиться - новую версию запустит фоновый скрипт). `how` -
/// "manual" | "auto": тихая установка без окон и заставки, новая версия
/// стартует в трее.
pub async fn install(
    info: &UpdateInfo,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
    how: &str,
    ready: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(), String> {
    let lock = InstallLock::acquire()?;
    let silent = how == "auto";
    let mode = if silent { Mode::Background } else { Mode::Manual };
    let (cached, trusted) = fetch_installer(info, mode, progress).await?;
    let size = std::fs::metadata(&cached).map(|m| m.len()).unwrap_or(0);
    progress(UpdateProgress { stage: "install", received: size, total: size });
    // Запускаем копию из новой папки со случайным именем, а не файл из кэша.
    let run = fresh_run_dir(&cache_root()?)?;
    let name = cached.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let staged = run.join(name);
    copy_verified(&cached, &staged, &trusted.sha256)?;
    // Тихая установка: условия могли измениться, пока шли проверки и копия.
    if silent && !ready() {
        let _ = std::fs::remove_dir_all(&run);
        return Err(POSTPONED.into());
    }
    platform_install(&run, &staged, &trusted, silent, how)?;
    // Установка пошла: блокировка остаётся до выхода приложения, чтобы за
    // паузу перед выходом не запустилась вторая.
    std::mem::forget(lock);
    Ok(())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
/// Путь программы, которую запустит скрипт после установки (отладка:
/// CLOD_UPDATE_RESTART_EXE - для живой проверки из тестового процесса).
fn restart_exe() -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("CLOD_UPDATE_RESTART_EXE") {
        return Ok(PathBuf::from(p));
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Сколько ждём выхода программы перед заменой (macOS), секунд.
#[cfg_attr(target_os = "windows", allow(dead_code))]
const SWAP_WAIT_SECS: u32 = 120;

/// Скрипт замены `.app` после выхода программы (macOS). Ждёт выхода не
/// дольше SWAP_WAIT_SECS, пишет журнал, оставляет одну резервную копию.
#[cfg_attr(target_os = "windows", allow(dead_code))]
struct SwapScript<'a> {
    pid: u32,
    log: &'a str,
    backup_dir: &'a str,
    backup_app: &'a str,
    app: &'a str,
    staged: &'a str,
    silent: bool,
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
impl SwapScript<'_> {
    /// Пути уже в кавычках (sh_quote).
    fn render(&self) -> String {
        let steps = SWAP_WAIT_SECS * 10 / 3; // шаг ожидания 0.3 с
        let open = if self.silent {
            format!("open -g {} --args --tray", self.app)
        } else {
            format!("open {}", self.app)
        };
        format!(
            "log={log}\nmkdir -p \"$(dirname \"$log\")\"\nexec >> \"$log\" 2>&1\n\
             echo \"$(date '+%F %T') ждём выхода программы, pid={pid}\"\n\
             pid={pid}\ni=0\n\
             while kill -0 $pid 2>/dev/null; do\n  i=$((i+1))\n  if [ $i -gt {steps} ]; then\n    echo \"$(date '+%F %T') программа не вышла за {wait} с, замену отменяю\"\n    rm -rf {new}\n    exit 1\n  fi\n  sleep 0.3\ndone\n\
             mkdir -p {bdir}\nrm -rf {bold}\n\
             if mv {app} {bold}; then\n  if mv {new} {app}; then\n    for f in {bdir}/*.app; do [ \"$f\" = {bold} ] || rm -rf \"$f\"; done\n    echo \"$(date '+%F %T') версия заменена\"\n  else\n    mv {bold} {app}\n    echo \"$(date '+%F %T') не удалось поставить новую, вернул старую\"\n  fi\nelse\n  echo \"$(date '+%F %T') не удалось переместить старую версию\"\n  rm -rf {new}\nfi\n\
             xattr -dr com.apple.quarantine {app} 2>/dev/null\n{open}\n\
             echo \"$(date '+%F %T') запуск: готово\"\n",
            log = self.log,
            pid = self.pid,
            steps = steps,
            wait = SWAP_WAIT_SECS,
            new = self.staged,
            bdir = self.backup_dir,
            bold = self.backup_app,
            app = self.app,
            open = open,
        )
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_install(run: &Path, dmg: &Path, info: &UpdateInfo, silent: bool, how: &str) -> Result<(), String> {
    // Где стоит текущее приложение: …/Clod Helper.app/Contents/MacOS/clod-helper
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle: PathBuf = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(PathBuf::from)
        .ok_or("Обновление работает только для установленного приложения (.app)")?;

    // Смонтировать, скопировать новую версию рядом, отмонтировать.
    let mount = run.join("mnt");
    let ok = std::process::Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount)
        .arg(dmg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Не удалось открыть образ обновления".into());
    }
    let staged = bundle.with_extension("app.new");
    let _ = std::fs::remove_dir_all(&staged);
    let copied = std::process::Command::new("/usr/bin/ditto")
        .arg(mount.join("Clod Helper.app"))
        .arg(&staged)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::process::Command::new("/usr/bin/hdiutil").args(["detach", "-quiet"]).arg(&mount).status();
    if !copied {
        return Err("Не удалось распаковать обновление".into());
    }

    // Замена после выхода: старую - в резервную копию (одну последнюю),
    // новую - на место.
    let backup_dir = dirs::data_dir().ok_or("Нет папки данных")?.join("ClodHelper-backup");
    let _ = std::fs::create_dir_all(&backup_dir);
    let log = dirs::home_dir().ok_or("Нет домашней папки")?.join("Library/Logs/ClodHelper/update-swap.log");
    let q = |p: &Path| crate::macos::sh_quote(&p.to_string_lossy());
    let backup_app = backup_dir.join(format!("Clod Helper ({}).app", env!("CARGO_PKG_VERSION")));
    let script = SwapScript {
        pid: std::process::id(),
        log: &q(&log),
        backup_dir: &q(&backup_dir),
        backup_app: &q(&backup_app),
        app: &q(&bundle),
        staged: &q(&staged),
        silent,
    }
    .render();
    let sh = run.join("swap.sh");
    std::fs::write(&sh, script).map_err(|e| e.to_string())?;
    crate::auto_update::mark_attempt(&info.version, how);
    let spawned = std::process::Command::new("/usr/bin/nohup")
        .arg("/bin/sh")
        .arg(&sh)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = spawned {
        crate::auto_update::clear_attempt();
        return Err(format!("Не удалось запустить установку: {e}"));
    }
    Ok(())
}

/// Ключи NSIS-установщика Tauri (как у его штатного апдейтера): /S - без
/// окон (белое окно /P выглядело чужеродно - вместо него наша заставка,
/// update_splash.rs); /UPDATE - поверх текущей версии, без её удаления и
/// без пересоздания ярлыков, которые пользователь мог убрать; /R -
/// запустить новую версию после установки.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const NSIS_UPDATE_ARGS: [&str; 3] = ["/S", "/UPDATE", "/R"];

/// Тихая установка: без /R (окно не нужно) и без заставки; новую версию в
/// трее запускает батник после установщика.
#[allow(dead_code)]
const NSIS_SILENT_ARGS: [&str; 2] = ["/S", "/UPDATE"];

/// Батник тихого обновления. Только ASCII: пути приходят аргументами
/// командной строки (она Юникодная), а не лежат в файле - имена пользователей
/// с кириллицей не ломают разбор. %1 - установщик, %2 - exe для запуска.
/// Установщик NSIS закрывает процессы только по имени clod-helper.exe, cmd.exe
/// он не трогает. Если установщик не справился, запускается прежняя версия.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const SILENT_BAT: &str = "@echo off\r\nrem Clod Helper: silent update. %1 = installer, %2 = app exe\r\nstart \"\" /wait \"%~1\" /S /UPDATE\r\nstart \"\" \"%~2\" --tray\r\n";

/// Полный путь к cmd.exe из %SystemRoot%\System32 (не по PATH: туда мог
/// попасть чужой cmd.exe). Нет переменной - обычное имя.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn system_cmd(root: Option<std::ffi::OsString>) -> PathBuf {
    match root.filter(|r| !r.is_empty()) {
        Some(r) => PathBuf::from(r).join("System32").join("cmd.exe"),
        None => PathBuf::from("cmd.exe"),
    }
}

/// Командная строка для `cmd.exe /d /s /c`: внешние кавычки снимает /s.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn silent_cmdline(bat: &Path, setup: &Path, exe: &Path) -> String {
    format!("/d /s /c \"\"{}\" \"{}\" \"{}\"\"", bat.display(), setup.display(), exe.display())
}

/// Windows: установка в профиль пользователя (currentUser) - без UAC.
/// Приложение сразу после этого выходит (update_install); установщик
/// дождётся выхода - запущенную копию он и сам закрывает. Мост работает из
/// своей папки и обновится при первом старте новой версии.
#[cfg(target_os = "windows")]
fn platform_install(run: &Path, setup: &Path, info: &UpdateInfo, silent: bool, how: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    /// Без окна консоли.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let spawned = if silent {
        let exe = restart_exe()?;
        let bat = run.join("update.cmd");
        std::fs::write(&bat, SILENT_BAT).map_err(|e| format!("Не удалось записать скрипт обновления: {e}"))?;
        crate::auto_update::mark_attempt(&info.version, how);
        std::process::Command::new(system_cmd(std::env::var_os("SystemRoot")))
            .raw_arg(silent_cmdline(&bat, setup, &exe))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    } else {
        crate::auto_update::mark_attempt(&info.version, how);
        std::process::Command::new(setup).args(NSIS_UPDATE_ARGS).spawn()
    };
    let child = match spawned {
        Ok(c) => c,
        Err(e) => {
            crate::auto_update::clear_attempt();
            return Err(format!("Не удалось запустить установщик: {e}"));
        }
    };
    if !silent {
        // Заставка - украшение: не запустилась, обновление всё равно идёт.
        if let Err(e) = crate::update_splash::spawn(run, child.id(), &info.version) {
            tracing::warn!("{e}");
        }
    }
    Ok(())
}

/// %20 → пробел и т.п. (имена ассетов GitHub в URL экранированы).
fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    fn keypair(seed: u8) -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let pk = base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().to_bytes());
        (sk, pk)
    }

    fn signed(sk: &SigningKey, body: &[u8]) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.encode(sk.sign(body).to_bytes()).into_bytes()
    }

    #[test]
    fn manifest_signature_is_required_and_checked() {
        let (sk, pk) = keypair(7);
        let (other, other_pk) = keypair(9);
        let body = br#"{"version":"0.9.0"}"#;
        assert!(verify_manifest(body, &signed(&sk, body), &[&pk]).is_ok());
        // Ключ на смену: годится любой из списка.
        assert!(verify_manifest(body, &signed(&sk, body), &[&other_pk, &pk]).is_ok());
        // Чужой ключ, изменённый манифест, мусор вместо подписи - отказ.
        assert!(verify_manifest(body, &signed(&other, body), &[&pk]).is_err());
        assert!(verify_manifest(br#"{"version":"9.9.9"}"#, &signed(&sk, body), &[&pk]).is_err());
        assert!(verify_manifest(body, b"not base64!", &[&pk]).is_err());
        assert!(verify_manifest(body, b"", &[&pk]).is_err());
    }

    #[test]
    fn real_update_keys_are_valid_and_distinct() {
        assert_eq!(UPDATE_KEYS.len(), 2, "основной и резервный");
        for k in UPDATE_KEYS {
            let raw = base64::engine::general_purpose::STANDARD.decode(k).unwrap();
            let raw: [u8; 32] = raw.try_into().unwrap();
            assert!(ed25519_dalek::VerifyingKey::from_bytes(&raw).is_ok());
        }
        assert_ne!(UPDATE_KEYS[0], UPDATE_KEYS[1]);
    }

    const NOW: u64 = 1_795_000_000; // конец ноября 2026
    fn ctx(mode: Mode, number: u8) -> Ctx {
        Ctx { mode, number, now: NOW }
    }
    fn bg() -> Ctx {
        ctx(Mode::Background, 10)
    }

    fn manifest(version: &str, tag: &str, file: &str, sha: &str) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "version": version, "tag": tag, "highlights": ["Новое"],
            "platforms": {
                "windows-x86_64": { "file": file, "sha256": sha },
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64) },
            }
        }))
        .unwrap()
    }

    fn manifest_with(extra: serde_json::Value) -> Manifest {
        let mut v = serde_json::json!({
            "version": "0.9.0", "tag": "v0.9.0", "highlights": ["Новое"],
            "platforms": { "windows-x86_64": { "file": "Clod.Helper_0.9.0_x64-setup.exe", "sha256": "a".repeat(64) } }
        });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn cmd_comes_from_system32() {
        assert_eq!(system_cmd(Some("C:\\Windows".into())), PathBuf::from("C:\\Windows").join("System32").join("cmd.exe"));
        assert_eq!(system_cmd(None), PathBuf::from("cmd.exe"));
        assert_eq!(system_cmd(Some("".into())), PathBuf::from("cmd.exe"));
    }

    #[test]
    fn manifest_seq_never_goes_back() {
        // Больше или равно сохранённому - берём, меньше - откат, отвергаем.
        assert!(seq_ok(Some(100), Some(100)));
        assert!(seq_ok(Some(101), Some(100)));
        assert!(!seq_ok(Some(99), Some(100)));
        // Без seq: можно, только пока ничего не сохранено.
        assert!(seq_ok(None, None));
        assert!(!seq_ok(None, Some(100)));
        assert!(seq_ok(Some(5), None));
        // Поле читается из подписанного манифеста.
        assert_eq!(manifest_with(serde_json::json!({"seq": 1_790_000_000u64})).seq, Some(1_790_000_000));
        assert_eq!(manifest_with(serde_json::json!({})).seq, None);
    }

    #[test]
    fn update_comes_only_from_a_consistent_newer_manifest() {
        let sha = "A".repeat(64);
        let exe = "Clod.Helper_0.9.0_x64-setup.exe";
        let win = update_from(manifest("0.9.0", "v0.9.0", exe, &sha), "0.8.8", "windows-x86_64", &bg()).unwrap();
        assert_eq!(win.url, format!("https://github.com/{REPO}/releases/download/v0.9.0/{exe}"));
        assert_eq!(win.page, format!("https://github.com/{REPO}/releases/tag/v0.9.0"));
        assert_eq!(win.sha256, "a".repeat(64));
        assert_eq!(win.highlights, ["Новое"]);
        assert_eq!((win.force, win.manual_only, win.rollout), (false, false, 100));
        let mac = update_from(manifest("0.9.0", "v0.9.0", exe, &sha), "0.8.8", "darwin-universal", &bg()).unwrap();
        assert!(mac.url.ends_with("_universal.dmg"));
        // Не новее; тег не тот; путь в имени файла; не сумма; нет своей ОС.
        let none = |m: Manifest, p: &str| update_from(m, "0.8.8", p, &bg()).is_none();
        assert!(update_from(manifest("0.8.8", "v0.8.8", exe, &sha), "0.8.8", "windows-x86_64", &bg()).is_none());
        assert!(none(manifest("0.9.0", "v0.9.1", exe, &sha), "windows-x86_64"));
        assert!(none(manifest("0.9.0", "v0.9.0", "../evil.exe", &sha), "windows-x86_64"));
        assert!(none(manifest("0.9.0", "v0.9.0", exe, "abc"), "windows-x86_64"));
        assert!(none(manifest("0.9.0", "v0.9.0", exe, &sha), "linux"));

        // Свои пункты у ОС: пусто - для этой ОС ничего нового, не предлагаем.
        let per_os: Manifest = serde_json::from_value(serde_json::json!({
            "version": "0.9.0", "tag": "v0.9.0", "highlights": ["Только Windows"],
            "platforms": {
                "windows-x86_64": { "file": exe, "sha256": sha, "highlights": ["Только Windows"] },
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64), "highlights": [] },
            }
        }))
        .unwrap();
        assert!(none(per_os, "darwin-universal"), "Mac не зовём на версию только для Windows");
        let per_os: Manifest = serde_json::from_value(serde_json::json!({
            "version": "0.9.0", "tag": "v0.9.0", "highlights": ["Общее", "Для Mac"],
            "platforms": {
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64), "highlights": ["Общее", "Для Mac"] },
                "windows-x86_64": { "file": exe, "sha256": sha, "highlights": ["Общее"] },
            }
        }))
        .unwrap();
        assert_eq!(update_from(per_os, "0.8.8", "windows-x86_64", &bg()).unwrap().highlights, ["Общее"]);
        assert!(parse_ver("v0.7.0") > parse_ver("0.6.0"));
        assert!(parse_ver("0.6.10") > parse_ver("0.6.9"));
        assert_eq!(urlencoding_decode("Clod%20Helper_0.6.0.dmg"), "Clod Helper_0.6.0.dmg");
    }

    #[test]
    fn manifest_controls_are_optional_and_parsed() {
        // Старый манифест без новых полей: всё по умолчанию.
        let m = manifest_with(serde_json::json!({}));
        assert!(!m.paused && !m.force && m.rollout.is_none() && m.min_version.is_none());
        let u = update_from(m, "0.8.8", "windows-x86_64", &bg()).unwrap();
        assert_eq!((u.rollout, u.force, u.manual_only), (100, false, false));
        // Новые поля читаются.
        let m = manifest_with(serde_json::json!({"rollout": 40, "paused": false, "min_version": "0.8.0", "force": true, "pub_date": "2026-11-20T10:00:00.000Z"}));
        assert_eq!((m.rollout, m.force, m.min_version.as_deref()), (Some(40), true, Some("0.8.0")));
        assert_eq!(m.pub_date.as_deref(), Some("2026-11-20T10:00:00.000Z"));
    }

    #[test]
    fn paused_hides_update_in_every_mode() {
        for mode in [Mode::Background, Mode::Manual] {
            let m = manifest_with(serde_json::json!({"paused": true}));
            assert!(update_from(m, "0.8.8", "windows-x86_64", &ctx(mode, 0)).is_none());
        }
    }

    #[test]
    fn rollout_applies_to_background_only() {
        let at = |rollout: u32, mode: Mode, number: u8| {
            let m = manifest_with(serde_json::json!({ "rollout": rollout }));
            update_from(m, "0.8.8", "windows-x86_64", &ctx(mode, number)).is_some()
        };
        assert!(at(30, Mode::Background, 29));
        assert!(!at(30, Mode::Background, 30));
        assert!(!at(0, Mode::Background, 0));
        assert!(at(100, Mode::Background, 99));
        // Ручная проверка раскатку не смотрит.
        assert!(at(0, Mode::Manual, 99));
        assert!(at(30, Mode::Manual, 80));
    }

    #[test]
    fn min_version_makes_update_manual_only() {
        let m = manifest_with(serde_json::json!({"min_version": "0.8.5"}));
        let old = update_from(m, "0.8.1", "windows-x86_64", &bg()).unwrap();
        assert!(old.manual_only, "старая установка: только вручную");
        assert!(old.page.ends_with("/releases/tag/v0.9.0"));
        let m = manifest_with(serde_json::json!({"min_version": "0.8.5"}));
        assert!(!update_from(m, "0.8.5", "windows-x86_64", &bg()).unwrap().manual_only, "ровно min_version - обычная");
        let m = manifest_with(serde_json::json!({"min_version": "0.8.5"}));
        assert!(update_from(m, "0.8.1", "windows-x86_64", &ctx(Mode::Manual, 0)).unwrap().manual_only);
    }

    #[test]
    fn force_installs_even_when_whats_new_is_empty() {
        let v = |force: bool| -> Manifest {
            serde_json::from_value(serde_json::json!({
                "version": "0.9.0", "tag": "v0.9.0", "highlights": [], "force": force,
                "platforms": { "windows-x86_64": { "file": "Clod.Helper_0.9.0_x64-setup.exe", "sha256": "a".repeat(64), "highlights": [] } }
            }))
            .unwrap()
        };
        assert!(update_from(v(false), "0.8.8", "windows-x86_64", &bg()).is_none(), "пусто и без force - не предлагаем");
        let u = update_from(v(true), "0.8.8", "windows-x86_64", &bg()).unwrap();
        assert!(u.force);
        assert_eq!(u.highlights, ["Срочное исправление"]);
    }

    #[test]
    fn stale_manifest_is_ignored() {
        let at = |date: &str| {
            let m = manifest_with(serde_json::json!({ "pub_date": date }));
            update_from(m, "0.8.8", "windows-x86_64", &bg()).is_some()
        };
        assert!(at("2026-11-20T10:00:00.000Z"));
        assert!(at("2025-11-01T00:00:00Z"), "год назад - ещё свежий");
        assert!(!at("2025-01-01T00:00:00Z"), "старше 400 дней");
        assert!(at("мусор"), "дата не читается - не отбрасываем");
        assert!(manifest_stale(Some("2020-01-01"), NOW));
        assert!(!manifest_stale(None, NOW));
        assert!(!manifest_stale(Some("2099-01-01"), NOW), "дата из будущего (часы) - не отбрасываем");
        assert_eq!(iso_day_secs("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(iso_day_secs("2026-11-21T01:02:03Z"), Some(1_795_219_200));
    }

    #[test]
    fn tags_are_strict() {
        assert!(tag_ok("v0.8.19"));
        for bad in ["0.8.19", "v0.8", "v1.2.3/../x", "v1.2.3\n", "vx.y.z", "v1.2.3.4", "v", ""] {
            assert!(!tag_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn windows_update_is_silent_in_place_and_restarts() {
        assert_eq!(NSIS_UPDATE_ARGS, ["/S", "/UPDATE", "/R"]);
        // Тихая установка: без /R - окно не открывается.
        assert_eq!(NSIS_SILENT_ARGS, ["/S", "/UPDATE"]);
        assert!(SILENT_BAT.contains("/wait \"%~1\" /S /UPDATE\r\n"));
        assert!(!SILENT_BAT.contains("/R"));
        assert!(SILENT_BAT.contains("start \"\" \"%~2\" --tray"), "новая версия стартует в трее");
        assert!(SILENT_BAT.is_ascii(), "батник без кириллицы: пути идут аргументами");
        let line = silent_cmdline(Path::new(r"C:\Users\Пётр\run-1\update.cmd"), Path::new(r"C:\a b\setup.exe"), Path::new(r"C:\Clod Helper\clod-helper.exe"));
        assert_eq!(
            line,
            "/d /s /c \"\"C:\\Users\\Пётр\\run-1\\update.cmd\" \"C:\\a b\\setup.exe\" \"C:\\Clod Helper\\clod-helper.exe\"\""
        );
    }

    #[test]
    fn mac_swap_script_is_bounded_logged_and_quiet() {
        let s = |silent| {
            SwapScript {
                pid: 4242,
                log: "'/L/update-swap.log'",
                backup_dir: "'/B'",
                backup_app: "'/B/Clod Helper (0.8.18).app'",
                app: "'/A/Clod Helper.app'",
                staged: "'/A/Clod Helper.app.new'",
                silent,
            }
            .render()
        };
        let quiet = s(true);
        assert!(quiet.contains("open -g '/A/Clod Helper.app' --args --tray"));
        assert!(!s(false).contains("--tray"));
        assert!(s(false).contains("\nopen '/A/Clod Helper.app'\n"));
        // Ожидание выхода ограничено (400 шагов по 0.3 с = 120 с) и пишется журнал.
        assert!(quiet.contains("-gt 400"));
        assert!(quiet.contains("sleep 0.3"));
        assert!(quiet.contains("exec >> \"$log\" 2>&1"));
        assert!(quiet.contains("log='/L/update-swap.log'"));
        // Лишние резервные копии убираются, остаётся предыдущая (она же bold).
        assert!(quiet.contains("for f in '/B'/*.app; do [ \"$f\" = '/B/Clod Helper (0.8.18).app' ] || rm -rf \"$f\"; done"));
        // Откат, если новая не встала.
        assert!(quiet.contains("mv '/B/Clod Helper (0.8.18).app' '/A/Clod Helper.app'"));
    }

    #[test]
    fn run_dir_is_fresh_unique_and_copy_is_verified() {
        let root = std::env::temp_dir().join(format!("clod-run-{}-{}", std::process::id(), random_hex()));
        let a = fresh_run_dir(&root).unwrap();
        let b = fresh_run_dir(&root).unwrap();
        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("run-"));
        assert!(a.file_name().unwrap().len() >= 4 + 32, "случайное имя из 128 бит");
        // Копия: сумма сходится - файл есть; нет - удалён.
        use sha2::{Digest, Sha256};
        let src = root.join("src.bin");
        std::fs::write(&src, b"installer bytes").unwrap();
        let want = format!("{:x}", Sha256::digest(b"installer bytes"));
        let ok = a.join("ok.bin");
        copy_verified(&src, &ok, &want).unwrap();
        assert_eq!(std::fs::read(&ok).unwrap(), b"installer bytes");
        let bad = a.join("bad.bin");
        assert!(copy_verified(&src, &bad, &"0".repeat(64)).is_err());
        assert!(!bad.exists());
        // Существующий файл не перезаписывается (create_new).
        assert!(copy_verified(&src, &ok, &want).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_lock_rejects_second_install() {
        // Общий флаг: тест единственный, который его трогает напрямую.
        let first = InstallLock::acquire().unwrap();
        assert!(is_installing());
        assert_eq!(InstallLock::acquire().err().as_deref(), Some("Обновление уже устанавливается"));
        drop(first);
        assert!(!is_installing());
        let again = InstallLock::acquire();
        assert!(again.is_ok(), "после освобождения можно снова");
    }

    #[test]
    fn content_range_is_parsed() {
        assert_eq!(content_range("bytes 100-999/1000"), Some((100, Some(1000))));
        assert_eq!(content_range("bytes 0-9/*"), Some((0, None)));
        assert_eq!(content_range("items 1-2/3"), None);
    }

    // ------------------------------------------------ тестовый HTTP-сервер

    /// Ответ тестового сервера на один запрос.
    #[derive(Clone)]
    struct Reply {
        status: u16,
        body: Vec<u8>,
        /// Заголовок Content-Range для 206.
        range: Option<String>,
        /// Заявить Content-Length тела целиком, но отдать только n байт и
        /// закрыть соединение (обрыв).
        cut_after: Option<usize>,
        /// Отдать n байт и замолчать (простой).
        stall_after: Option<usize>,
    }

    impl Reply {
        fn ok(body: &[u8]) -> Self {
            Self { status: 200, body: body.to_vec(), range: None, cut_after: None, stall_after: None }
        }
    }

    struct Req {
        path: String,
        range_start: Option<u64>,
        conn: usize,
    }

    /// Мини-HTTP-сервер: `handler` решает ответ на каждый запрос. Возвращает
    /// порт и журнал запросов (путь, Range).
    fn serve(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> (u16, Arc<Mutex<Vec<(String, Option<u64>)>>>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log: Arc<Mutex<Vec<(String, Option<u64>)>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for (conn, s) in l.incoming().flatten().enumerate() {
                let (handler, log) = (handler.clone(), log2.clone());
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match s.read(&mut tmp) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let range_start = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string))
                        .and_then(|v| v.trim_end_matches('-').split('-').next().and_then(|n| n.parse().ok()));
                    log.lock().unwrap().push((path.clone(), range_start));
                    let r = handler(&Req { path, range_start, conn });
                    let reason = match r.status {
                        200 => "OK",
                        206 => "Partial Content",
                        404 => "Not Found",
                        _ => "X",
                    };
                    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", r.status, r.body.len());
                    if let Some(cr) = &r.range {
                        head.push_str(&format!("Content-Range: {cr}\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = s.write_all(head.as_bytes());
                    let n = r.cut_after.or(r.stall_after).unwrap_or(r.body.len()).min(r.body.len());
                    let _ = s.write_all(&r.body[..n]);
                    let _ = s.flush();
                    if r.stall_after.is_some() {
                        std::thread::sleep(Duration::from_secs(5));
                    }
                });
            }
        });
        (port, log)
    }

    /// Ответ на Range-запрос из данных `data`.
    fn ranged(data: &[u8], r: &Req) -> Reply {
        match r.range_start {
            Some(s) if (s as usize) < data.len() => Reply {
                status: 206,
                body: data[s as usize..].to_vec(),
                range: Some(format!("bytes {s}-{}/{}", data.len() - 1, data.len())),
                cut_after: None,
                stall_after: None,
            },
            _ => Reply::ok(data),
        }
    }

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn sha_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(data))
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clod-upd-{name}-{}-{}", std::process::id(), random_hex()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fast_cfg() -> DlConfig {
        DlConfig { idle: Duration::from_millis(600), pauses: vec![Duration::from_millis(20), Duration::from_millis(20)] }
    }

    fn dead_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn download_resumes_after_cut_with_range() {
        let data = payload(200_000);
        let want = sha_hex(&data);
        let d2 = data.clone();
        let (port, log) = serve(move |r| {
            if r.conn == 0 {
                // Первое соединение: обрыв посередине.
                Reply { cut_after: Some(70_000), ..ranged(&d2, r) }
            } else {
                ranged(&d2, r)
            }
        });
        let dir = tmp("resume");
        let dest = dir.join("setup.exe");
        let url = format!("http://127.0.0.1:{port}/setup.exe");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let progress = move |got: u64, total: u64| s2.lock().unwrap().push((got, total));
        download_file(&format!("http://127.0.0.1:{}", dead_port()), &url, &dest, &want, &fast_cfg(), &progress)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data, "файл целый, сумма сошлась");
        assert!(!PathBuf::from(format!("{}.part", dest.display())).exists());
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2, "обрыв и одна докачка");
        assert_eq!(log[0].1, None);
        assert_eq!(log[1].1, Some(70_000), "докачка с места обрыва через Range");
        assert_eq!(seen.lock().unwrap().last().copied(), Some((200_000, 200_000)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_resumes_a_part_left_by_a_previous_run() {
        let data = payload(100_000);
        let want = sha_hex(&data);
        let d2 = data.clone();
        let (port, log) = serve(move |r| ranged(&d2, r));
        let dir = tmp("leftover");
        let dest = dir.join("setup.exe");
        std::fs::write(format!("{}.part", dest.display()), &data[..40_000]).unwrap();
        download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dest, &want, &fast_cfg(), &|_, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(log.lock().unwrap()[0].1, Some(40_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_restarts_when_server_ignores_range() {
        let data = payload(50_000);
        let want = sha_hex(&data);
        let d2 = data.clone();
        let (port, _) = serve(move |r| if r.conn == 0 { Reply { cut_after: Some(10_000), ..Reply::ok(&d2) } } else { Reply::ok(&d2) });
        let dir = tmp("norange");
        let dest = dir.join("f.bin");
        download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dest, &want, &fast_cfg(), &|_, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data, "сервер без Range отдал всё заново, файл не склеен криво");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_gives_up_after_three_attempts() {
        let data = payload(30_000);
        let want = sha_hex(&data);
        let d2 = data.clone();
        // Каждое соединение рвётся, не доходя до конца, но чуть двигается вперёд.
        let (port, log) = serve(move |r| {
            Reply { cut_after: Some(1000), ..ranged(&d2, r) }
        });
        let dir = tmp("giveup");
        let dest = dir.join("f.bin");
        let e = download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dest, &want, &fast_cfg(), &|_, _| {})
            .await
            .unwrap_err();
        assert!(e.starts_with("Загрузка:"), "{e}");
        assert_eq!(log.lock().unwrap().len(), 3, "три попытки: первая и две повторные");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_idle_timeout_then_resume() {
        let data = payload(80_000);
        let want = sha_hex(&data);
        let d2 = data.clone();
        let (port, log) = serve(move |r| if r.conn == 0 { Reply { stall_after: Some(30_000), ..ranged(&d2, r) } } else { ranged(&d2, r) });
        let dir = tmp("stall");
        let dest = dir.join("f.bin");
        let started = std::time::Instant::now();
        download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dest, &want, &fast_cfg(), &|_, _| {})
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(4), "ждали простой, а не весь сон сервера");
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(log.lock().unwrap()[1].1, Some(30_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_fails_fast_on_404_and_bad_checksum() {
        let (port, log) = serve(|_| Reply { status: 404, ..Reply::ok(b"") });
        let dir = tmp("fatal");
        let e = download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dir.join("a"), &"0".repeat(64), &fast_cfg(), &|_, _| {})
            .await
            .unwrap_err();
        assert!(e.contains("404"), "{e}");
        assert_eq!(log.lock().unwrap().len(), 1, "404 не повторяем");
        // Сумма не сошлась: файл удалён, без повторов.
        let (port, log) = serve(|_| Reply::ok(b"not what we wanted"));
        let dest = dir.join("b");
        let e = download_file("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}/f"), &dest, &"0".repeat(64), &fast_cfg(), &|_, _| {})
            .await
            .unwrap_err();
        assert!(e.contains("контрольная сумма"), "{e}");
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(!dest.exists() && !PathBuf::from(format!("{}.part", dest.display())).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Мини-прокси (как мост): на любой запрос отдаёт `body`; журнал - это и есть обращения к мосту.
    fn fake_bridge(body: Vec<u8>) -> (u16, Arc<Mutex<Vec<(String, Option<u64>)>>>) {
        serve(move |_| Reply::ok(&body))
    }

    #[tokio::test]
    async fn direct_first_then_bridge_only_when_cannot_connect() {
        let data = b"hello update".to_vec();
        // Прямое соединение работает: мост не трогаем (адрес моста - закрытый порт).
        let d2 = data.clone();
        let (port, _) = serve(move |_| Reply::ok(&d2));
        let got = get_small(&format!("http://127.0.0.1:{}", dead_port()), &format!("http://127.0.0.1:{port}/m")).await.ok().unwrap();
        assert_eq!(got, data);
        // Прямо не соединиться (порт закрыт): идём через мост.
        let (bridge, hits) = fake_bridge(b"via bridge".to_vec());
        let url = format!("http://127.0.0.1:{}/m", dead_port());
        let got = get_small(&format!("http://127.0.0.1:{bridge}"), &url).await.ok().unwrap();
        assert_eq!(got, b"via bridge");
        assert!(!hits.lock().unwrap().is_empty(), "мост вызван");
        // HTTP-ошибка прямого ответа - не повод идти через мост.
        let (port, _) = serve(|_| Reply { status: 404, ..Reply::ok(b"") });
        let (bridge, hits) = fake_bridge(b"never".to_vec());
        let e = get_small(&format!("http://127.0.0.1:{bridge}"), &format!("http://127.0.0.1:{port}/m")).await.err().unwrap();
        assert!(e.not_found);
        assert!(hits.lock().unwrap().is_empty(), "мост не вызывался");
    }

    #[tokio::test]
    async fn download_goes_to_bridge_when_direct_cannot_connect() {
        let data = payload(10_000);
        let want = sha_hex(&data);
        let (bridge, _) = fake_bridge(data.clone());
        // Закрытый порт на Windows отказывает не сразу (до 2 с): простой не короче.
        let slow = || DlConfig { idle: Duration::from_secs(8), ..fast_cfg() };
        let dir = tmp("viabridge");
        let dest = dir.join("f.bin");
        let url = format!("http://127.0.0.1:{}/f", dead_port());
        download_file(&format!("http://127.0.0.1:{bridge}"), &url, &dest, &want, &slow(), &|_, _| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        // Нет ни прямого пути, ни моста: ошибка после повторов, не зависание.
        let e = download_file(&format!("http://127.0.0.1:{}", dead_port()), &url, &dir.join("g"), &want, &slow(), &|_, _| {}).await.unwrap_err();
        assert!(e.starts_with("Загрузка:"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn signature_is_taken_by_tag_and_retried_once_by_tag() {
        let (sk, pk) = keypair(21);
        let (evil, _) = keypair(22);
        let body = br#"{"version":"0.9.0","tag":"v0.9.0","highlights":["x"],"platforms":{"windows-x86_64":{"file":"a.exe","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}"#.to_vec();
        let good_sig = signed(&sk, &body);
        let (b2, g2) = (body.clone(), good_sig.clone());
        // По latest лежит другой (устаревший) манифест того же тега.
        let mut stale = body.clone();
        stale.push(b' ');
        // Кэш CDN: по latest лежит манифест без нужной подписи (подпись чужая),
        // по тегу - всё верно.
        let (port, log) = serve(move |r| match r.path.as_str() {
            "/latest/download/latest.json" => Reply::ok(&stale),
            "/download/v0.9.0/latest.json" => Reply::ok(&b2),
            "/download/v0.9.0/latest.json.sig" => Reply::ok(&g2),
            _ => Reply { status: 404, ..Reply::ok(b"") },
        });
        let base = format!("http://127.0.0.1:{port}");
        let m = fetch_manifest_from("http://127.0.0.1:1", &base, &[&pk]).await.unwrap();
        assert_eq!(m.tag, "v0.9.0");
        // Подпись запрашивалась по тегу, не по latest.
        let paths: Vec<String> = log.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
        assert!(paths.contains(&"/download/v0.9.0/latest.json.sig".to_string()));
        assert!(!paths.iter().any(|p| p.starts_with("/latest/download/latest.json.")));
        assert_eq!(paths.len(), 4, "latest, подпись по тегу, затем повтор: манифест по тегу и его подпись");
        // Подпись не та и по тегу тоже: отказ после одного повтора.
        let bad = signed(&evil, &body);
        let b3 = body.clone();
        let (port, log) = serve(move |r| match r.path.as_str() {
            "/latest/download/latest.json" | "/download/v0.9.0/latest.json" => Reply::ok(&b3),
            "/download/v0.9.0/latest.json.sig" => Reply::ok(&bad),
            _ => Reply { status: 404, ..Reply::ok(b"") },
        });
        let e = fetch_manifest_from("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}"), &[&pk]).await.err().unwrap();
        assert!(e.contains("Подпись обновления не сошлась"), "{e}");
        assert_eq!(log.lock().unwrap().len(), 4, "latest, подпись, затем один повтор: манифест и подпись по тегу");
        // Нет подписи вовсе (404): не ставим.
        let b4 = body.clone();
        let (port, _) = serve(move |r| if r.path.ends_with("latest.json") { Reply::ok(&b4) } else { Reply { status: 404, ..Reply::ok(b"") } });
        assert!(fetch_manifest_from("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}"), &[&pk]).await.is_err());
    }

    #[tokio::test]
    async fn manifest_with_odd_tag_is_not_followed() {
        let (_, pk) = keypair(23);
        let body = br#"{"version":"0.9.0","tag":"v1.0.0/../../evil","platforms":{}}"#.to_vec();
        let (port, log) = serve(move |_| Reply::ok(&body));
        let e = fetch_manifest_from("http://127.0.0.1:1", &format!("http://127.0.0.1:{port}"), &[&pk]).await.err().unwrap();
        assert!(e.contains("странный тег"), "{e}");
        assert_eq!(log.lock().unwrap().len(), 1, "по странному тегу ничего не запрашиваем");
    }

    /// Живая проверка: cargo test --lib live_update_check -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_update_check() {
        println!("update → {:?}", check(Mode::Manual).await);
    }

    /// ЖИВАЯ тихая установка (только отладочная сборка, ничего не мокает):
    /// скачивает установщик с локального сервера, проверяет сумму и запускает
    /// его как тихое обновление (/S /UPDATE без окон, затем программа
    /// стартует в трее). ВНИМАНИЕ: заменяет установленный Clod Helper.
    ///
    /// Подготовка: любой установщик NSIS в папке, раздать её локально
    /// (`npx http-server` или `python`-аналог), посчитать SHA-256 файла.
    ///   CLOD_UPDATE_URL=http://127.0.0.1:8000/Clod.Helper_x64-setup.exe
    ///   CLOD_UPDATE_SHA=<sha256 установщика>
    ///   CLOD_UPDATE_VERSION=99.0.0            (необязательно)
    ///   CLOD_UPDATE_RESTART_EXE=<путь к установленному clod-helper.exe>
    /// Запуск: cargo test --lib live_silent_install -- --ignored --nocapture
    /// Те же переменные делают «обновлением» локальный файл и в запущенной
    /// отладочной программе (npm run tauri dev): тихая установка пойдёт по
    /// обычным условиям (окно скрыто, простой 10 минут, мост молчит).
    #[tokio::test]
    #[ignore]
    async fn live_silent_install() {
        let info = debug_override().expect("нужны CLOD_UPDATE_URL и CLOD_UPDATE_SHA");
        println!("ставлю {} из {}", info.version, info.url);
        install(&info, &|p| println!("{} {}/{}", p.stage, p.received, p.total), "auto", &|| true)
            .await
            .expect("установка не запустилась");
        println!("установщик запущен; смотрите update_attempt в settings.json и процесс clod-helper.exe --tray");
        // Дать скрипту стартовать до выхода тестового процесса.
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    /// Живая проверка подписи релиза: cargo test --lib live_update -- --ignored
    #[tokio::test]
    #[ignore]
    async fn live_update_signature() {
        fetch_manifest().await.expect("подпись последнего релиза не сошлась");
    }
    /// Живой прогон скрипта замены в sh на временных папках: в резерве
    /// остаётся только предыдущая версия.
    #[cfg(unix)]
    #[test]
    fn swap_keeps_only_previous_backup() {
        let root = std::env::temp_dir().join(format!("clod-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let backup = root.join("backup dir");
        let app = root.join("Apps/Clod Helper.app");
        let staged = root.join("Apps/Clod Helper.app.new");
        for (dir, mark) in [(&app, "old"), (&staged, "new")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("v"), mark).unwrap();
        }
        let stale = backup.join("Clod Helper (0.0.1).app");
        std::fs::create_dir_all(&stale).unwrap();
        // pid уже завершившегося процесса: ждать нечего.
        let mut done = std::process::Command::new("true").spawn().unwrap();
        let pid = done.id();
        done.wait().unwrap();
        let q = |p: &Path| crate::macos::sh_quote(&p.to_string_lossy());
        let script = SwapScript {
            pid,
            log: &q(&root.join("swap.log")),
            backup_dir: &q(&backup),
            backup_app: &q(&backup.join("Clod Helper (9.9.9).app")),
            app: &q(&app),
            staged: &q(&staged),
            silent: false,
        }
        .render()
        .replace("\nopen ", "\n: open ");
        let ok = std::process::Command::new("/bin/sh").arg("-c").arg(&script).status().unwrap();
        assert!(ok.success());
        assert_eq!(std::fs::read_to_string(app.join("v")).unwrap(), "new");
        let prev = backup.join("Clod Helper (9.9.9).app");
        assert_eq!(std::fs::read_to_string(prev.join("v")).unwrap(), "old");
        assert!(!stale.exists(), "старые копии должны удаляться");
        assert!(!staged.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
