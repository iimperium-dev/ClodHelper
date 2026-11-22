//! Статистика и отчёты о сбоях. Без идентификаторов, имени компьютера и IP.
//!
//! Три канала к своему серверу:
//! - `/v1/ping` - раз в сутки: версия, система, включено ли автообновление и
//!   флаги «первый за день/неделю/месяц» (сервер считает активных людей);
//! - `/v1/crash` - сбой (Rust, интерфейс, мост, обновление), только если
//!   включено в настройках;
//! - `/v1/report` - отчёт, который человек отправил сам (всегда).
//!
//! Пока человек не увидел переключатели или строку про отчёты
//! (`privacy_notice_seen`), ничего из автоматического не уходит.
//!
//! Любой текст перед отправкой чистится (telemetry_scrub.rs). Сначала запрос
//! идёт напрямую; через мост - только если соединиться не удалось. Без повторов.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::{self, TelemetryState};
use crate::telemetry_scrub::{clip_bytes, rust_loc, scrub, scrub_clip, sig, sig_rust, sig_ui, ui_function};

#[cfg(not(debug_assertions))]
const BASE: &str = "https://stats.clodhelper.com";
const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Ручной отчёт тяжелее (до 48 КБ): ждём дольше.
const REPORT_TIMEOUT: Duration = Duration::from_secs(20);
const REPORT_FAIL: &str = "Не получилось отправить. Проверьте интернет и попробуйте ещё раз";
const REPORT_REFUSED: &str = "Сервер сейчас не принимает отчёты. Попробуйте позже";
/// 429: лимит на этот адрес на сегодня. Интерфейс узнаёт его по этому тексту.
const REPORT_LIMIT: &str = "Лимит отчётов на сегодня исчерпан. Попробуйте завтра";
const LOC_MAX_CHARS: usize = 128;
/// Пинг дня уходит, когда монитор отметил трафик, а если трафика не было -
/// через столько работы программы.
const MAX_WAIT_FOR_USE: Duration = Duration::from_secs(30 * 60);
/// Как часто проверять, не пора ли слать пинг.
const CHECK_EVERY: Duration = Duration::from_secs(3 * 3600);
const MAX_UI_REPORTS: usize = 5;
// Лимиты сервера (admin-API.md): сверх них ответ 400 или 413. Всё в байтах UTF-8.
const MSG_MAX: usize = 1024;
const STACK_MAX: usize = 16 * 1024;
const CRASH_BODY_MAX: usize = 24 * 1024;
const REPORT_TEXT_MAX: usize = 4 * 1024;
const REPORT_DIAG_MAX: usize = 32 * 1024;
const REPORT_CONTACT_MAX_CHARS: usize = 200;
const REPORT_BODY_MAX: usize = 48 * 1024;
const PING_BODY_MAX: usize = 2 * 1024;
const PENDING_FILE: &str = "crash-pending.json";

/// Адрес сервера. В отладочной сборке - только из `CLOD_STATS_URL`
/// (для живой проверки), иначе отправки нет.
fn base() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("CLOD_STATS_URL").ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
    }
    #[cfg(not(debug_assertions))]
    {
        Some(BASE.to_string())
    }
}

fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else {
        "macos"
    }
}

// ---------------------------------------------------------------- сеть

fn client(proxy: Option<&str>, timeout: Duration) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("ClodHelper/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout);
    match proxy {
        Some(p) => b = b.proxy(reqwest::Proxy::all(p).map_err(|e| e.to_string())?),
        // Прямая ветка действительно прямая: без HTTPS_PROXY из окружения
        // (он ведёт в мост, и «напрямую» перестало бы быть прямым).
        None => b = b.no_proxy(),
    }
    b.build().map_err(|e| e.to_string())
}

fn bridge_proxy() -> String {
    format!("http://127.0.0.1:{}", crate::claude_desktop::PROXY_PORT)
}

/// Через мост идём только когда напрямую не удалось соединиться: запрос
/// не дошёл до сервера. Тайм-аут, обрыв после отправки и любой HTTP-статус
/// повторять нельзя: сервер мог уже принять запрос.
fn should_try_bridge(e: &reqwest::Error) -> bool {
    e.is_connect()
}

/// Выполнить запрос напрямую; не удалось соединиться - через мост.
pub(crate) async fn with_fallback<F, Fut, T>(f: F) -> Result<T, String>
where
    F: Fn(reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
{
    with_fallback_cfg(TIMEOUT, &bridge_proxy(), should_try_bridge, f).await
}

/// То же с тайм-аутом, адресом моста и правилом «когда идти через мост».
async fn with_fallback_cfg<F, Fut, T>(
    timeout: Duration,
    bridge: &str,
    retry: fn(&reqwest::Error) -> bool,
    f: F,
) -> Result<T, String>
where
    F: Fn(reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
{
    match f(client(None, timeout)?).await {
        Ok(v) => Ok(v),
        Err(direct) if retry(&direct) => {
            f(client(Some(bridge), timeout)?).await.map_err(|via| format!("напрямую: {direct}; через мост: {via}"))
        }
        Err(direct) => Err(direct.to_string()),
    }
}

// ---------------------------------------------------------------- отказы сервера

/// Что сервер ответил отказом на этот вид запроса: 429 - лимит на сегодня
/// (до конца суток UTC), 503 - предохранитель (до конца запуска).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Day(String),
    Run,
}

static BLOCKS: Mutex<Vec<(String, Block)>> = Mutex::new(Vec::new());

fn is_blocked(path: &str, day: &str) -> bool {
    let Ok(g) = BLOCKS.lock() else { return false };
    g.iter().any(|(p, b)| p == path && matches!(b, Block::Run) || p == path && matches!(b, Block::Day(d) if d == day))
}

fn note_status(path: &str, status: u16, day: &str) {
    let block = match status {
        429 => Block::Day(day.to_string()),
        503 => Block::Run,
        _ => return,
    };
    if let Ok(mut g) = BLOCKS.lock() {
        g.retain(|(p, _)| p != path);
        g.push((path.to_string(), block));
    }
}

/// POST JSON. Успех - только ответ 2xx. 429 и 503 запоминаются: этот вид
/// запроса больше не шлётся (сутки или до конца запуска).
async fn post(path: &str, body: &Value) -> Result<(), String> {
    let base = base().ok_or("отправка выключена")?;
    post_to(&base, path, body).await
}

async fn post_to(base: &str, path: &str, body: &Value) -> Result<(), String> {
    let day = Stamp::now().day;
    if is_blocked(path, &day) {
        return Err(format!("{path}: сервер просил не слать"));
    }
    let url = format!("{base}{path}");
    let status = with_fallback(|c| {
        let (url, body) = (url.clone(), body.clone());
        async move { c.post(url).json(&body).send().await.map(|r| r.status().as_u16()) }
    })
    .await?;
    if (200..300).contains(&status) {
        return Ok(());
    }
    note_status(path, status, &day);
    Err(format!("ответ {status}"))
}

/// GET к серверу статистики: Ok(None) - 404 или сервер не задан.
pub async fn get(path_and_query: &str) -> Result<Option<String>, String> {
    let Some(base) = base() else { return Ok(None) };
    // До показа переключателей сервер не спрашиваем.
    let seen = tokio::task::spawn_blocking(|| settings::load_checked().is_ok_and(|s| s.privacy_notice_seen)).await;
    if !matches!(seen, Ok(true)) {
        return Err("уведомление о данных ещё не показано".into());
    }
    let url = format!("{base}{path_and_query}");
    with_fallback(|c| {
        let url = url.clone();
        async move {
            let r = c.get(url).send().await?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            r.error_for_status()?.text().await.map(Some)
        }
    })
    .await
}

/// Файл с сервера статистики (фото и видео баннеров): только при включённой
/// статистике, не больше max байт. Напрямую, нет связи - через мост.
pub async fn get_bytes(path: &str, max: usize, timeout: Duration) -> Result<Vec<u8>, String> {
    let base = base().ok_or("сервер не задан")?;
    let on = tokio::task::spawn_blocking(load_if_stats_on).await.ok().flatten().is_some();
    if !on {
        return Err("статистика выключена".into());
    }
    let url = format!("{base}{path}");
    let body = with_fallback_cfg(timeout, &bridge_proxy(), should_try_bridge, |c| {
        let url = url.clone();
        async move {
            let mut r = c.get(url).send().await?.error_for_status()?;
            let mut out: Vec<u8> = Vec::new();
            while let Some(chunk) = r.chunk().await? {
                out.extend_from_slice(&chunk);
                if out.len() > max {
                    // Сервер отдал больше обещанного: дальше не читаем.
                    return Ok(None);
                }
            }
            Ok(Some(out))
        }
    })
    .await?;
    body.ok_or_else(|| "файл больше допустимого".to_string())
}

// ---------------------------------------------------------------- даты

/// Дни от 1970-01-01 до гражданской даты (H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// ISO-неделя даты: (год недели, номер). Неделя начинается в понедельник,
/// первая неделя года - та, где четверг этого года.
pub fn iso_week(y: i64, m: i64, d: i64) -> (i64, i64) {
    let days = days_from_civil(y, m, d);
    let weekday = (days + 3).rem_euclid(7); // понедельник = 0
    let thursday = days - weekday + 3;
    let (ty, _, _) = civil_from_days(thursday);
    (ty, (thursday - days_from_civil(ty, 1, 1)) / 7 + 1)
}

/// Отметки времени для флагов пинга.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub day: String,
    /// ISO-неделя (для iw).
    pub week: String,
    /// Дни от 1970-01-01.
    pub days: i64,
}

