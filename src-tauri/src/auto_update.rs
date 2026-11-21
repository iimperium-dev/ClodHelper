//! Тихая автоустановка обновлений.
//!
//! Включена, если `settings.auto_update == Some(true)`. Найденное обновление
//! скачивается заранее (в кэш, с проверкой суммы), а ставится, когда все
//! условия сошлись (см. `decide`): человек не за компьютером, окно скрыто,
//! мост не занят трафиком, ничего другого не идёт. Пока ждём, условия
//! проверяются раз в 5 минут.
//!
//! Мост перезапускается при первом старте новой версии и обрывает
//! соединения (Claude Code и др.), поэтому ставим только при простое.
//!
//! Каждое условие - отдельная чистая функция (тесты ниже). Установка и её
//! сбои: отметка `update_attempt` пишется перед запуском установщика, на
//! следующем запуске по ней видно, получилось ли (`settle_on_start`).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::settings::{self, Settings, UpdateAttempt, UpdateFailures, UpdateNotice, UpdateSeen};
use crate::updater::{Mode, UpdateInfo};

/// Пока ждём условий - проверка раз в 5 минут.
const TICK: Duration = Duration::from_secs(5 * 60);
/// Человек не трогал мышь и клавиатуру не меньше 10 минут.
pub const MIN_IDLE_SECS: u64 = 10 * 60;
/// Мост «тихий»: за окно в 5 минут через входы Claude прошло меньше 64 КБ.
/// Открытые соединения не считаем: Claude Code держит их, пока запущен.
pub const QUIET_WINDOW_SECS: u64 = 5 * 60;
pub const QUIET_BYTES: u64 = 64 * 1024;
/// Как часто смотрим счётчики моста.
const SAMPLE_EVERY: Duration = Duration::from_secs(30);
/// Тихая установка ждёт дольше этого - показываем обычное уведомление с кнопкой.
pub const PATIENCE_SECS: u64 = 48 * 3600;
/// После неудачной установки - пауза не меньше 6 часов.
pub const FAIL_COOLDOWN_SECS: u64 = 6 * 3600;
/// Столько неудач подряд для одной версии - и снова ручная кнопка.
pub const MAX_FAILURES: u32 = 3;
/// Установщик ещё мог работать: раньше этого срока по отметке не судим.
const SETTLE_SECS: u64 = 180;
/// Скачивание не вышло: повторять не чаще раза в полчаса.
const PREFETCH_RETRY: Duration = Duration::from_secs(30 * 60);
/// «Обновлено до X» живёт в колокольчике неделю.
const DONE_NOTICE_DAYS: u64 = 7;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- раскатка

/// Предложена ли версия этой установке: номер 0-99 меньше доли `rollout`.
/// 100 - всем, 0 - никому.
pub fn rollout_passes(rollout: u32, number: u8) -> bool {
    u32::from(number) < rollout.min(100)
}

/// Номер из случайной hex-строки (0-99).
fn number_from_hex(hex: &str) -> u8 {
    let n = u64::from_str_radix(&hex[..hex.len().min(16)], 16).unwrap_or(0);
    (n % 100) as u8
}

/// Сохранённый номер, если он годится, иначе новый.
fn pick_number(existing: Option<u8>, fresh: u8) -> u8 {
    existing.filter(|n| *n < 100).unwrap_or(fresh)
}

/// Случайный номер установки 0-99: создаётся один раз и хранится в
/// настройках. Никуда не отправляется.
pub fn rollout_number() -> u8 {
    static CACHED: OnceLock<u8> = OnceLock::new();
    *CACHED.get_or_init(|| {
        if let Some(n) = settings::load().update_rollout.filter(|n| *n < 100) {
            return n;
        }
        let fresh = number_from_hex(&crate::updater::random_hex());
        match settings::with_settings_strict(|s| {
            s.update_rollout = Some(pick_number(s.update_rollout, fresh));
            Ok(())
        }) {
            Ok(s) => pick_number(s.update_rollout, fresh),
            Err(_) => fresh,
        }
    })
}

// ---------------------------------------------------------------- занятость

static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Пока жив, тихая установка ждёт. Берут установка мастера, очистка Claude,
/// экспорт чатов.
pub struct BusyGuard(());

pub fn busy_guard() -> BusyGuard {
    BUSY.fetch_add(1, Ordering::SeqCst);
    BusyGuard(())
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn is_busy() -> bool {
    BUSY.load(Ordering::SeqCst) > 0
}

// ---------------------------------------------------------------- мост

/// Счётчики входов Claude (8889 и 8890): байты и число соединений. Вход
/// Telegram (SOCKS) и закреплённые порты не берём: Telegram пингует раз в
/// минуту, и мост никогда не казался бы тихим.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    pub bytes: u64,
    pub total: u64,
    /// У всех ответивших входов есть поле "bytes" (мост новее этой правки).
    pub has_bytes: bool,
}

/// Окно наблюдения: база (время и счётчики на начало), итог прошлого
/// полного окна и последний замер.
#[derive(Debug, Default)]
pub struct BridgeWindow {
    base: Option<(u64, Counters)>,
    last_quiet: Option<bool>,
    latest: Option<Counters>,
}

fn grew(now: u64, then: u64) -> u64 {
    // Счётчик упал - мост перезапускался, считаем с нуля.
    now.checked_sub(then).unwrap_or(now)
}

