//! Статистика и отчёты о сбоях. Без идентификаторов, имени компьютера и IP.
//!
//! Три канала к своему серверу:
//! - `/v1/ping` - раз в сутки: версия, система, включено ли автообновление и
//!   флаги «первый за день/неделю/месяц» (сервер считает активных людей);
//! - `/v1/crash` - сбой (Rust, интерфейс, мост, обновление), только если
//!   включено в настройках;
//! - `/v1/report` - отчёт, который человек отправил сам (всегда).
//!
//! Любой текст перед отправкой чистится (telemetry_scrub.rs). Сначала запрос
//! идёт напрямую, не вышло - через мост. Без повторов.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::{self, TelemetryState};
use crate::telemetry_scrub::{place_from_stack, scrub, sig, truncate};

#[cfg(not(debug_assertions))]
const BASE: &str = "https://stats.clodhelper.com";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Первый пинг и разбор «отложенного» сбоя - через минуту после запуска.
const FIRST_DELAY: Duration = Duration::from_secs(60);
/// Как часто проверять, не пора ли слать пинг.
const CHECK_EVERY: Duration = Duration::from_secs(3 * 3600);
const MAX_UI_REPORTS: usize = 5;
const STACK_MAX: usize = 8 * 1024;
const MSG_MAX: usize = 2000;
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

fn client(proxy: Option<&str>) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("ClodHelper/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT);
    if let Some(p) = proxy {
        b = b.proxy(reqwest::Proxy::all(p).map_err(|e| e.to_string())?);
    }
    b.build().map_err(|e| e.to_string())
}

fn bridge_proxy() -> String {
    format!("http://127.0.0.1:{}", crate::claude_desktop::PROXY_PORT)
}

/// Выполнить запрос напрямую, не вышло - через мост. Без повторов.
async fn with_fallback<F, Fut, T>(f: F) -> Result<T, String>
where
    F: Fn(reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    match f(client(None)?).await {
        Ok(v) => Ok(v),
        Err(direct) => {
            let bridge = bridge_proxy();
            f(client(Some(&bridge))?).await.map_err(|via| format!("напрямую: {direct}; через мост: {via}"))
        }
    }
}

/// POST JSON. Успех - только ответ 2xx.
async fn post(path: &str, body: &Value) -> Result<(), String> {
    let base = base().ok_or("отправка выключена")?;
    let url = format!("{base}{path}");
    with_fallback(|c| {
        let (url, body) = (url.clone(), body.clone());
        async move {
            let r = c.post(url).json(&body).send().await.map_err(|e| e.to_string())?;
            r.error_for_status().map(|_| ()).map_err(|e| e.to_string())
        }
    })
    .await
}

/// GET к серверу статистики: Ok(None) - 404 или сервер не задан.
pub async fn get(path_and_query: &str) -> Result<Option<String>, String> {
    let Some(base) = base() else { return Ok(None) };
    let url = format!("{base}{path_and_query}");
    with_fallback(|c| {
        let url = url.clone();
        async move {
            let r = c.get(url).send().await.map_err(|e| e.to_string())?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            r.error_for_status().map_err(|e| e.to_string())?.text().await.map(Some).map_err(|e| e.to_string())
        }
    })
    .await
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

/// Собрать пинг по сохранённому состоянию. Чистая функция.
pub fn plan_ping(st: &TelemetryState, now: &Stamp, version: &str, au: bool) -> Ping {
    let how = if st.legacy && !st.first_sent {
        None
    } else if st.how_version.as_deref() != Some(version) {
        Some(st.pending_how.clone().filter(|h| h == "auto" || h == "manual").unwrap_or_else(|| "installer".to_string()))
    } else {
        None
    };
    Ping {
        v: version.to_string(),
        os: os_name(),
        arch: std::env::consts::ARCH,
        au,
        d: st.last_day.as_deref() != Some(&now.day),
        w: gap_passed(st.last_w_day.as_deref(), now, 7),
        m: gap_passed(st.last_m_day.as_deref(), now, 30),
        first: !st.first_sent && !st.legacy,
        iw: if st.legacy { None } else { Some(st.install_week.clone().unwrap_or_else(|| now.week.clone())) },
        how,
        used: st.used_day.as_deref() == Some(&now.day),
    }
}

/// Записать, что пинг дошёл (только после ответа 2xx).
pub fn commit_ping(st: &mut TelemetryState, p: &Ping, now: &Stamp) {
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
        st.pending_how = None;
    }
}

/// Мониторинг видит, что проверка прокси прошла: сегодня прокси работал.
/// В настройки пишем только при смене дня.
pub fn note_used() {
    static DAY: Mutex<String> = Mutex::new(String::new());
    let day = Stamp::now().day;
    let Ok(mut g) = DAY.lock() else { return };
    if *g == day {
        return;
    }
    if settings::update_telemetry(|t| t.used_day = Some(day.clone())).is_ok() {
        *g = day;
    }
}

/// Обновились со старой версии (до статистики) - не считать новой установкой.
/// Вызывать при старте, до интерфейса.
pub fn mark_legacy() {
    let s = settings::load();
    if !s.telemetry.first_sent && s.telemetry.install_week.is_none() && !s.telemetry.legacy && s.last_version_seen.is_some() {
        let _ = settings::update_telemetry(|t| t.legacy = true);
    }
}