/// "YYYY-MM-DD" -> дни от 1970-01-01.
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.split("-");
    let (y, m, d) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Прошло ли не меньше `gap` суток с отмеченного дня (нет отметки - да).
fn gap_passed(last: Option<&str>, now: &Stamp, gap: i64) -> bool {
    last.and_then(parse_day).is_none_or(|d| now.days - d >= gap)
}

impl Stamp {
    pub fn from_days(days: i64) -> Stamp {
        let (y, m, d) = civil_from_days(days);
        let (wy, wn) = iso_week(y, m, d);
        Stamp { day: format!("{y:04}-{m:02}-{d:02}"), week: format!("{wy:04}-W{wn:02}"), days }
    }

    pub fn now() -> Stamp {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Stamp::from_days((secs / 86_400) as i64)
    }
}

// ---------------------------------------------------------------- пинг

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ping {
    pub v: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub au: bool,
    pub d: bool,
    pub w: bool,
    pub m: bool,
    pub first: bool,
    pub iw: Option<String>,
    pub how: Option<String>,
    pub used: bool,
}

/// Архитектура в виде, который принимает сервер. Незнакомая - пинг не шлём.
fn arch_for(a: &str) -> Option<&'static str> {
    match a {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        "x86" => Some("i686"),
        _ => None,
    }
}

fn ping_arch() -> Option<&'static str> {
    arch_for(std::env::consts::ARCH)
}