/// Тихо ли за промежуток между `base` и `now`.
fn span_quiet(base: Counters, now: Counters) -> bool {
    if now.has_bytes && base.has_bytes {
        grew(now.bytes, base.bytes) < QUIET_BYTES
    } else {
        // Старый мост без байтов: любое новое соединение - не тишина.
        grew(now.total, base.total) == 0
    }
}

/// Новый замер (`t` - секунды). Каждые QUIET_WINDOW_SECS окно закрывается.
pub fn window_step(w: &mut BridgeWindow, t: u64, c: Counters) {
    w.latest = Some(c);
    match w.base {
        None => w.base = Some((t, c)),
        Some((t0, c0)) if t.saturating_sub(t0) >= QUIET_WINDOW_SECS => {
            w.last_quiet = Some(span_quiet(c0, c));
            w.base = Some((t, c));
        }
        Some(_) => {}
    }
}

/// Мост тихий: прошлое полное окно было тихим и в текущем уже набежало
/// меньше порога (поток, начавшийся только что, не пропустим).
pub fn window_quiet(w: &BridgeWindow) -> bool {
    match (w.last_quiet, w.base, w.latest) {
        (Some(true), Some((_, c0)), Some(c)) => span_quiet(c0, c),
        _ => false,
    }
}

static BRIDGE: Mutex<BridgeWindow> = Mutex::new(BridgeWindow { base: None, last_quiet: None, latest: None });

/// Счётчики входов Claude из /healthz. Не ответил (моста нет) - ноль:
/// нечего обрывать.
fn read_counters() -> Counters {
    use crate::monitor::fetch_healthz_port;
    let mut c = Counters { has_bytes: true, ..Counters::default() };
    for port in [crate::shim::LISTEN_PORT, crate::claude_desktop::PROXY_PORT] {
        let Some(raw) = fetch_healthz_port(port, Duration::from_millis(500)) else { continue };
        if raw["service"] != "clod-bridge" {
            continue;
        }
        c.total += raw["total"].as_u64().unwrap_or(0);
        match raw["bytes"].as_u64() {
            Some(b) => c.bytes += b,
            None => c.has_bytes = false,
        }
    }
    c
}

/// Снять замер и вернуть, тихо ли сейчас (блокирующий вызов).
fn sample_bridge() -> bool {
    let c = read_counters();
    let Ok(mut w) = BRIDGE.lock() else { return false };
    window_step(&mut w, now_secs(), c);
    window_quiet(&w)
}

// ---------------------------------------------------------------- простой

/// Секунд без мыши и клавиатуры (None - не удалось узнать).
#[cfg(target_os = "windows")]
pub fn idle_secs() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: li - корректно заполненная структура, cbSize задан.
    if unsafe { GetLastInputInfo(&mut li) } == 0 {
        return None;
    }
    // SAFETY: простой вызов без аргументов. Тики по модулю 2^32: вычитаем с обёрткой.
    let now = unsafe { GetTickCount() };
    Some(u64::from(now.wrapping_sub(li.dwTime)) / 1000)
}

#[cfg(target_os = "macos")]
pub fn idle_secs() -> Option<u64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState = 0, kCGAnyInputEventType = ~0.
    // SAFETY: чистая функция CoreGraphics без указателей.
    let s = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
    (s.is_finite() && s >= 0.0).then_some(s as u64)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn idle_secs() -> Option<u64> {
    None
}

// ---------------------------------------------------------------- условия

/// Окно спрятано в трей или свёрнуто.
pub fn window_hidden(visible: bool, minimized: bool) -> bool {
    !visible || minimized
}

/// Человек не за компьютером не меньше MIN_IDLE_SECS. Не узнали - считаем,
/// что за компьютером.
pub fn user_idle(idle_secs: Option<u64>) -> bool {
    idle_secs.is_some_and(|s| s >= MIN_IDLE_SECS)
}

#[derive(Debug, PartialEq, Eq)]
pub enum FailGate {
    Ok,
    /// Недавняя неудача: ждём не меньше FAIL_COOLDOWN_SECS.
    Cooldown,
    /// Три неудачи подряд: больше сами не пробуем, нужна ручная кнопка.
    Exhausted,
}

/// Неудачи установки именно этой версии.
pub fn failure_gate(f: Option<&UpdateFailures>, version: &str, now: u64) -> FailGate {
    let Some(f) = f.filter(|f| f.version == version) else { return FailGate::Ok };
    if f.count >= MAX_FAILURES {
        FailGate::Exhausted
    } else if now.saturating_sub(f.last_at) < FAIL_COOLDOWN_SECS {
        FailGate::Cooldown
    } else {
        FailGate::Ok
    }
}

