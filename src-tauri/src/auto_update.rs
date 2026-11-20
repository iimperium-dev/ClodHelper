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

use crate::settings::{self, Settings, UpdateAttempt, UpdateFailures, UpdateNotice};
use crate::updater::{Mode, UpdateInfo};

/// Пока ждём условий - проверка раз в 5 минут.
const TICK: Duration = Duration::from_secs(5 * 60);
/// Человек не трогал мышь и клавиатуру не меньше 10 минут.
pub const MIN_IDLE_SECS: u64 = 10 * 60;
/// Мост «тихий»: 5 минут не росли счётчики трафика. Открытые соединения
/// не считаем: Claude Code держит их, пока запущен.
pub const BRIDGE_QUIET_SECS: u64 = 5 * 60;
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

#[derive(Default)]
struct BridgeSeen {
    last_counter: Option<u64>,
    /// Когда в последний раз рос счётчик трафика.
    busy_at: Option<Instant>,
}

static BRIDGE: Mutex<BridgeSeen> = Mutex::new(BridgeSeen { last_counter: None, busy_at: None });

/// Счётчик трафика из /healthz: байты (поле "bytes"), если мост их отдаёт,
/// иначе число принятых соединений "total". Активные соединения не берём.
fn traffic_counter(raw: &serde_json::Value) -> Option<u64> {
    raw["bytes"].as_u64().or_else(|| raw["total"].as_u64())
}

/// Монитор моста (monitor.rs) передаёт каждый свежий /healthz: по нему видно,
/// когда через мост в последний раз рос счётчик трафика.
pub fn note_bridge(raw: &serde_json::Value) {
    let counter = traffic_counter(raw);
    let Ok(mut b) = BRIDGE.lock() else { return };
    if matches!((b.last_counter, counter), (Some(a), Some(c)) if a != c) {
        b.busy_at = Some(Instant::now());
    }
    if counter.is_some() {
        b.last_counter = counter;
    }
}

/// Свежий /healthz всех входов моста обновляет счётчики трафика.
fn sample_bridge() {
    use crate::monitor::{fetch_healthz_port, merge_listener, other_health_ports};
    let Some(mut raw) = fetch_healthz_port(crate::shim::LISTEN_PORT, Duration::from_millis(800)) else { return };
    for port in other_health_ports() {
        if let Some(other) = fetch_healthz_port(port, Duration::from_millis(300)) {
            merge_listener(&mut raw, &other);
        }
    }
    note_bridge(&raw);
}

/// Сколько секунд назад рос счётчик трафика (None - с запуска не видели).
fn bridge_quiet_secs() -> Option<u64> {
    BRIDGE.lock().ok().and_then(|b| b.busy_at).map(|t| t.elapsed().as_secs())
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

/// Счётчики трафика моста не росли BRIDGE_QUIET_SECS. Открытые соединения
/// не мешают. `quiet_for` - сколько секунд мост молчит (None - не видели).
pub fn bridge_quiet(quiet_for: Option<u64>) -> bool {
    quiet_for.is_none_or(|q| q >= BRIDGE_QUIET_SECS)
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
    pub bridge_quiet_secs: Option<u64>,
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
    if !bridge_quiet(f.bridge_quiet_secs) {
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

/// При запуске: удалась ли установка, начатая прошлой копией программы.
pub fn on_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut again = true;
        while again {
            again = false;
            let current = env!("CARGO_PKG_VERSION");
            let mut out = Settled::Nothing;
            let res = tokio::task::spawn_blocking(move || {
                let mut out = Settled::Nothing;
                let r = settings::with_settings_strict(|s| {
                    out = settle_on_start(s, current, now_secs());
                    Ok(())
                });
                (r.is_ok(), out)
            })
            .await;
            if let Ok((true, o)) = res {
                out = o;
            }
            match out {
                Settled::Nothing => {}
                Settled::Pending { wait_secs } => {
                    // Установщик, возможно, ещё работает: посмотрим позже.
                    tokio::time::sleep(Duration::from_secs(wait_secs + 1)).await;
                    again = true;
                }
                Settled::Succeeded { version, how } => {
                    tracing::info!("обновление до {version} установлено ({how})");
                    let _ = app.emit("notices:changed", crate::notices::unread());
                }
                Settled::Failed { version, how, count } => {
                    tracing::warn!("установка {version} не завершилась ({how}), неудач подряд: {count}");
                    crate::telemetry::set_pending_how(None);
                    crate::telemetry::report_update(&format!("установка {version} не завершилась"));
                    let _ = app.emit("notices:changed", crate::notices::unread());
                }
            }
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
    let s = settings::load();
    if s.auto_update != Some(true) {
        return;
    }
    let Some(info) = crate::update_notify::latest() else { return };
    let now = now_secs();
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
    let probe = tokio::task::spawn_blocking(|| { sample_bridge(); (idle_secs(), bridge_quiet_secs()) })
        .await
        .unwrap_or((None, Some(0)));
    let hidden = app
        .get_webview_window("main")
        .map(|w| window_hidden(w.is_visible().unwrap_or(true), w.is_minimized().unwrap_or(false)))
        .unwrap_or(true);
    let facts = Facts {
        enabled: true,
        update: Some(UpdateView::from(&info)),
        number: rollout_number_for(&s),
        failures: s.update_failures.clone(),
        now,
        window_hidden: hidden,
        idle_secs: probe.0,
        bridge_quiet_secs: probe.1,
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
                Err(e) if e == crate::updater::ALREADY => {}
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
            bridge_quiet_secs: Some(BRIDGE_QUIET_SECS),
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
            ("трафик", Box::new(|f| f.bridge_quiet_secs = Some(BRIDGE_QUIET_SECS - 1))),
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
        assert!(!bridge_quiet(Some(BRIDGE_QUIET_SECS - 1)));
        assert!(bridge_quiet(Some(BRIDGE_QUIET_SECS)));
        assert!(bridge_quiet(None), "трафика не видели");
    }

    #[test]
    fn bridge_traffic_is_remembered_from_healthz() {
        // Общий статик: тест единственный, кто его трогает.
        *BRIDGE.lock().unwrap() = BridgeSeen::default();
        note_bridge(&serde_json::json!({"active": 0, "total": 10}));
        assert_eq!(bridge_quiet_secs(), None, "первый замер: трафика не видно");
        note_bridge(&serde_json::json!({"active": 0, "total": 10}));
        assert_eq!(bridge_quiet_secs(), None);
        note_bridge(&serde_json::json!({"active": 0, "total": 12}));
        assert_eq!(bridge_quiet_secs(), Some(0), "счётчик вырос: был трафик");
        *BRIDGE.lock().unwrap() = BridgeSeen::default();
        note_bridge(&serde_json::json!({"active": 2, "total": 5}));
        note_bridge(&serde_json::json!({"active": 2, "total": 5}));
        assert_eq!(bridge_quiet_secs(), None, "открытые соединения трафиком не считаются");
        *BRIDGE.lock().unwrap() = BridgeSeen::default();
        note_bridge(&serde_json::json!({"total": 5, "bytes": 100}));
        note_bridge(&serde_json::json!({"total": 5, "bytes": 900}));
        assert_eq!(bridge_quiet_secs(), Some(0), "байты растут при том же total");
        *BRIDGE.lock().unwrap() = BridgeSeen::default();
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
    }
}