/// Версия вида 1.2.3 без хвоста (-beta, +сборка): с хвостом пинг не шлём.
fn ping_version_ok(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Собрать пинг по сохранённому состоянию. Чистая функция.
pub fn plan_ping(st: &TelemetryState, now: &Stamp, version: &str, au: bool) -> Ping {
    let how = if st.legacy && !st.first_sent {
        None
    } else if st.how_version.as_deref() != Some(version) {
        Some(st.pending_how.clone().filter(|h| h == "auto" || h == "manual").unwrap_or_else(|| "installer".to_string()))
    } else {
        None
    };
    let iw = if st.legacy { None } else { Some(st.install_week.clone().unwrap_or_else(|| now.week.clone())) };
    let first = !st.first_sent && !st.legacy && iw.is_some();
    let w = gap_passed(st.last_w_day.as_deref(), now, 7);
    // Неделя установки нужна серверу только при first или w: в каждом пинге
    // она делала бы строку дня почти уникальной.
    let iw = iw.filter(|_| first || w);
    Ping {
        v: version.to_string(),
        os: os_name(),
        arch: ping_arch().unwrap_or(std::env::consts::ARCH),
        au,
        d: st.last_day.as_deref() != Some(&now.day),
        w,
        m: gap_passed(st.last_m_day.as_deref(), now, 30),
        // Сервер: iw = null требует first = false (first всегда несёт iw).
        first,
        iw,
        how,
        used: st.used_day.as_deref() == Some(&now.day),
    }
}

/// Записать, что пинг дошёл (только после ответа 2xx).
#[cfg(test)]
pub fn commit_ping(st: &mut TelemetryState, p: &Ping, now: &Stamp) {
    let planned = st.pending_how.clone();
    commit_ping_planned(st, p, now, &planned);
}

/// То же, но pending_how очищается, только если с момента расчёта пинга он
/// не поменялся (установщик мог записать новое значение, пока шёл запрос).
pub fn commit_ping_planned(st: &mut TelemetryState, p: &Ping, now: &Stamp, planned_how: &Option<String>) {
    st.first_sent = true;
    st.last_day = Some(now.day.clone());
    if p.w {
        st.last_w_day = Some(now.day.clone());
    }
    if p.m {
        st.last_m_day = Some(now.day.clone());
    }
    if st.install_week.is_none() && !st.legacy {
        st.install_week = p.iw.clone();
    }
    if p.how.is_some() || st.legacy {
        st.how_version = Some(p.v.clone());
        if st.pending_how == *planned_how {
            st.pending_how = None;
        }
    }
}

/// Будит задачу пинга, когда монитор впервые за день отметил трафик.
static USED_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Мониторинг видит, что проверка прокси прошла: сегодня прокси работал.
/// В настройки пишем только при смене дня и сразу будим задачу пинга.
pub fn note_used() {
    static DAY: Mutex<String> = Mutex::new(String::new());
    let day = Stamp::now().day;
    let Ok(mut g) = DAY.lock() else { return };
    if *g == day {
        return;
    }
    if settings::update_telemetry(|t| t.used_day = Some(day.clone())).is_ok() {
        *g = day;
        USED_NOTIFY.notify_one();
    }
}

/// Первая отметка установки. Уже есть версия из «Что нового» - обновились со
/// старой версии (до статистики): legacy, не новая установка. Нет - новая
/// установка: неделя пишется сразу, не дожидаясь сервера. Возвращает, менялось ли.
pub fn init_install_state(t: &mut TelemetryState, had_version: bool, week: &str) -> bool {
    if t.first_sent || t.install_week.is_some() || t.legacy {
        return false;
    }
    if had_version {
        t.legacy = true;
    } else {
        t.install_week = Some(week.to_string());
    }
    true
}

/// Вызывать из setup: после проверки единственного экземпляра, не в режиме
/// заставки, до интерфейса.
pub fn mark_legacy() {
    let week = Stamp::now().week;
    let Ok(s) = settings::load_checked() else { return };
    if !init_install_state(&mut s.telemetry.clone(), s.last_version_seen.is_some(), &week) {
        return;
    }
    let _ = settings::with_settings_strict(|s| {
        let had = s.last_version_seen.is_some();
        init_install_state(&mut s.telemetry, had, &week);
        Ok(())
    });
}

/// День, за который пинг уже принят сервером (2xx). Если запись в настройки
/// не удалась, в тот же день повторно не шлём.
static SENT_DAY: Mutex<String> = Mutex::new(String::new());

fn sent_today(day: &str) -> bool {
    SENT_DAY.lock().map(|g| *g == day).unwrap_or(false)
}

fn mark_sent_today(day: &str) {
    if let Ok(mut g) = SENT_DAY.lock() {
        *g = day.to_string();
    }
}

/// Статистика разрешена только по прочитанным настройкам и только после
/// показа переключателей: если файл не читается, считаем выключенной.
fn load_if_stats_on() -> Option<settings::Settings> {
    settings::load_checked().ok().filter(|s| settings::usage_stats_on(s) && s.privacy_notice_seen)
}

/// Отчёты о сбоях разрешены только по прочитанным настройкам и после показа
/// переключателей.
fn reports_on() -> bool {
    settings::load_checked().is_ok_and(|s| settings::crash_reports_on(&s) && s.privacy_notice_seen)
}

/// Отправить пинг, если сегодня ещё не отправляли.
async fn ping_if_due() {
    let now = Stamp::now();
    if sent_today(&now.day) || base().is_none() || ping_arch().is_none() || !ping_version_ok(env!("CARGO_PKG_VERSION")) {
        return;
    }
    // Чтение файла - не в потоке async-задачи.
    let Ok(Some(s)) = tokio::task::spawn_blocking(load_if_stats_on).await else { return };
    if s.telemetry.last_day.as_deref() == Some(&now.day) {
        return;
    }
    let planned_how = s.telemetry.pending_how.clone();
    let p = plan_ping(&s.telemetry, &now, env!("CARGO_PKG_VERSION"), s.auto_update == Some(true));
    let Ok(body) = serde_json::to_value(&p) else { return };
    if body.to_string().len() > PING_BODY_MAX {
        return;
    }
    match post("/v1/ping", &body).await {
        Ok(()) => {
            mark_sent_today(&now.day);
            let day = now.day.clone();
            let res = tokio::task::spawn_blocking(move || {
                settings::update_telemetry(|t| commit_ping_planned(t, &p, &now, &planned_how))
            })
            .await;
            match res {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("статистика: пинг за {day} принят, но запись не удалась: {e}"),
                Err(e) => tracing::warn!("статистика: запись отметки пинга оборвалась: {e}"),
            }
        }
        Err(e) => tracing::info!("статистика: {e}"),
    }
}

/// Фон: сбой прошлого запуска и пинг. Вызывается из setup.
pub fn spawn() {
    // Сбой прошлого запуска - сразу, без ожидания.
    tauri::async_runtime::spawn(async {
        send_pending_crash().await;
    });
    tauri::async_runtime::spawn(async {
        let started = tokio::time::Instant::now();
        loop {
            // Пинг дня: когда монитор отметил трафик (used = true) или через
            // 30 минут работы, если трафика не было. Что раньше.
            let left = MAX_WAIT_FOR_USE.saturating_sub(started.elapsed());
            if !left.is_zero() {
                tokio::select! {
                    _ = USED_NOTIFY.notified() => {}
                    _ = tokio::time::sleep(left) => {}
                }
            }
            ping_if_due().await;
            // Дальше: проснуться от трафика нового дня или проверить через 3 часа.
            tokio::select! {
                _ = USED_NOTIFY.notified() => {}
                _ = tokio::time::sleep(CHECK_EVERY) => {}
            }
        }
    });
}

/// Как запущено обновление ("auto" | "manual"): пишет установщик до запуска.
pub fn set_pending_how(how: Option<&str>) {
    let how = how.map(str::to_string);
    let _ = settings::update_telemetry(|t| t.pending_how = how);
}

// ---------------------------------------------------------------- сбои

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CrashReport {
    v: String,
    os: String,
    kind: String,
    msg: String,
    stack: String,
    sig: String,
    /// Где упало: файл (rust), функция (ui), операция (bridge, update).
    /// Необязательное поле сервера, до 128 знаков.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    loc: String,
}

/// Место для сервера: очищено, только [A-Za-z0-9_./:<>-] (остальное - «_»),
/// не длиннее 128 знаков. Пустое не шлётся.
fn clean_loc(loc: &str) -> String {
    scrub(loc.trim())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "_./:<>-".contains(c) { c } else { '_' })
        .take(LOC_MAX_CHARS)
        .collect()
}

fn build_crash(kind: &str, msg: &str, stack: &str, sig: String, loc: &str) -> CrashReport {
    let mut c = CrashReport {
        v: env!("CARGO_PKG_VERSION").to_string(),
        os: os_name().to_string(),
        kind: kind.to_string(),
        // Чистим, потом режем: замены могут удлинить текст.
        msg: scrub_clip(msg, MSG_MAX),
        stack: scrub_clip(stack, STACK_MAX),
        sig,
        loc: clean_loc(loc),
    };
    fit_crash(&mut c);
    c
}

/// Уложить сбой в лимиты сервера: msg до 1 КБ, stack до 16 КБ, тело до 24 КБ
/// (в JSON переводы строк и кавычки занимают по два байта).
fn fit_crash(c: &mut CrashReport) {
    c.msg = clip_bytes(&c.msg, MSG_MAX);
    c.stack = clip_bytes(&c.stack, STACK_MAX);
    while serde_json::to_vec(c).map_or(0, |b| b.len()) > CRASH_BODY_MAX && !c.stack.is_empty() {
        let keep = c.stack.len() * 9 / 10;
        c.stack = clip_bytes(&c.stack, keep);
    }
}

async fn send_crash(c: &CrashReport) -> Result<(), String> {
    post("/v1/crash", &serde_json::to_value(c).map_err(|e| e.to_string())?).await
}