/// Всё, от чего зависит решение «ставить сейчас».
#[derive(Debug, Clone)]
pub struct Facts {
    /// `auto_update == Some(true)`.
    pub enabled: bool,
    /// Найденное обновление (не paused: остановленное check() не отдаёт).
    pub update: Option<UpdateView>,
    pub number: u8,
    pub failures: Option<UpdateFailures>,
    pub now: u64,
    pub window_hidden: bool,
    pub idle_secs: Option<u64>,
    /// Мост тихий (см. window_quiet).
    pub bridge_quiet: bool,
    /// В настройках есть неразобранная отметка прошлой установки: итог ещё
    /// не вынесен (on_start), неудача могла бы не засчитаться.
    pub unsettled: bool,
    pub busy: bool,
    /// Установка (ручная или тихая) уже идёт.
    pub installing: bool,
    /// Установщик уже скачан и проверен.
    pub downloaded: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateView {
    pub version: String,
    pub manual_only: bool,
    pub rollout: u32,
}

impl From<&UpdateInfo> for UpdateView {
    fn from(i: &UpdateInfo) -> Self {
        Self { version: i.version.clone(), manual_only: i.manual_only, rollout: i.rollout }
    }
}

/// Решение: ставить сейчас или чего ждём.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Install,
    Wait(&'static str),
}

pub fn decide(f: &Facts) -> Verdict {
    use Verdict::Wait;
    if !f.enabled {
        return Wait("автообновление выключено");
    }
    let Some(u) = &f.update else { return Wait("обновления нет") };
    if f.unsettled {
        return Wait("итог прошлой установки ещё не разобран");
    }
    if u.manual_only {
        return Wait("эта установка обновляется вручную");
    }
    if !rollout_passes(u.rollout, f.number) {
        return Wait("версия ещё не дошла до этой установки");
    }
    match failure_gate(f.failures.as_ref(), &u.version, f.now) {
        FailGate::Ok => {}
        FailGate::Cooldown => return Wait("после неудачи ещё не прошло 6 часов"),
        FailGate::Exhausted => return Wait("три неудачи подряд, нужна ручная установка"),
    }
    if !f.window_hidden {
        return Wait("окно программы открыто");
    }
    if !user_idle(f.idle_secs) {
        return Wait("человек за компьютером");
    }
    if !f.bridge_quiet {
        return Wait("через мост идёт трафик");
    }
    if f.busy {
        return Wait("программа занята другой работой");
    }
    if f.installing {
        return Wait("обновление уже устанавливается");
    }
    if !f.downloaded {
        return Wait("установщик ещё скачивается");
    }
    Verdict::Install
}

/// Обновление ставится само (а не всплывающим уведомлением): автообновление
/// включено, оно не ручное и не исчерпало неудачи.
pub fn handles_silently(s: &Settings, info: &UpdateInfo, now: u64) -> bool {
    s.auto_update == Some(true)
        && !info.manual_only
        && rollout_passes(info.rollout, rollout_number_for(s))
        && failure_gate(s.update_failures.as_ref(), &info.version, now) != FailGate::Exhausted
        && !patience_over(s.update_seen.as_ref(), &info.version, now)
}

/// Тихая установка ждёт эту версию дольше PATIENCE_SECS: пора показать
/// обычное уведомление с кнопкой.
pub fn patience_over(seen: Option<&UpdateSeen>, version: &str, now: u64) -> bool {
    seen.filter(|s| s.version == version).is_some_and(|s| now.saturating_sub(s.at) >= PATIENCE_SECS)
}

/// Запомнить, когда тихая установка впервые взялась за версию.
fn note_seen(version: &str, now: u64) {
    let _ = settings::with_settings_strict(|s| {
        if s.update_seen.as_ref().is_none_or(|x| x.version != version) {
            s.update_seen = Some(UpdateSeen { version: version.to_string(), at: now });
        }
        Ok(())
    });
}

/// Перед самым запуском установщика: окно всё ещё скрыто, человек всё ещё
/// не за компьютером. Нет - установку откладываем без записи неудачи.
pub fn still_quiet(app: &AppHandle) -> bool {
    main_hidden(app) && user_idle(idle_secs())
}

fn main_hidden(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| window_hidden(w.is_visible().unwrap_or(true), w.is_minimized().unwrap_or(false)))
        .unwrap_or(true)
}

/// Какие ошибки установки стоит отправлять в отчёт при тихой установке:
/// файл не сошёлся по сумме или установщик не запустился. Сеть и диск - нет.
pub fn report_worthy(err: &str) -> bool {
    err.contains("контрольная сумма") || err.contains("Не удалось запустить установщик") || err.contains("Не удалось записать скрипт обновления")
}

static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Первый ли это отчёт об этой версии (не чаще одного раза на версию).
fn first_report(version: &str) -> bool {
    let Ok(mut r) = REPORTED.lock() else { return false };
    if r.iter().any(|v| v == version) {
        return false;
    }
    r.push(version.to_string());
    true
}

/// Отчёт об ошибке тихой установки: только достойные и раз на версию.
pub fn report_auto_error(version: &str, err: &str) {
    if report_worthy(err) && first_report(version) {
        crate::telemetry::report_update(err);
    }
}

fn rollout_number_for(s: &Settings) -> u8 {
    s.update_rollout.filter(|n| *n < 100).unwrap_or_else(rollout_number)
}

// ---------------------------------------------------------------- отметка установки

/// Записать «ставлю версию X» перед запуском установщика.
pub fn mark_attempt(version: &str, how: &str) {
    let a = UpdateAttempt { version: version.to_string(), at: now_secs(), how: how.to_string() };
    if let Err(e) = settings::with_settings_strict(|s| {
        s.update_attempt = Some(a);
        Ok(())
    }) {
        tracing::warn!("отметка установки не записана: {e}");
    }
}