/// Отправить пинг, если сегодня ещё не отправляли.
async fn ping_if_due() {
    let s = settings::load();
    if !settings::usage_stats_on(&s) || base().is_none() {
        return;
    }
    let now = Stamp::now();
    if s.telemetry.last_day.as_deref() == Some(&now.day) {
        return;
    }
    let p = plan_ping(&s.telemetry, &now, env!("CARGO_PKG_VERSION"), s.auto_update == Some(true));
    let Ok(body) = serde_json::to_value(&p) else { return };
    match post("/v1/ping", &body).await {
        Ok(()) => {
            let _ = settings::update_telemetry(|t| commit_ping(t, &p, &now));
        }
        Err(e) => tracing::info!("статистика: {e}"),
    }
}

/// Фон: сбой прошлого запуска и пинг. Вызывается из setup.
pub fn spawn() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(FIRST_DELAY).await;
        send_pending_crash().await;
        loop {
            ping_if_due().await;
            tokio::time::sleep(CHECK_EVERY).await;
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
}

fn build_crash(kind: &str, msg: &str, stack: &str, place: &str) -> CrashReport {
    CrashReport {
        v: env!("CARGO_PKG_VERSION").to_string(),
        os: os_name().to_string(),
        kind: kind.to_string(),
        msg: scrub(&truncate(msg, MSG_MAX)),
        stack: scrub(&truncate(stack, STACK_MAX)),
        sig: sig(kind, place),
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
fn spawn_crash(kind: &str, msg: &str, stack: &str, place: &str) {
    if !settings::crash_reports_on(&settings::load()) || base().is_none() {
        return;
    }
    let c = build_crash(kind, msg, stack, place);
    if !first_time(&c.sig) {
        return;
    }
    tauri::async_runtime::spawn(async move {
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
    let place = place_from_stack(msg, stack);
    if !settings::crash_reports_on(&settings::load()) || base().is_none() || !first_time(&sig("ui", &place)) {
        return;
    }
    UI_COUNT.fetch_add(1, Ordering::Relaxed);
    let c = build_crash("ui", msg, stack, &place);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = send_crash(&c).await {
            tracing::info!("отчёт о сбое: {e}");
        }
    });
}

/// Мост умер не по ручной остановке. `place` - где замечено.
pub fn report_bridge(msg: &str, place: &str) {
    spawn_crash("bridge", msg, "", place);
}

/// Ошибка установки обновления.
pub fn report_update(msg: &str) {
    spawn_crash("update", msg, "", "updater::install");
}

fn pending_path() -> Option<std::path::PathBuf> {
    settings::data_dir().map(|d| d.join(PENDING_FILE))
}

/// Хук паники. В релизе `panic = "abort"`: процесс умрёт сразу после хука,
/// поэтому запись идёт синхронно в файл, а отправка - при следующем старте.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_pending(info)));
        prev(info);
    }));
}

fn write_pending(info: &std::panic::PanicHookInfo<'_>) {
    if base().is_none() || !settings::crash_reports_on(&settings::load()) {
        return;
    }
    let msg = match info.payload().downcast_ref::<&str>() {
        Some(s) => (*s).to_string(),
        None => info.payload().downcast_ref::<String>().cloned().unwrap_or_else(|| "panic".into()),
    };
    let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
    let stack = std::backtrace::Backtrace::force_capture().to_string();
    let c = build_crash("rust", &format!("{msg} ({place})"), &stack, &place);
    if let (Some(path), Ok(text)) = (pending_path(), serde_json::to_string(&c)) {
        let _ = std::fs::write(path, text);
    }
}

/// Отправить сбой прошлого запуска. Файл удаляется после одной попытки.
async fn send_pending_crash() {
    let Some(path) = pending_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let _ = std::fs::remove_file(&path);
    if !settings::crash_reports_on(&settings::load()) || base().is_none() {
        return;
    }
    let Ok(c) = serde_json::from_str::<CrashReport>(&text) else { return };
    if let Err(e) = send_crash(&c).await {
        tracing::info!("отчёт о сбое: {e}");
    }
}

// ---------------------------------------------------------------- отчёт человека

/// Отчёт, отправленный человеком: текст, состояние программы и контакт.
pub async fn send_report(text: String, contact: Option<String>) -> Result<(), String> {
    const FAIL: &str = "Не получилось отправить. Проверьте интернет и попробуйте ещё раз";
    let diag = tokio::task::spawn_blocking(crate::diagnostics::report).await.map_err(|_| FAIL.to_string())?;
    let mut body = json!({
        "v": env!("CARGO_PKG_VERSION"),
        "os": os_name(),
        "text": scrub(&truncate(&text, 10_000)),
        "diag": scrub(&truncate(&diag, 64 * 1024)),
    });
    // Контакт человек вписал сам и хочет, чтобы ему ответили: не чистим.
    if let Some(c) = contact.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        body["contact"] = json!(truncate(&c, 200));
    }
    post("/v1/report", &body).await.map_err(|e| {
        tracing::info!("отчёт: {e}");
        FAIL.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(p2.iw.as_deref(), Some("2026-W47"));
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
        let a = build_crash("rust", r"boom at C:\Users\anna\x.rs for me@mail.com 1.2.3.4", "", r"C:\Users\anna\x.rs:5");
        let b = build_crash("rust", "other", "", "C:/Users/boris/x.rs:5");
        assert_eq!(a.sig, b.sig);
        assert!(!a.msg.contains("anna") && !a.msg.contains("mail.com") && !a.msg.contains("1.2.3.4"));
        assert_eq!(a.kind, "rust");
    }

    #[test]
    fn crash_dedup_per_sig() {
        assert!(first_time("test-sig-aaaa"));
        assert!(!first_time("test-sig-aaaa"));
        assert!(first_time("test-sig-bbbb"));
    }
}