static SENT_SIGS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static UI_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Один отчёт на каждый sig за запуск.
fn first_time(sig: &str) -> bool {
    let Ok(mut g) = SENT_SIGS.lock() else { return false };
    g.get_or_insert_with(HashSet::new).insert(sig.to_string())
}

/// Отправить сбой в фоне (мост, обновление, интерфейс).
fn spawn_crash(kind: &str, msg: &str, stack: &str, place: &str, op: &str) {
    if base().is_none() {
        return;
    }
    let (kind, msg, stack, place, op) = (kind.to_string(), msg.to_string(), stack.to_string(), place.to_string(), op.to_string());
    tauri::async_runtime::spawn(async move {
        // Чтение настроек и очистка текста - не в потоке async-рантайма.
        let built = tokio::task::spawn_blocking(move || {
            if !reports_on() {
                return None;
            }
            let c = build_crash(&kind, &msg, &stack, sig(&kind, &place), &op);
            first_time(&c.sig).then_some(c)
        })
        .await;
        let Ok(Some(c)) = built else { return };
        if let Err(e) = send_crash(&c).await {
            tracing::info!("отчёт о сбое: {e}");
        }
    });
}

/// Сбой интерфейса: не больше 5 за запуск и один раз на каждый sig.
pub fn report_ui(msg: &str, stack: &str) {
    if UI_COUNT.load(Ordering::Relaxed) >= MAX_UI_REPORTS {
        return;
    }
    let sg = sig_ui(msg, stack);
    if !reports_on() || base().is_none() || !first_time(&sg) {
        return;
    }
    UI_COUNT.fetch_add(1, Ordering::Relaxed);
    let c = build_crash("ui", msg, stack, sg, &ui_function(stack));
    tauri::async_runtime::spawn(async move {
        if let Err(e) = send_crash(&c).await {
            tracing::info!("отчёт о сбое: {e}");
        }
    });
}

/// Мост умер не по ручной остановке. `place` - где замечено.
pub fn report_bridge(msg: &str, place: &str) {
    // Операция для loc: «watchdog:revived» -> «watchdog».
    spawn_crash("bridge", msg, "", place, place.split(':').next().unwrap_or(""));
}

/// Ошибка установки обновления.
pub fn report_update(msg: &str) {
    spawn_crash("update", msg, "", "updater::install", "install");
}

fn pending_path() -> Option<std::path::PathBuf> {
    settings::data_dir().map(|d| d.join(PENDING_FILE))
}

/// Хук паники. В релизе `panic = "abort"`: процесс умрёт сразу после хука,
/// поэтому запись идёт синхронно в файл, а отправка - при следующем старте.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Паника внутри хука при панике - сразу abort, поймать её нельзя.
        // Поэтому write_pending не делает ничего рискованного: чтение
        // настроек и запись файла, без unwrap.
        write_pending(info);
        prev(info);
    }));
}

fn write_pending(info: &std::panic::PanicHookInfo<'_>) {
    if base().is_none() || !reports_on() {
        return;
    }
    let msg = match info.payload().downcast_ref::<&str>() {
        Some(s) => (*s).to_string(),
        None => info.payload().downcast_ref::<String>().cloned().unwrap_or_else(|| "panic".into()),
    };
    let file = info.location().map(|l| l.file().to_string()).unwrap_or_default();
    let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
    let stack = std::backtrace::Backtrace::force_capture().to_string();
    let c = build_crash("rust", &format!("{msg} ({place})"), &stack, sig_rust(&file, &msg), &rust_loc(&file));
    if let (Some(path), Ok(text)) = (pending_path(), serde_json::to_string(&c)) {
        // Атомарно: tmp + rename, чтобы при аварийном выходе не остался обрезанный файл.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Отправить сбой прошлого запуска. Файл удаляется после одной попытки.
async fn send_pending_crash() {
    // Файлы и настройки - в блокирующей задаче. Файл удаляется до попытки.
    let read = tokio::task::spawn_blocking(|| {
        let path = pending_path()?;
        let text = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        if !reports_on() || base().is_none() {
            return None;
        }
        serde_json::from_str::<CrashReport>(&text).ok()
    })
    .await;
    let Ok(Some(mut c)) = read else { return };
    fit_crash(&mut c);
    if let Err(e) = send_crash(&c).await {
        tracing::info!("отчёт о сбое: {e}");
    }
}

// ---------------------------------------------------------------- события рассылок

static SENT_EVENTS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Тело события или None: неизвестное событие, пустой id и уведомления об
/// обновлении (у них нет записи в рассылках) не шлём.
fn notice_event_body(id: &str, ev: &str) -> Option<Value> {
    if !matches!(ev, "shown" | "clicked" | "dismissed") || id.is_empty() || id.len() > 128 || id.starts_with("update-") {
        return None;
    }
    Some(json!({ "id": id, "ev": ev, "v": env!("CARGO_PKG_VERSION"), "os": os_name() }))
}

/// Одно событие на пару (id, ev) за запуск.
fn first_event(id: &str, ev: &str) -> bool {
    let Ok(mut g) = SENT_EVENTS.lock() else { return false };
    g.get_or_insert_with(HashSet::new).insert(format!("{ev}:{id}"))
}

/// Событие уведомления для статистики рассылок (shown, clicked, dismissed):
/// только при включённой статистике, один раз, без повторов.
pub fn report_notice_event(id: &str, ev: &str) {
    let Some(body) = notice_event_body(id, ev) else { return };
    if base().is_none() || !first_event(id, ev) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let Ok(Some(_)) = tokio::task::spawn_blocking(load_if_stats_on).await else { return };
        if let Err(e) = post("/v1/notice-event", &body).await {
            tracing::info!("событие уведомления: {e}");
        }
    });
}

// ---------------------------------------------------------------- отчёт человека

/// Тело отчёта в лимитах сервера: text до 4 КБ, diag до 32 КБ, contact до 200
/// символов, всё до 48 КБ. Тексты чистятся до обрезки.
fn build_report_body(text: &str, diag: &str, contact: Option<&str>, nonce: &str) -> Value {
    let mut diag = scrub_clip(diag, REPORT_DIAG_MAX);
    let text = scrub_clip(text, REPORT_TEXT_MAX);
    let contact = contact
        .map(|c| c.trim().chars().take(REPORT_CONTACT_MAX_CHARS).collect::<String>())
        .filter(|c| !c.is_empty());
    loop {
        let mut body = json!({
            "v": env!("CARGO_PKG_VERSION"),
            "os": os_name(),
            "nonce": nonce,
            "text": text,
            "diag": diag,
        });
        // Контакт человек вписал сам и хочет, чтобы ему ответили: не чистим.
        if let Some(c) = &contact {
            body["contact"] = json!(c);
        }
        if body.to_string().len() <= REPORT_BODY_MAX || diag.is_empty() {
            return body;
        }
        let keep = diag.len() * 9 / 10;
        diag = clip_bytes(&diag, keep);
    }
}