/// Снять отметку (установщик не запустился).
pub fn clear_attempt() {
    let _ = settings::with_settings_strict(|s| {
        s.update_attempt = None;
        Ok(())
    });
}

#[derive(Debug, PartialEq, Eq)]
pub enum AttemptState {
    /// Установщик ещё мог работать.
    Pending { wait_secs: u64 },
    Succeeded,
    Failed,
}

pub fn attempt_state(a: &UpdateAttempt, current: &str, now: u64) -> AttemptState {
    let (cur, want) = (crate::updater::parse_ver(current), crate::updater::parse_ver(&a.version));
    if matches!((cur, want), (Some(c), Some(w)) if c >= w) {
        return AttemptState::Succeeded;
    }
    let settle_at = a.at + SETTLE_SECS;
    if now < settle_at {
        AttemptState::Pending { wait_secs: settle_at - now }
    } else {
        AttemptState::Failed
    }
}

/// Счётчик неудач подряд: та же версия - плюс один, другая - с единицы.
pub fn bump_failures(prev: Option<UpdateFailures>, version: &str, now: u64) -> UpdateFailures {
    let count = prev.filter(|p| p.version == version).map_or(0, |p| p.count) + 1;
    UpdateFailures { version: version.to_string(), count, last_at: now }
}

/// Итог проверки отметки при запуске.
#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    Nothing,
    Pending { wait_secs: u64 },
    Succeeded { version: String, how: String },
    Failed { version: String, how: String, count: u32 },
}

/// Разобрать отметку установки по текущей версии и записать итог в `s`.
pub fn settle_on_start(s: &mut Settings, current: &str, now: u64) -> Settled {
    let Some(a) = s.update_attempt.clone() else { return Settled::Nothing };
    match attempt_state(&a, current, now) {
        AttemptState::Pending { wait_secs } => Settled::Pending { wait_secs },
        AttemptState::Succeeded => {
            s.update_attempt = None;
            s.update_failures = None;
            if a.how == "auto" {
                s.update_notice = Some(UpdateNotice { ok: true, version: a.version.clone(), at: now });
            } else if s.update_notice.as_ref().is_some_and(|n| !n.ok) {
                s.update_notice = None;
            }
            Settled::Succeeded { version: a.version, how: a.how }
        }
        AttemptState::Failed => {
            s.update_attempt = None;
            let f = bump_failures(s.update_failures.take(), &a.version, now);
            let count = f.count;
            s.update_failures = Some(f);
            s.update_notice = Some(UpdateNotice { ok: false, version: a.version.clone(), at: now });
            if count >= MAX_FAILURES {
                // Снова обычное уведомление с кнопкой: забыть, что о версии уже сообщали.
                s.update_notified = None;
            }
            Settled::Failed { version: a.version, how: a.how, count }
        }
    }
}

/// Элементы колокольчика про итог установки. Без правки состояния.
pub fn bell_items(s: &Settings, now: u64) -> Vec<crate::notices::NoticeItem> {
    let Some(n) = &s.update_notice else { return Vec::new() };
    if n.ok && now.saturating_sub(n.at) > DONE_NOTICE_DAYS * 86_400 {
        return Vec::new();
    }
    let (id, title, body, link, link_label) = if n.ok {
        (
            format!("update-done-{}", n.version),
            format!("Обновлено до {}", n.version),
            "Новая версия поставилась сама.".to_string(),
            None,
            None,
        )
    } else {
        (
            format!("update-failed-{}", n.version),
            format!("Обновление до {} не установилось", n.version),
            "Установите вручную: скачайте установщик на странице релиза.".to_string(),
            Some(crate::updater::release_page(&n.version)),
            Some("Скачать вручную".to_string()),
        )
    };
    vec![crate::notices::NoticeItem {
        read: s.notices_read.contains(&id),
        id,
        kind: "news",
        date: String::new(),
        title,
        body,
        link,
        link_label,
    }]
}

/// Разобрать отметку установки (если есть) и сообщить об итоге.
async fn settle_once(app: &AppHandle) -> Settled {
    let current = env!("CARGO_PKG_VERSION");
    let res = tokio::task::spawn_blocking(move || {
        let mut out = Settled::Nothing;
        let r = settings::with_settings_strict(|s| {
            out = settle_on_start(s, current, now_secs());
            Ok(())
        });
        (r.is_ok(), out)
    })
    .await;
    let out = match res {
        Ok((true, o)) => o,
        _ => Settled::Nothing,
    };
    match &out {
        Settled::Nothing | Settled::Pending { .. } => {}
        Settled::Succeeded { version, how } => {
            tracing::info!("обновление до {version} установлено ({how})");
            let _ = app.emit("notices:changed", crate::notices::unread());
        }
        Settled::Failed { version, how, count } => {
            tracing::warn!("установка {version} не завершилась ({how}), неудач подряд: {count}");
            crate::telemetry::set_pending_how(None);
            if first_report(version) {
                crate::telemetry::report_update(&format!("установка {version} не завершилась"));
            }
            let _ = app.emit("notices:changed", crate::notices::unread());
        }
    }
    out
}

/// При запуске: удалась ли установка, начатая прошлой копией программы.
pub fn on_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Settled::Pending { wait_secs } = settle_once(&app).await {
            // Установщик, возможно, ещё работает: посмотрим позже.
            tokio::time::sleep(Duration::from_secs(wait_secs + 1)).await;
        }
    });
}

// ---------------------------------------------------------------- цикл

enum Prefetch {
    Idle,
    Running(String),
    Ready(String),
    Failed(String, Instant),
}

static PREFETCH: Mutex<Prefetch> = Mutex::new(Prefetch::Idle);
static TICKING: AtomicBool = AtomicBool::new(false);
/// Не пытаться ставить раньше этого времени (unix, с): сеть или диск подвели.
static RETRY_AFTER: AtomicU64 = AtomicU64::new(0);

pub fn spawn(app: AppHandle) {
    on_start(&app);
    // Счётчики моста: окно в 5 минут набирается по замерам раз в 30 секунд.
    tauri::async_runtime::spawn(async move {
        loop {
            let _ = tokio::task::spawn_blocking(sample_bridge).await;
            tokio::time::sleep(SAMPLE_EVERY).await;
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            tick(&app).await;
        }
    });
}

/// Фоновая проверка нашла обновление: качать и решать сразу, не ждать цикла.
pub fn on_update_found(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { tick(&app).await });
}

/// Скачать заранее и поставить, когда условия сошлись.
async fn tick(app: &AppHandle) {
    if TICKING.swap(true, Ordering::SeqCst) {
        return;
    }
    tick_inner(app).await;
    TICKING.store(false, Ordering::SeqCst);
}