/// Одноразовый номер отчёта: 32 hex. Повтор того же отчёта идёт с тем же
/// номером, сервер выбрасывает дубль (помнит двое суток).
fn new_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    for _ in 0..2 {
        let mut st = std::collections::hash_map::RandomState::new().build_hasher();
        st.write_u128(nanos);
        st.write_usize(COUNTER.fetch_add(1, Ordering::Relaxed));
        sha2::Digest::update(&mut h, st.finish().to_le_bytes());
    }
    sha2::Digest::update(&mut h, std::process::id().to_le_bytes());
    sha2::Digest::finalize(h).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Номер отчёта от интерфейса: 16-64 знака [A-Za-z0-9_-]. Иначе свой.
/// Интерфейс создаёт номер один раз на окно, и «Повторить» шлёт тот же.
fn pick_nonce(from_ui: Option<String>) -> String {
    match from_ui {
        Some(n) if (16..=64).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') => n,
        _ => new_nonce(),
    }
}

/// Подготовка тела: на сервер уходит урезанная диагностика (без сайтов,
/// меток, корпоративных прокси и чужих доменов), человек же видит в
/// программе полный текст. Чтение файлов - в блокирующей задаче.
async fn prepare_report(text: String, contact: Option<String>, nonce: String) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        let diag = crate::diagnostics::report_for_server();
        build_report_body(&text, &diag, contact.as_deref(), &nonce)
    })
    .await
    .map_err(|_| REPORT_FAIL.to_string())
}

/// Тело ручного отчёта в том виде, в каком оно уйдёт на сервер (для
/// «Посмотреть, что уйдёт»). Номер отчёта показан тот же формат, что и в отправке.
pub async fn report_preview(text: String, contact: Option<String>, nonce: Option<String>) -> String {
    match prepare_report(text, contact, pick_nonce(nonce)).await {
        Ok(body) => serde_json::to_string_pretty(&body).unwrap_or_default(),
        Err(e) => e,
    }
}

/// Отправка отчёта: напрямую (20 с); не удалось соединиться или вышло время -
/// один раз через мост, то же тело с тем же nonce. 429 и 503 - понятная ошибка
/// без повтора.
async fn post_report_to(base: &str, body: &Value, timeout: Duration, bridge: &str) -> Result<(), String> {
    let url = format!("{base}/v1/report");
    let status = with_fallback_cfg(timeout, bridge, |e| e.is_connect() || e.is_timeout(), |c| {
        let (url, body) = (url.clone(), body.clone());
        async move { c.post(url).json(&body).send().await.map(|r| r.status().as_u16()) }
    })
    .await
    .map_err(|e| {
        tracing::info!("отчёт: {e}");
        REPORT_FAIL.to_string()
    })?;
    match status {
        200..=299 => Ok(()),
        429 => Err(REPORT_LIMIT.to_string()),
        503 => Err(REPORT_REFUSED.to_string()),
        s => {
            tracing::info!("отчёт: ответ {s}");
            Err(REPORT_FAIL.to_string())
        }
    }
}