async fn tick_inner(app: &AppHandle) {
    let mut s = settings::load();
    if s.auto_update != Some(true) {
        return;
    }
    // Итог прошлой установки - раньше всего остального: иначе неудача могла
    // бы не засчитаться, а новая попытка пошла бы поверх.
    if s.update_attempt.is_some() {
        settle_once(app).await;
        s = settings::load();
    }
    let Some(info) = crate::update_notify::latest() else { return };
    let now = now_secs();
    note_seen(&info.version, now);
    s.update_seen.get_or_insert(UpdateSeen { version: info.version.clone(), at: now });
    if patience_over(s.update_seen.as_ref(), &info.version, now) {
        // Ждали больше 48 часов: дальше - обычное уведомление с кнопкой.
        return;
    }
    if info.manual_only
        || !rollout_passes(info.rollout, rollout_number_for(&s))
        || failure_gate(s.update_failures.as_ref(), &info.version, now) != FailGate::Ok
    {
        return;
    }
    if now < RETRY_AFTER.load(Ordering::SeqCst) {
        return;
    }
    // Скачивание заранее.
    let downloaded = {
        let mut pf = PREFETCH.lock().unwrap_or_else(|e| e.into_inner());
        match &*pf {
            Prefetch::Ready(v) if *v == info.version => true,
            Prefetch::Running(v) if *v == info.version => return,
            Prefetch::Failed(v, at) if *v == info.version && at.elapsed() < PREFETCH_RETRY => return,
            _ => {
                *pf = Prefetch::Running(info.version.clone());
                false
            }
        }
    };
    if !downloaded {
        let (app, info) = (app.clone(), info.clone());
        tauri::async_runtime::spawn(async move {
            let r = crate::updater::fetch_installer(&info, Mode::Background, &|_| {}).await;
            let ok = r.is_ok();
            if let Err(e) = &r {
                tracing::info!("скачивание обновления заранее: {e}");
            }
            if let Ok(mut pf) = PREFETCH.lock() {
                *pf = if ok { Prefetch::Ready(info.version.clone()) } else { Prefetch::Failed(info.version.clone(), Instant::now()) };
            }
            if ok {
                // Скачано: если условия уже сошлись, не ждём следующего цикла.
                on_update_found(&app);
            }
        });
        return;
    }
    // Факты о системе (простой, мост) - вне async-потока.
    let probe = tokio::task::spawn_blocking(|| (idle_secs(), sample_bridge()))
        .await
        .unwrap_or((None, false));
    // Только отладка: живая проверка на машине, где через мост идёт своя
    // работа (CLOD_UPDATE_BRIDGE_QUIET - считать мост тихим).
    #[cfg(debug_assertions)]
    let probe = if std::env::var_os("CLOD_UPDATE_BRIDGE_QUIET").is_some() { (probe.0, true) } else { probe };
    let hidden = main_hidden(app);
    let facts = Facts {
        enabled: true,
        update: Some(UpdateView::from(&info)),
        number: rollout_number_for(&s),
        failures: s.update_failures.clone(),
        now,
        window_hidden: hidden,
        idle_secs: probe.0,
        bridge_quiet: probe.1,
        unsettled: s.update_attempt.is_some(),
        busy: is_busy(),
        installing: crate::updater::is_installing(),
        downloaded: true,
    };
    match decide(&facts) {
        Verdict::Wait(why) => tracing::debug!("тихая установка ждёт: {why}"),
        Verdict::Install => {
            tracing::info!("тихая установка {}", info.version);
            match crate::commands::install_update(app.clone(), info, "auto").await {
                Ok(()) => {}
                Err(e) if e == crate::updater::ALREADY || e == crate::updater::POSTPONED => {}
                Err(e) => {
                    tracing::warn!("тихая установка не удалась: {e}");
                    RETRY_AFTER.store(now_secs() + PREFETCH_RETRY.as_secs(), Ordering::SeqCst);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Facts {
        Facts {
            enabled: true,
            update: Some(UpdateView { version: "0.9.0".into(), manual_only: false, rollout: 100 }),
            number: 10,
            failures: None,
            now: 1_800_000_000,
            window_hidden: true,
            idle_secs: Some(MIN_IDLE_SECS),
            bridge_quiet: true,
            unsettled: false,
            busy: false,
            installing: false,
            downloaded: true,
        }
    }

    fn waits(f: &Facts) -> &'static str {
        match decide(f) {
            Verdict::Wait(w) => w,
            Verdict::Install => "",
        }
    }

    #[test]
    fn install_when_everything_is_ready() {
        assert_eq!(decide(&good()), Verdict::Install);
    }

    #[test]
    fn each_condition_blocks_installation_on_its_own() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Facts)>)> = vec![
            ("выключено", Box::new(|f| f.enabled = false)),
            ("нет", Box::new(|f| f.update = None)),
            ("вручную", Box::new(|f| f.update.as_mut().unwrap().manual_only = true)),
            ("не дошла", Box::new(|f| f.update.as_mut().unwrap().rollout = 10)),
            ("6 часов", Box::new(|f| f.failures = Some(UpdateFailures { version: "0.9.0".into(), count: 1, last_at: f.now - 60 }))),
            ("ручная", Box::new(|f| f.failures = Some(UpdateFailures { version: "0.9.0".into(), count: 3, last_at: 0 }))),
            ("окно", Box::new(|f| f.window_hidden = false)),
            ("за компьютером", Box::new(|f| f.idle_secs = Some(MIN_IDLE_SECS - 1))),
            ("за компьютером", Box::new(|f| f.idle_secs = None)),
            ("трафик", Box::new(|f| f.bridge_quiet = false)),
            ("не разобран", Box::new(|f| f.unsettled = true)),
            ("занята", Box::new(|f| f.busy = true)),
            ("устанавливается", Box::new(|f| f.installing = true)),
            ("скачивается", Box::new(|f| f.downloaded = false)),
        ];
        for (needle, change) in cases {
            let mut f = good();
            change(&mut f);
            let w = waits(&f);
            assert!(w.contains(needle), "ожидали «{needle}», получили «{w}»");
        }
    }

    #[test]
    fn failures_of_other_versions_do_not_block() {
        let mut f = good();
        f.failures = Some(UpdateFailures { version: "0.8.9".into(), count: 3, last_at: f.now });
        assert_eq!(decide(&f), Verdict::Install);
    }

    #[test]
    fn rollout_is_stable_and_has_edges() {
        // 0 - никому, 100 - всем, граница по номеру.
        for n in [0u8, 50, 99] {
            assert!(!rollout_passes(0, n));
            assert!(rollout_passes(100, n));
            assert!(rollout_passes(250, n), "больше 100 - как 100");
        }
        assert!(rollout_passes(30, 29));
        assert!(!rollout_passes(30, 30));
        // Номер: из hex в диапазоне 0-99, одинаковый вход - одинаковый номер.
        for h in ["0", "ffffffffffffffff", "123456789abcdef0", "deadbeefdeadbeef", "garbage"] {
            let n = number_from_hex(h);
            assert!(n < 100);
            assert_eq!(n, number_from_hex(h));
        }
        // Сохранённый номер важнее свежего; негодный (>= 100) заменяется.
        assert_eq!(pick_number(Some(42), 7), 42);
        assert_eq!(pick_number(None, 7), 7);
        assert_eq!(pick_number(Some(100), 7), 7);
        // Сгенерированные номера не залипают в одной точке.
        let set: std::collections::HashSet<u8> = (0..64).map(|_| number_from_hex(&crate::updater::random_hex())).collect();
        assert!(set.len() > 10);
    }

    #[test]
    fn failure_gate_cooldown_and_exhaustion() {
        let now = 1_000_000;
        let f = |count, last_at| UpdateFailures { version: "0.9.0".into(), count, last_at };
        assert_eq!(failure_gate(None, "0.9.0", now), FailGate::Ok);
        assert_eq!(failure_gate(Some(&f(1, now - 60)), "0.9.0", now), FailGate::Cooldown);
        assert_eq!(failure_gate(Some(&f(2, now - FAIL_COOLDOWN_SECS + 1)), "0.9.0", now), FailGate::Cooldown);
        assert_eq!(failure_gate(Some(&f(1, now - FAIL_COOLDOWN_SECS)), "0.9.0", now), FailGate::Ok, "ровно 6 часов - можно");
        assert_eq!(failure_gate(Some(&f(3, now - 99 * 3600)), "0.9.0", now), FailGate::Exhausted);
        assert_eq!(failure_gate(Some(&f(3, now)), "0.9.1", now), FailGate::Ok, "для новой версии счёт заново");
    }

    #[test]
    fn idle_window_and_bridge_predicates() {
        assert!(!user_idle(Some(MIN_IDLE_SECS - 1)));
        assert!(user_idle(Some(MIN_IDLE_SECS)));
        assert!(!user_idle(None));
        assert!(window_hidden(false, false));
        assert!(window_hidden(true, true));
        assert!(!window_hidden(true, false));
    }

    fn c(bytes: u64, total: u64) -> Counters {
        Counters { bytes, total, has_bytes: true }
    }

    #[test]
    fn bridge_is_quiet_only_after_a_quiet_window_and_without_a_fresh_stream() {
        let mut w = BridgeWindow::default();
        assert!(!window_quiet(&w), "замеров нет - не тихо");
        window_step(&mut w, 0, c(1000, 5));
        assert!(!window_quiet(&w), "окно ещё не закрыто");
        // Пинги Telegram сюда не попадают; здесь мелочь меньше порога.
        window_step(&mut w, 150, c(1000 + 10_000, 6));
        assert!(!window_quiet(&w));
        window_step(&mut w, QUIET_WINDOW_SECS, c(1000 + 20_000, 7));
        assert!(window_quiet(&w), "20 КБ за 5 минут - тихо");
        // Поток начался уже в новом окне: не пропускаем, не дожидаясь его конца.
        window_step(&mut w, QUIET_WINDOW_SECS + 30, c(1000 + 20_000 + QUIET_BYTES, 7));
        assert!(!window_quiet(&w), "идёт ответ");
        // Закрылось шумное окно - не тихо; следующее тихое - тихо.
        window_step(&mut w, 2 * QUIET_WINDOW_SECS, c(1000 + 20_000 + QUIET_BYTES + 5, 7));
        assert!(!window_quiet(&w));
        window_step(&mut w, 3 * QUIET_WINDOW_SECS, c(1000 + 20_000 + QUIET_BYTES + 9, 7));
        assert!(window_quiet(&w));
        // Порог: ровно QUIET_BYTES - уже не тихо.
        let mut w = BridgeWindow::default();
        window_step(&mut w, 0, c(0, 0));
        window_step(&mut w, QUIET_WINDOW_SECS, c(QUIET_BYTES, 0));
        assert!(!window_quiet(&w));
        // Мост перезапустился: счётчик упал - считаем с нуля, не паникуем.
        let mut w = BridgeWindow::default();
        window_step(&mut w, 0, c(9_000_000, 90));
        window_step(&mut w, QUIET_WINDOW_SECS, c(100, 1));
        assert!(window_quiet(&w));
    }

    #[test]
    fn old_bridge_without_bytes_falls_back_to_connections() {
        let old = |total| Counters { bytes: 0, total, has_bytes: false };
        let mut w = BridgeWindow::default();
        window_step(&mut w, 0, old(10));
        window_step(&mut w, QUIET_WINDOW_SECS, old(10));
        assert!(window_quiet(&w), "новых соединений нет");
        window_step(&mut w, QUIET_WINDOW_SECS + 30, old(11));
        assert!(!window_quiet(&w), "новое соединение");
    }

    #[test]
    fn patience_runs_out_after_48_hours_for_the_same_version() {
        let seen = UpdateSeen { version: "0.9.0".into(), at: 1000 };
        assert!(!patience_over(None, "0.9.0", 10_000_000));
        assert!(!patience_over(Some(&seen), "0.9.0", 1000 + PATIENCE_SECS - 1));
        assert!(patience_over(Some(&seen), "0.9.0", 1000 + PATIENCE_SECS));
        assert!(!patience_over(Some(&seen), "0.9.1", 1000 + PATIENCE_SECS * 3), "другая версия - счёт заново");
    }

    #[test]
    fn only_real_install_failures_are_reported_once_per_version() {
        assert!(report_worthy("Файл обновления повреждён (контрольная сумма не совпала) - не ставлю"));
        assert!(report_worthy("Не удалось запустить установщик: отказано в доступе"));
        assert!(!report_worthy("Загрузка: нет сети"));
        assert!(!report_worthy("Не удалось проверить обновления: таймаут"));
        assert!(first_report("9.9.1"));
        assert!(!first_report("9.9.1"));
        assert!(first_report("9.9.2"));
    }

    #[test]
    fn busy_guard_counts_and_releases() {
        let before = BUSY.load(Ordering::SeqCst);
        let a = busy_guard();
        let b = busy_guard();
        assert!(is_busy());
        drop(a);
        assert!(is_busy(), "второй ещё держит");
        drop(b);
        assert_eq!(BUSY.load(Ordering::SeqCst), before);
    }

    fn attempt(version: &str, at: u64, how: &str) -> UpdateAttempt {
        UpdateAttempt { version: version.into(), at, how: how.into() }
    }

    #[test]
    fn attempt_is_cleared_when_version_changed() {
        let a = attempt("0.9.0", 1000, "auto");
        assert_eq!(attempt_state(&a, "0.9.0", 1010), AttemptState::Succeeded);
        assert_eq!(attempt_state(&a, "0.9.5", 5000), AttemptState::Succeeded, "обновились ещё дальше");
        assert_eq!(attempt_state(&a, "0.8.9", 1010), AttemptState::Pending { wait_secs: SETTLE_SECS - 10 });
        assert_eq!(attempt_state(&a, "0.8.9", 1000 + SETTLE_SECS), AttemptState::Failed);
    }

    #[test]
    fn failed_install_is_counted_and_reported_in_bell() {
        let now = 1_800_000_000;
        let mut s = Settings { update_attempt: Some(attempt("0.9.0", now - 1000, "auto")), update_notified: Some("0.9.0".into()), ..Settings::default() };
        let r = settle_on_start(&mut s, "0.8.18", now);
        assert_eq!(r, Settled::Failed { version: "0.9.0".into(), how: "auto".into(), count: 1 });
        assert!(s.update_attempt.is_none(), "отметка снята");
        assert_eq!(s.update_failures, Some(UpdateFailures { version: "0.9.0".into(), count: 1, last_at: now }));
        assert_eq!(s.update_notified.as_deref(), Some("0.9.0"), "до трёх неудач тост не повторяем");
        let items = bell_items(&s, now);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Обновление до 0.9.0 не установилось");
        assert_eq!(items[0].id, "update-failed-0.9.0");
        assert_eq!(items[0].link.as_deref(), Some("https://github.com/iimperium-dev/ClodHelper/releases/tag/v0.9.0"));
        assert!(!items[0].read);
        // Вторая и третья неудача подряд: счёт растёт; на третьей снова обычное уведомление.
        s.update_attempt = Some(attempt("0.9.0", now, "auto"));
        assert!(matches!(settle_on_start(&mut s, "0.8.18", now + 4000), Settled::Failed { count: 2, .. }));
        s.update_attempt = Some(attempt("0.9.0", now, "manual"));
        assert!(matches!(settle_on_start(&mut s, "0.8.18", now + 9000), Settled::Failed { count: 3, .. }));
        assert_eq!(s.update_notified, None, "после 3 неудач тост снова покажется");
        assert_eq!(failure_gate(s.update_failures.as_ref(), "0.9.0", now + 9000), FailGate::Exhausted);
        // Неудача другой версии - счёт заново.
        s.update_attempt = Some(attempt("0.9.1", now, "auto"));
        assert!(matches!(settle_on_start(&mut s, "0.8.18", now + 20_000), Settled::Failed { count: 1, .. }));
    }

    #[test]
    fn pending_attempt_is_left_alone_and_success_clears_everything() {
        let now = 1_800_000_000;
        let mut s = Settings { update_attempt: Some(attempt("0.9.0", now - 10, "auto")), ..Settings::default() };
        assert!(matches!(settle_on_start(&mut s, "0.8.18", now), Settled::Pending { .. }));
        assert!(s.update_attempt.is_some() && s.update_failures.is_none() && s.update_notice.is_none());
        // Версия сменилась: тихая установка - в колокольчике «Обновлено до X», счётчик сброшен.
        s.update_failures = Some(UpdateFailures { version: "0.9.0".into(), count: 2, last_at: 1 });
        let r = settle_on_start(&mut s, "0.9.0", now);
        assert_eq!(r, Settled::Succeeded { version: "0.9.0".into(), how: "auto".into() });
        assert!(s.update_attempt.is_none() && s.update_failures.is_none());
        let items = bell_items(&s, now);
        assert_eq!(items[0].title, "Обновлено до 0.9.0");
        assert!(items[0].link.is_none());
        assert!(bell_items(&s, now + 8 * 86_400).is_empty(), "через неделю пункт уходит");
        // Ручная установка: итог «обновились» в колокольчик не пишется, старая запись о сбое снимается.
        let mut s = Settings {
            update_attempt: Some(attempt("0.9.1", now - 500, "manual")),
            update_notice: Some(UpdateNotice { ok: false, version: "0.9.1".into(), at: 1 }),
            ..Settings::default()
        };
        settle_on_start(&mut s, "0.9.1", now);
        assert!(s.update_notice.is_none());
        // Нет отметки - ничего не происходит.
        assert_eq!(settle_on_start(&mut Settings::default(), "0.9.0", now), Settled::Nothing);
    }

    #[test]
    fn bump_counts_only_same_version() {
        let a = bump_failures(None, "0.9.0", 5);
        assert_eq!((a.count, a.last_at), (1, 5));
        let b = bump_failures(Some(a), "0.9.0", 9);
        assert_eq!((b.count, b.last_at), (2, 9));
        assert_eq!(bump_failures(Some(b), "0.9.1", 11).count, 1);
    }

    #[test]
    fn silent_handling_replaces_the_toast_only_when_it_will_really_happen() {
        let info = |manual_only, rollout| UpdateInfo {
            version: "0.9.0".into(),
            notes: String::new(),
            url: String::new(),
            sha256: String::new(),
            highlights: vec![],
            force: false,
            manual_only,
            page: String::new(),
            rollout,
            handles_silently: false,
        };
        let on = Settings { auto_update: Some(true), update_rollout: Some(5), ..Settings::default() };
        let now = 1_000_000;
        assert!(handles_silently(&on, &info(false, 100), now));
        assert!(!handles_silently(&Settings { auto_update: Some(false), ..on.clone() }, &info(false, 100), now));
        assert!(!handles_silently(&Settings { auto_update: None, ..on.clone() }, &info(false, 100), now));
        assert!(!handles_silently(&on, &info(true, 100), now), "ручное - с кнопкой");
        assert!(!handles_silently(&on, &info(false, 5), now), "не дошло по раскатке");
        let spent = Settings { update_failures: Some(UpdateFailures { version: "0.9.0".into(), count: 3, last_at: now }), ..on.clone() };
        assert!(!handles_silently(&spent, &info(false, 100), now), "после трёх неудач - обычное уведомление");
        let waited = Settings { update_seen: Some(UpdateSeen { version: "0.9.0".into(), at: now - PATIENCE_SECS }), ..on.clone() };
        assert!(!handles_silently(&waited, &info(false, 100), now), "ждём больше 48 часов - обычное уведомление");
    }
}