/// Отчёт, отправленный человеком: текст, состояние программы и контакт.
/// Явное действие: согласие (privacy_notice_seen) на него не влияет.
pub async fn send_report(text: String, contact: Option<String>, nonce: Option<String>) -> Result<(), String> {
    let body = prepare_report(text, contact, pick_nonce(nonce)).await?;
    let base = base().ok_or_else(|| REPORT_FAIL.to_string())?;
    post_report_to(&base, &body, REPORT_TIMEOUT, &bridge_proxy()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn st() -> TelemetryState {
        TelemetryState::default()
    }

    #[test]
    fn iso_week_at_year_boundary() {
        let w = |y, m, d| iso_week(y, m, d);
        assert_eq!(w(2026, 12, 31), (2026, 53));
        assert_eq!(w(2027, 1, 1), (2026, 53));
        assert_eq!(w(2027, 1, 3), (2026, 53));
        assert_eq!(w(2027, 1, 4), (2027, 1));
        assert_eq!(w(2024, 12, 30), (2025, 1));
        assert_eq!(w(2026, 11, 20), (2026, 47));
        assert_eq!(w(2021, 1, 3), (2020, 53));
        assert_eq!(w(1970, 1, 1), (1970, 1));
    }

    #[test]
    fn stamp_formats() {
        let s = Stamp::from_days(days_from_civil(2026, 11, 20));
        assert_eq!((s.day.as_str(), s.week.as_str()), ("2026-11-20", "2026-W47"));
        let s = Stamp::from_days(days_from_civil(2027, 1, 1));
        assert_eq!((s.day.as_str(), s.week.as_str()), ("2027-01-01", "2026-W53"));
        assert_eq!(Stamp::now().day.len(), 10);
        assert_eq!(parse_day("2026-11-20"), Some(days_from_civil(2026, 11, 20)));
        assert_eq!(parse_day("2026-W47"), None);
    }

    fn at(y: i64, m: i64, d: i64) -> Stamp {
        Stamp::from_days(days_from_civil(y, m, d))
    }

    #[test]
    fn ping_flags_first_ping_sets_everything() {
        let now = at(2026, 11, 20);
        let p = plan_ping(&st(), &now, "0.9.0", true);
        assert!(p.first && p.d && p.w && p.m && p.au);
        assert_eq!(p.iw.as_deref(), Some("2026-W47"));
        let mut s = st();
        commit_ping(&mut s, &p, &now);
        assert!(s.first_sent);
        assert_eq!((s.last_w_day.as_deref(), s.last_m_day.as_deref()), (Some("2026-11-20"), Some("2026-11-20")));
        // Следующий день: только d.
        let p2 = plan_ping(&s, &at(2026, 11, 21), "0.9.0", false);
        assert!(!p2.first && p2.d && !p2.w && !p2.m && !p2.au);
        assert_eq!(p2.iw, None, "iw только при first или w");
        assert_eq!(p.iw.as_deref(), Some("2026-W47"));
    }

    #[test]
    fn w_flag_after_seven_days_m_after_thirty() {
        let mut s = st();
        let n = at(2026, 11, 1);
        let p = plan_ping(&s, &n, "0.9.0", false);
        commit_ping(&mut s, &p, &n);
        assert!(!plan_ping(&s, &at(2026, 11, 7), "0.9.0", false).w, "шестой день без w");
        assert!(plan_ping(&s, &at(2026, 11, 8), "0.9.0", false).w, "седьмой с w");
        assert!(!plan_ping(&s, &at(2026, 11, 30), "0.9.0", false).m, "29 дней без m");
        assert!(plan_ping(&s, &at(2026, 12, 1), "0.9.0", false).m, "30 дней с m");
        // Пинг без w не сдвигает отсчёт недели.
        let n = at(2026, 11, 5);
        let p = plan_ping(&s, &n, "0.9.0", false);
        commit_ping(&mut s, &p, &n);
        assert_eq!(s.last_w_day.as_deref(), Some("2026-11-01"));
        assert!(plan_ping(&s, &at(2026, 11, 8), "0.9.0", false).w);
        // Стык годов считается сутками, не неделями.
        let mut y = st();
        y.last_w_day = Some("2026-12-31".into());
        assert!(!plan_ping(&y, &at(2027, 1, 6), "0.9.0", false).w);
        assert!(plan_ping(&y, &at(2027, 1, 7), "0.9.0", false).w);
        // Старый формат отбрасывается: как будто пинга не было.
        y.last_w_day = Some("2026-W47".into());
        assert!(plan_ping(&y, &at(2027, 1, 1), "0.9.0", false).w);
        let iw = plan_ping(&st(), &at(2027, 1, 1), "0.9.0", false).iw;
        assert_eq!(iw.as_deref(), Some("2026-W53"));
    }

    #[test]
    fn legacy_update_is_not_a_new_install() {
        let now = Stamp::from_days(days_from_civil(2026, 11, 20));
        let mut s = TelemetryState { legacy: true, ..st() };
        let p = plan_ping(&s, &now, "0.9.0", true);
        assert!(!p.first && p.iw.is_none() && p.how.is_none());
        assert!(p.d && p.w && p.m, "d, w и m считаются как обычно");
        commit_ping(&mut s, &p, &now);
        assert!(s.first_sent);
        assert_eq!(s.how_version.as_deref(), Some("0.9.0"));
        assert_eq!(s.install_week, None, "install_week у legacy пустой");
        let next = Stamp::from_days(days_from_civil(2026, 11, 21));
        let p2 = plan_ping(&s, &next, "0.9.0", true);
        assert!(!p2.first && p2.iw.is_none() && p2.how.is_none());
        // Следующая версия после ручного обновления.
        s.pending_how = Some("manual".into());
        assert_eq!(plan_ping(&s, &next, "0.9.1", true).how.as_deref(), Some("manual"));
    }

    #[test]
    fn ping_not_committed_without_success() {
        let now = Stamp::from_days(days_from_civil(2026, 11, 20));
        let s = st();
        let _ = plan_ping(&s, &now, "0.9.0", false);
        assert_eq!(s, st(), "до ответа сервера состояние не меняется");
    }

    #[test]
    fn ping_used_flag() {
        let now = Stamp::from_days(days_from_civil(2026, 11, 20));
        let mut s = st();
        assert!(!plan_ping(&s, &now, "1.0.0", false).used);
        s.used_day = Some("2026-11-19".into());
        assert!(!plan_ping(&s, &now, "1.0.0", false).used);
        s.used_day = Some("2026-11-20".into());
        assert!(plan_ping(&s, &now, "1.0.0", false).used);
    }

    #[test]
    fn ping_how_installer_manual_null() {
        let now = Stamp::from_days(days_from_civil(2026, 11, 20));
        let mut s = st();
        assert_eq!(plan_ping(&s, &now, "0.9.0", false).how.as_deref(), Some("installer"));
        s.pending_how = Some("manual".into());
        let p = plan_ping(&s, &now, "0.9.0", false);
        assert_eq!(p.how.as_deref(), Some("manual"));
        commit_ping(&mut s, &p, &now);
        assert_eq!(s.pending_how, None);
        assert_eq!(s.how_version.as_deref(), Some("0.9.0"));
        // Та же версия - how уже отправлен.
        let next = Stamp::from_days(days_from_civil(2026, 11, 21));
        assert_eq!(plan_ping(&s, &next, "0.9.0", false).how, None);
        // Новая версия с автообновлением.
        s.pending_how = Some("auto".into());
        assert_eq!(plan_ping(&s, &next, "0.9.1", false).how.as_deref(), Some("auto"));
        // Мусор в pending_how - как будто его нет.
        s.pending_how = Some("zzz".into());
        assert_eq!(plan_ping(&s, &next, "0.9.1", false).how.as_deref(), Some("installer"));
    }

    #[test]
    fn crash_report_is_scrubbed_and_stable() {
        let a = build_crash(
            "rust",
            r"boom at C:\Users\anna\x.rs for me@mail.com 1.2.3.4",
            "",
            sig_rust(r"C:\Users\anna\x.rs:5", "boom 7"),
            &rust_loc(r"C:\Users\anna\x.rs:5"),
        );
        let b = build_crash("rust", "other", "", sig_rust("C:/Users/boris/x.rs:9", "boom 8"), &rust_loc("C:/Users/boris/x.rs:9"));
        assert_eq!(a.sig, b.sig);
        assert_eq!(a.loc, "C:/Users/<user>/x.rs");
        assert_eq!(build_crash("ui", "m", "", "ui-0123456789abcdef".into(), "").loc, "");
        let long_loc = "f".repeat(300);
        assert_eq!(build_crash("ui", "m", "", "ui-0123456789abcdef".into(), &long_loc).loc.chars().count(), 128);
        let with = serde_json::to_value(build_crash("ui", "m", "", "ui-0123456789abcdef".into(), "Xo")).unwrap();
        assert_eq!(with["loc"], "Xo");
        let without = serde_json::to_value(build_crash("ui", "m", "", "ui-0123456789abcdef".into(), "")).unwrap();
        assert!(without.get("loc").is_none(), "пустой loc не шлём");
        assert!(!a.msg.contains("anna") && !a.msg.contains("mail.com") && !a.msg.contains("1.2.3.4"));
        assert_eq!(a.kind, "rust");
    }

    #[test]
    fn fresh_install_gets_week_at_once_update_gets_legacy() {
        let mut t = st();
        assert!(init_install_state(&mut t, false, "2026-W47"));
        assert_eq!(t.install_week.as_deref(), Some("2026-W47"));
        assert!(!t.legacy, "новая установка не legacy, даже если первый пинг не дошёл");
        // Повторный запуск ничего не меняет.
        assert!(!init_install_state(&mut t, false, "2026-W48"));
        assert_eq!(t.install_week.as_deref(), Some("2026-W47"));
        let mut old = st();
        assert!(init_install_state(&mut old, true, "2026-W47"));
        assert!(old.legacy && old.install_week.is_none());
        let mut sent = TelemetryState { first_sent: true, ..st() };
        assert!(!init_install_state(&mut sent, false, "2026-W47"));
        // Пинг новой установки несёт установленную неделю, а не «текущую».
        let p = plan_ping(&t, &at(2026, 12, 20), "0.9.0", false);
        assert_eq!(p.iw.as_deref(), Some("2026-W47"));
        assert!(p.first);
    }

    #[test]
    fn pending_how_survives_change_during_request() {
        let now = at(2026, 11, 20);
        let mut s = TelemetryState { pending_how: Some("manual".into()), ..st() };
        let planned = s.pending_how.clone();
        let p = plan_ping(&s, &now, "0.9.0", false);
        // Пока шёл запрос, установщик записал новое значение.
        s.pending_how = Some("auto".into());
        commit_ping_planned(&mut s, &p, &now, &planned);
        assert_eq!(s.pending_how.as_deref(), Some("auto"), "новое значение не затираем");
        assert_eq!(s.how_version.as_deref(), Some("0.9.0"));
        // Не менялось - очищается.
        let mut s2 = TelemetryState { pending_how: Some("manual".into()), ..st() };
        let planned = s2.pending_how.clone();
        commit_ping_planned(&mut s2, &p, &now, &planned);
        assert_eq!(s2.pending_how, None);
    }

    #[test]
    fn sent_today_blocks_repeat_in_same_day() {
        assert!(!sent_today("2099-01-01"));
        mark_sent_today("2099-01-01");
        assert!(sent_today("2099-01-01"));
        assert!(!sent_today("2099-01-02"));
    }

    /// Мини-сервер: на каждое соединение отвечает заданным статусом.
    fn serve_status(status: &'static str) -> u16 {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
            }
        });
        port
    }

    #[tokio::test]
    async fn fallback_only_on_connect_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // HTTP-статус 500: сервер ответил, моста не пробуем.
        let port = serve_status("500 Internal Server Error");
        let calls = AtomicUsize::new(0);
        let url = format!("http://127.0.0.1:{port}/x");
        let r: Result<(), String> = with_fallback(|c| {
            calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            async move { c.post(url).send().await?.error_for_status().map(|_| ()) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "после HTTP-статуса повтора нет");
        // Тайм-аут после отправки: тоже без повтора.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let calls = AtomicUsize::new(0);
        let url = format!("http://127.0.0.1:{port}/x");
        let r: Result<(), String> = with_fallback(|c| {
            calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            async move {
                c.post(url).timeout(Duration::from_millis(300)).send().await?.error_for_status().map(|_| ())
            }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "после тайм-аута повтора нет");
        drop(l);
        // Соединиться не удалось (порт закрыт): второй заход через мост.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let calls = AtomicUsize::new(0);
        let url = format!("http://127.0.0.1:{port}/x");
        let r: Result<(), String> = with_fallback(|c| {
            calls.fetch_add(1, Ordering::SeqCst);
            let url = url.clone();
            async move { c.post(url).send().await?.error_for_status().map(|_| ()) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2, "ошибка соединения: напрямую и через мост");
    }

    #[test]
    fn long_cyrillic_fits_server_limits() {
        let long = "Ошибка соединения с 1.2.3.4 при запуске моста. ".repeat(3000);
        let c = build_crash("rust", &long, &long, sig("rust", "x.rs:1"), "x.rs");
        assert!(c.msg.len() <= MSG_MAX && c.stack.len() <= STACK_MAX);
        assert!(serde_json::to_vec(&c).unwrap().len() <= CRASH_BODY_MAX);
        // Стек из одних переводов строк: в JSON каждый занимает два байта.
        let nl = "\n".repeat(20_000);
        let c = build_crash("ui", "m", &nl, sig("ui", "x.js:1:1"), "");
        assert!(serde_json::to_vec(&c).unwrap().len() <= CRASH_BODY_MAX);
        // Отложенный сбой из старой версии тоже режется.
        let mut old = CrashReport { v: "0".into(), os: "windows".into(), kind: "rust".into(), msg: long.clone(), stack: long.clone(), sig: "rs-abcd".into(), loc: String::new() };
        fit_crash(&mut old);
        assert!(old.msg.len() <= MSG_MAX && serde_json::to_vec(&old).unwrap().len() <= CRASH_BODY_MAX);

        let contact = "я".repeat(500);
        let b = build_report_body(&long, &long, Some(&contact), NONCE);
        assert!(b.to_string().len() <= REPORT_BODY_MAX);
        assert!(b["text"].as_str().unwrap().len() <= REPORT_TEXT_MAX);
        assert!(b["diag"].as_str().unwrap().len() <= REPORT_DIAG_MAX);
        assert_eq!(b["contact"].as_str().unwrap().chars().count(), 200);
        // Диагностика из переводов строк тоже укладывается в 48 КБ.
        let nl = "\n".repeat(40_000);
        assert!(build_report_body("t", &nl, None, NONCE).to_string().len() <= REPORT_BODY_MAX);
        assert!(build_report_body("t", "d", Some("  "), NONCE).get("contact").is_none());
    }

    #[test]
    fn ping_matches_server_contract() {
        let now = at(2026, 11, 20);
        // iw = null только у legacy, и тогда first = false: в любых состояниях.
        for legacy in [false, true] {
            for first_sent in [false, true] {
                for week in [None, Some("2026-W40".to_string())] {
                    let s = TelemetryState { legacy, first_sent, install_week: week, ..st() };
                    let p = plan_ping(&s, &now, "0.9.1", true);
                    assert!(p.iw.is_some() || !p.first, "iw=null и first=true недопустимы: {s:?}");
                    assert!(serde_json::to_vec(&p).unwrap().len() <= PING_BODY_MAX);
                }
            }
        }
        assert!(ping_version_ok("0.9.1") && ping_version_ok("12.0.30"));
        assert!(!ping_version_ok("0.9.1-beta") && !ping_version_ok("0.9") && !ping_version_ok("0.9.1+b2") && !ping_version_ok(""));
        assert_eq!(arch_for("x86_64"), Some("x86_64"));
        assert_eq!(arch_for("aarch64"), Some("aarch64"));
        assert_eq!(arch_for("x86"), Some("i686"));
        assert_eq!(arch_for("riscv64"), None);
        assert!(ping_arch().is_some(), "на этой машине архитектура известна");
        // Версия самой программы подходит под правило.
        assert!(ping_version_ok(env!("CARGO_PKG_VERSION")), "в Cargo.toml версия с хвостом: пинг не уйдёт");
    }

    #[test]
    fn notice_events_are_valid_and_once() {
        let b = notice_event_body("whats-new-0-9-1", "shown").unwrap();
        assert_eq!((b["id"].as_str(), b["ev"].as_str()), (Some("whats-new-0-9-1"), Some("shown")));
        assert!(b["v"].is_string() && b["os"].is_string());
        assert!(notice_event_body("x", "opened").is_none());
        assert!(notice_event_body("", "shown").is_none());
        assert!(notice_event_body("update-0.9.1", "clicked").is_none());
        assert!(first_event("evt-test-a", "clicked"));
        assert!(!first_event("evt-test-a", "clicked"));
        assert!(first_event("evt-test-a", "dismissed"));
    }

    #[test]
    fn crash_dedup_per_sig() {
        assert!(first_time("test-sig-aaaa"));
        assert!(!first_time("test-sig-aaaa"));
        assert!(first_time("test-sig-bbbb"));
    }

    /// Сервер, который записывает запросы целиком. `hang` - читает и молчит.
    fn serve_recording(status: &'static str, hang: bool) -> (u16, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let log = log2.clone();
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = s.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        data.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&data).to_string();
                        if let Some(h) = text.find("\r\n\r\n") {
                            let len = text[..h]
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            if data.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    log.lock().unwrap().push(String::from_utf8_lossy(&data).to_string());
                    if hang {
                        std::thread::sleep(Duration::from_secs(4));
                    } else {
                        let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
                    }
                });
            }
        });
        (port, log)
    }

    fn nonce_of(req: &str) -> String {
        let i = req.find("\"nonce\":\"").expect("nonce в теле") + 9;
        req[i..i + 32].to_string()
    }

    #[tokio::test]
    async fn status_429_blocks_this_kind_until_next_day() {
        let (port, log) = serve_recording("429 Too Many Requests", false);
        let base = format!("http://127.0.0.1:{port}");
        let body = json!({"a": 1});
        assert!(post_to(&base, "/v1/t429", &body).await.is_err());
        assert_eq!(log.lock().unwrap().len(), 1);
        // Повторно в тот же день сервер не трогаем.
        assert!(post_to(&base, "/v1/t429", &body).await.is_err());
        assert_eq!(log.lock().unwrap().len(), 1, "после 429 запрос не уходит");
        let today = Stamp::now().day;
        assert!(is_blocked("/v1/t429", &today));
        assert!(!is_blocked("/v1/other-kind", &today), "другой вид запроса не заблокирован");
        // Наступили следующие сутки: блок снят.
        assert!(!is_blocked("/v1/t429", "2099-01-01"));
    }

    #[tokio::test]
    async fn status_503_blocks_this_kind_for_the_run() {
        let (port, log) = serve_recording("503 Service Unavailable", false);
        let base = format!("http://127.0.0.1:{port}");
        assert!(post_to(&base, "/v1/t503", &json!({})).await.is_err());
        assert!(post_to(&base, "/v1/t503", &json!({})).await.is_err());
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(is_blocked("/v1/t503", "2099-01-01"), "503 держится и через сутки");
        // Другие ответы (500) не блокируют.
        let (p2, log2) = serve_recording("500 Internal Server Error", false);
        let b2 = format!("http://127.0.0.1:{p2}");
        assert!(post_to(&b2, "/v1/t500", &json!({})).await.is_err());
        assert!(post_to(&b2, "/v1/t500", &json!({})).await.is_err());
        assert_eq!(log2.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn report_refused_gives_clear_error_without_bridge() {
        for st in ["429 Too Many Requests", "503 Service Unavailable"] {
            let (port, log) = serve_recording(st, false);
            let (bp, blog) = serve_recording("204 No Content", false);
            let base = format!("http://127.0.0.1:{port}");
            let body = build_report_body("t", "d", None, NONCE);
            let r = post_report_to(&base, &body, Duration::from_secs(5), &format!("http://127.0.0.1:{bp}")).await;
            let want = if st.starts_with("429") { REPORT_LIMIT } else { REPORT_REFUSED };
            assert_eq!(r, Err(want.to_string()), "{st}");
            assert_eq!(log.lock().unwrap().len(), 1);
            assert_eq!(blog.lock().unwrap().len(), 0, "через мост на {st} не повторяем");
        }
        // Обычная ошибка сервера: общий текст.
        let (port, _) = serve_recording("500 Internal Server Error", false);
        let body = build_report_body("t", "d", None, NONCE);
        let r = post_report_to(&format!("http://127.0.0.1:{port}"), &body, Duration::from_secs(5), "http://127.0.0.1:1").await;
        assert_eq!(r, Err(REPORT_FAIL.to_string()));
    }

    #[tokio::test]
    async fn report_timeout_retries_once_via_bridge_with_same_nonce() {
        let (port, direct) = serve_recording("204 No Content", true);
        let (bp, bridge) = serve_recording("204 No Content", false);
        let nonce = new_nonce();
        let body = build_report_body("текст", "диагностика", None, &nonce);
        let r = post_report_to(
            &format!("http://127.0.0.1:{port}"),
            &body,
            Duration::from_millis(400),
            &format!("http://127.0.0.1:{bp}"),
        )
        .await;
        assert_eq!(r, Ok(()));
        let (d, b) = (direct.lock().unwrap().clone(), bridge.lock().unwrap().clone());
        assert_eq!((d.len(), b.len()), (1, 1), "ровно два вызова: напрямую и через мост");
        assert_eq!(nonce_of(&d[0]), nonce);
        assert_eq!(nonce_of(&b[0]), nonce, "тот же nonce");
    }

    #[test]
    fn loc_is_restricted_to_safe_chars() {
        assert_eq!(clean_loc("$e"), "_e");
        assert_eq!(clean_loc("global code"), "global_code");
        assert_eq!(clean_loc("src/hosts.rs"), "src/hosts.rs");
        assert_eq!(clean_loc("<anonymous>"), "<anonymous>");
        assert_eq!(clean_loc("Функция"), "_______");
        assert_eq!(clean_loc(&"a".repeat(300)).len(), 128);
        assert_eq!(clean_loc("   "), "");
    }

    #[test]
    fn nonce_from_ui_is_checked() {
        let ok = "0123456789abcdef0123456789abcdef".to_string();
        assert_eq!(pick_nonce(Some(ok.clone())), ok);
        assert_eq!(pick_nonce(Some("short".into())).len(), 32);
        assert_eq!(pick_nonce(Some("x".repeat(65))).len(), 32);
        assert_ne!(pick_nonce(Some("bad nonce with spaces!!".into())), "bad nonce with spaces!!");
        assert_eq!(pick_nonce(None).len(), 32);
    }

    #[test]
    fn nonce_is_32_hex_and_unique() {
        let a = new_nonce();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, new_nonce());
        let body = build_report_body("t", "d", Some("@me"), &a);
        assert_eq!(body["nonce"], a.as_str());
        let pretty = serde_json::to_string_pretty(&body).unwrap();
        assert!(pretty.contains("\n  \"nonce\""), "JSON с отступами");
    }

    #[test]
    fn ping_sends_iw_only_with_first_or_w() {
        let s = TelemetryState { first_sent: true, install_week: Some("2026-W40".into()), last_w_day: Some("2026-11-18".into()), ..st() };
        // Обычный день: ни first, ни w - iw нет.
        let p = plan_ping(&s, &at(2026, 11, 20), "0.9.1", true);
        assert!(!p.first && !p.w && p.iw.is_none());
        assert!(serde_json::to_value(&p).unwrap()["iw"].is_null());
        // Прошла неделя - w, и iw на месте.
        let p = plan_ping(&s, &at(2026, 11, 25), "0.9.1", true);
        assert!(p.w && !p.first);
        assert_eq!(p.iw.as_deref(), Some("2026-W40"));
        // Первый пинг: first и iw.
        let p = plan_ping(&TelemetryState { install_week: Some("2026-W40".into()), ..st() }, &at(2026, 11, 20), "0.9.1", true);
        assert!(p.first);
        assert_eq!(p.iw.as_deref(), Some("2026-W40"));
        // Старая установка: iw пуст и first = false всегда.
        let l = TelemetryState { legacy: true, ..st() };
        for (y, m, d) in [(2026, 11, 20), (2026, 11, 28)] {
            let p = plan_ping(&l, &at(y, m, d), "0.9.1", true);
            assert!(p.iw.is_none() && !p.first);
        }
    }

    #[test]
    fn consent_flag_defaults_to_not_seen() {
        // Без флага ничего автоматического не уходит (load_if_stats_on и reports_on его требуют).
        let off = settings::Settings::default();
        assert!(!off.privacy_notice_seen);
        let on = settings::Settings { privacy_notice_seen: true, ..settings::Settings::default() };
        assert!(settings::usage_stats_on(&on) && settings::crash_reports_on(&on) && on.privacy_notice_seen);
    }
}
