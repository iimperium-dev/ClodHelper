//! Живой статус в трее + системные уведомления.
//!
//! Фоновый поток раз в 5 секунд:
//!   - жив ли мост (лёгкий TCP-чек порта, без скана процессов);
//!   - /healthz нового моста (мини-HTTP-клиент на std::net, без async);
//!   - раз в ~60с — латентность TCP-коннекта до upstream-прокси.
//!
//! Результат: иконка трея меняет цвет (зелёная/жёлтая/красная точка),
//! tooltip и статус-строка меню показывают режим и число соединений,
//! а на ПЕРЕХОДАХ состояний летят системные тосты (мост упал/ожил,
//! upstream не отвечает, высокий пинг) — с дебаунсом, чтобы не спамить.
//!
//! Легаси-мост (python, без /healthz): после двух неудачных парсов healthz
//! перестаём слать GET (он уходил бы в upstream мусором) и минуту проверяем
//! только порт; потом пробуем healthz снова (вдруг мост обновили).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::shim::{self, LISTEN_PORT};

#[cfg(not(target_os = "macos"))]
const TRAY_OK: &[u8] = include_bytes!("../icons/tray-ok.png");
#[cfg(not(target_os = "macos"))]
const TRAY_WARN: &[u8] = include_bytes!("../icons/tray-warn.png");
#[cfg(not(target_os = "macos"))]
const TRAY_DOWN: &[u8] = include_bytes!("../icons/tray-down.png");

/// macOS: «C» в цвет строки меню + цветная точка состояния (зелёная /
/// жёлтая / красная). Template-картинку с цветной точкой сделать нельзя:
/// macOS перекрашивает её целиком — поэтому по два варианта на тему.
/// Template — только первая картинка до первого замера.
/// Генератор: scripts/gen-tray-icons-mac.py.
#[cfg(target_os = "macos")]
pub const MAC_TEMPLATE: &[u8] = include_bytes!("../icons/tray-mac-template.png");
#[cfg(target_os = "macos")]
const MAC_OK: [&[u8]; 2] = [
    include_bytes!("../icons/tray-mac-ok-light.png"),
    include_bytes!("../icons/tray-mac-ok-dark.png"),
];
#[cfg(target_os = "macos")]
const MAC_WARN: [&[u8]; 2] = [
    include_bytes!("../icons/tray-mac-warn-light.png"),
    include_bytes!("../icons/tray-mac-warn-dark.png"),
];
#[cfg(target_os = "macos")]
const MAC_DOWN: [&[u8]; 2] = [
    include_bytes!("../icons/tray-mac-down-light.png"),
    include_bytes!("../icons/tray-mac-down-dark.png"),
];

/// Тёмная ли строка меню (системная тема). Дёшево: чтение NSUserDefaults.
#[cfg(target_os = "macos")]
fn menu_bar_dark() -> bool {
    use objc2_foundation::{NSString, NSUserDefaults};
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults
        .stringForKey(&NSString::from_str("AppleInterfaceStyle"))
        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark"))
}

/// Картинка трея для состояния: (байты PNG, template?) + «вариант» для
/// сравнения (на macOS тревожная иконка зависит ещё и от темы).
fn tray_icon_for(state: BridgeState) -> (&'static [u8], bool, u8) {
    #[cfg(target_os = "macos")]
    {
        let dark = usize::from(menu_bar_dark());
        match state {
            BridgeState::Ok => (MAC_OK[dark], false, 5 + dark as u8),
            BridgeState::Warn => (MAC_WARN[dark], false, 1 + dark as u8),
            BridgeState::Down => (MAC_DOWN[dark], false, 3 + dark as u8),
        }
    }
    #[cfg(not(target_os = "macos"))]
    match state {
        BridgeState::Ok => (TRAY_OK, false, 0),
        BridgeState::Warn => (TRAY_WARN, false, 1),
        BridgeState::Down => (TRAY_DOWN, false, 3),
    }
}

const TICK: Duration = Duration::from_secs(5);
/// Пинг до upstream: раз в 12 тиков (~60с); «высоким» считаем > 1500 мс.
const LATENCY_EVERY_TICKS: u32 = 12;
const LATENCY_WARN_MS: u128 = 1500;
/// Высокий пинг должен повториться столько замеров подряд (~2 мин).
const PING_ALERT_STREAK: u32 = 2;
/// Прокси нездоров столько тиков подряд (~30с), прежде чем тревожить.
const UPSTREAM_ALERT_TICKS: u32 = 6;

/// Ручки трея, которые монитор обновляет на лету (иконка + tooltip;
/// нативного меню нет — статус живёт в попапе и tooltip'е).
pub struct TrayHandles {
    pub tray: TrayIcon<tauri::Wry>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum BridgeState {
    Ok,
    Warn,
    Down,
}

/// Минимальное зеркало /healthz (только нужные монитору поля).
#[derive(Deserialize)]
struct Health {
    mode: String,
    active: u64,
    upstreams: Vec<UpHealth>,
}

#[derive(Deserialize)]
struct UpHealth {
    url: String,
    /// Мост ≥1.5: false только после серии отказов подряд, а не после
    /// единичного сбоя — уведомление больше не дёргается от каждого флапа.
    healthy: bool,
}

/// Дебаунс уведомлений: не чаще чем раз в min на каждый тип события.
struct Debounce {
    last: Option<Instant>,
    min: Duration,
}

impl Debounce {
    fn new(min: Duration) -> Self {
        Self { last: None, min }
    }
    fn ready(&mut self) -> bool {
        let ok = self.last.map(|t| t.elapsed() >= self.min).unwrap_or(true);
        if ok {
            self.last = Some(Instant::now());
        }
        ok
    }
}

pub fn spawn(app: AppHandle, handles: TrayHandles) {
    std::thread::spawn(move || run(app, handles));
}

fn run(app: AppHandle, h: TrayHandles) {
    let mut prev_state: Option<BridgeState> = None;
    let mut prev_upstream_ok = true;
    let mut healthz_fail_streak: u32 = 0;
    let mut skip_healthz_until: Option<Instant> = None;
    let mut last_latency_ms: Option<u128> = None;
    let mut tick: u32 = 0;

    let mut nf_down = Debounce::new(Duration::from_secs(60));
    let mut nf_up = Debounce::new(Duration::from_secs(60));
    let mut nf_upstream = Debounce::new(Duration::from_secs(300));
    let mut nf_upstream_back = Debounce::new(Duration::from_secs(300));
    let mut upstream_alerted = false;
    let mut prev_tooltip = String::new();
    let mut prev_icon: Option<u8> = None;
    let mut nf_ping = Debounce::new(Duration::from_secs(600));
    // Сколько замеров подряд пинг высокий / тиков подряд прокси нездоров.
    // Тревожим только затяжные проблемы: единичный потерянный пакет при
    // TCP-коннекте — это повтор SYN через 1–2с, а не «медленный прокси».
    let mut ping_high_streak: u32 = 0;
    let mut upstream_bad_ticks: u32 = 0;

    loop {
        let listening = port_listening();

        // healthz — только если порт жив и мы не в «легаси-паузе».
        let health: Option<Health> = if listening
            && skip_healthz_until.map(|t| Instant::now() > t).unwrap_or(true)
        {
            match fetch_healthz() {
                Some((x, raw)) => {
                    healthz_fail_streak = 0;
                    skip_healthz_until = None;
                    // Один опрос моста на всё приложение: окна получают
                    // свежий /healthz событием и сами мост не опрашивают.
                    let _ = app.emit("bridge-health", raw);
                    Some(x)
                }
                None => {
                    healthz_fail_streak += 1;
                    if healthz_fail_streak >= 2 {
                        // Легаси-мост: не мусорить GET'ами через upstream.
                        skip_healthz_until = Some(Instant::now() + Duration::from_secs(60));
                    }
                    None
                }
            }
        } else {
            None
        };

        // Латентность до upstream — раз в LATENCY_EVERY_TICKS.
        if tick % LATENCY_EVERY_TICKS == 0 {
            if let Some(hh) = &health {
                if let Some(addr) = hh.upstreams.first().and_then(|u| upstream_addr(&u.url)) {
                    last_latency_ms = measure_latency_median(&addr);
                    let high = last_latency_ms.is_some_and(|ms| ms > LATENCY_WARN_MS);
                    ping_high_streak = if high { ping_high_streak + 1 } else { 0 };
                    if ping_high_streak == PING_ALERT_STREAK && internet_ok() && nf_ping.ready() {
                        let ms = last_latency_ms.unwrap_or_default();
                        notify(&app, &format!("Прокси отвечает медленно ({ms} мс) уже пару минут — соединения могут тормозить"));
                    }
                }
            }
        }

        // Telegram мимо моста → один раз сами открываем его окно прокси.
        if tick % LATENCY_EVERY_TICKS == 6 && listening {
            crate::telegram::auto_prompt_once();
        }

        let upstream_ok = health
            .as_ref()
            .map(|hh| hh.upstreams.iter().all(|u| u.healthy))
            .unwrap_or(true);
        let high_ping = ping_high_streak >= PING_ALERT_STREAK;

        let state = if !listening {
            BridgeState::Down
        } else if !upstream_ok || high_ping {
            BridgeState::Warn
        } else {
            BridgeState::Ok
        };

        // ── Иконка: меняем только на переходе (без мерцания) ──
        // (на macOS ещё и при смене темы, пока иконка тревожная)
        let (bytes, template, variant) = tray_icon_for(state);
        if prev_icon != Some(variant) {
            // Порядок важен: set_icon создаёт новую картинку БЕЗ флага template
            // (tray-icon), флаг ставится уже на неё. Наоборот — чёрная «C»
            // на тёмной строке меню, то есть невидимая иконка.
            if let Ok(img) = tauri::image::Image::from_bytes(bytes) {
                let _ = h.tray.set_icon(Some(img));
            }
            let _ = h.tray.set_icon_as_template(template);
            prev_icon = Some(variant);
        }

        // ── Tooltip ──
        // set_tooltip — вызов в UI-поток; дёргаем только когда текст поменялся.
        let tooltip = describe(state, &health, last_latency_ms);
        if tooltip != prev_tooltip {
            let _ = h.tray.set_tooltip(Some(&tooltip));
            prev_tooltip = tooltip;
        }

        // ── Уведомления на переходах ──
        match (prev_state, state) {
            (Some(BridgeState::Ok | BridgeState::Warn), BridgeState::Down) => {
                if !shim::manually_stopped() && nf_down.ready() {
                    notify(&app, "Мост остановился — он перезапустится автоматически в течение ~5 секунд");
                }
            }
            (Some(BridgeState::Down), BridgeState::Ok | BridgeState::Warn) => {
                if nf_up.ready() {
                    notify(&app, "Мост снова в строю ✅");
                }
            }
            _ => {}
        }
        upstream_bad_ticks = if upstream_ok { 0 } else { upstream_bad_ticks + 1 };
        // Тревога — только если сбой держится UPSTREAM_ALERT_TICKS тиков (~30с)
        // и интернет у самого Mac есть: при отвалившемся Wi‑Fi прокси ни при чём.
        if upstream_bad_ticks == UPSTREAM_ALERT_TICKS && internet_ok() && nf_upstream.ready() {
            let pool = health.as_ref().map(|hh| hh.upstreams.len()).unwrap_or(1);
            notify(&app, &upstream_down_text(pool));
            upstream_alerted = true;
        }
        // Парное «отбой тревоги» — иначе пользователь не знает, кончилось ли.
        if upstream_alerted && !prev_upstream_ok && upstream_ok && nf_upstream_back.ready() {
            notify(&app, "Прокси снова отвечает — соединения идут как обычно");
            upstream_alerted = false;
        }

        prev_state = Some(state);
        prev_upstream_ok = upstream_ok;
        tick = tick.wrapping_add(1);
        std::thread::sleep(TICK);
    }
}

/// Текст тревоги по прокси. Закреплённый сервис (Claude) НИКОГДА не уходит
/// на другой прокси — обещать «failover» нельзя: мост повторяет попытку к
/// тому же прокси и ждёт его, выходной IP не меняется.
fn upstream_down_text(pool: usize) -> String {
    if pool > 1 {
        "Прокси перестал отвечать. Claude ждёт свой прокси (IP не меняется), \
         остальные сервисы переходят на резервный"
            .into()
    } else {
        "Прокси перестал отвечать. Мост повторяет подключение к нему же — \
         IP не меняется, трафик в обход прокси не идёт"
            .into()
    }
}

fn describe(
    state: BridgeState,
    health: &Option<Health>,
    latency: Option<u128>,
) -> String {
    match state {
        BridgeState::Down => {
            if shim::manually_stopped() {
                "Clod Helper — мост остановлен вручную".into()
            } else {
                "Clod Helper — мост не работает!".into()
            }
        }
        _ => match health {
            Some(hh) => {
                let mode = if hh.mode == "smart" { "smart" } else { "весь трафик" };
                let ping = latency.map(|ms| format!(" · пинг {ms} мс")).unwrap_or_default();
                let warn = if state == BridgeState::Warn { " ⚠" } else { "" };
                format!("Clod Helper — мост активен · {mode} · соединений: {}{ping}{warn}", hh.active)
            }
            None => "Clod Helper — мост активен (легаси, без телеметрии)".into(),
        },
    }
}

fn notify(app: &AppHandle, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title("Clod Helper")
        .body(body)
        .show();
}

/// Порт моста слушается? Лёгкий connect — без скана процессов.
fn port_listening() -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{LISTEN_PORT}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(600)).is_ok()
}

/// GET /healthz мини-клиентом на std::net (монитор — обычный поток, не tokio).
/// Счётчики соединений — по всему мосту (см. merge_listener).
fn fetch_healthz() -> Option<(Health, serde_json::Value)> {
    let mut raw = fetch_healthz_port(LISTEN_PORT, Duration::from_millis(800))?;
    for port in other_health_ports() {
        if let Some(other) = fetch_healthz_port(port, Duration::from_millis(300)) {
            merge_listener(&mut raw, &other);
        }
    }
    let h = serde_json::from_value::<Health>(raw.clone()).ok()?;
    Some((h, raw))
}

fn fetch_healthz_port(port: u16, connect: Duration) -> Option<serde_json::Value> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut s = TcpStream::connect_timeout(&addr, connect).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let body = text.split("\r\n\r\n").nth(1)?;
    serde_json::from_str(body.trim()).ok()
}

/// Остальные HTTP-входы моста со своими счётчиками: строгий вход Claude и
/// закреплённые за прокси пула (по одному на прокси). SOCKS-вход Telegram
/// (8891) не HTTP — его статистики в /healthz нет.
pub(crate) fn other_health_ports() -> Vec<u16> {
    let pool = crate::settings::effective_proxies(&crate::settings::load()).len() as u16;
    [crate::claude_desktop::PROXY_PORT, crate::tasks::SITES_PORT]
        .into_iter()
        .chain((0..pool.min(5)).map(|i| crate::tasks::PIN_PORT_BASE + i))
        .collect()
}

/// Добавить к /healthz общего входа счётчики другого входа моста: у каждого
/// входа они свои (upstream'ы — общие), а «Обзор» и трей показывают трафик
/// всего моста — Claude Desktop, например, ходит только через свой вход
/// 8890, и по общему его трафика не было видно вовсе.
pub(crate) fn merge_listener(main: &mut serde_json::Value, other: &serde_json::Value) {
    if other["service"] != "clod-bridge" {
        return;
    }
    for key in ["total", "active", "via_upstream", "via_direct", "errors"] {
        let sum = main[key].as_u64().unwrap_or(0) + other[key].as_u64().unwrap_or(0);
        main[key] = sum.into();
    }
    // Журналы сбоев — на общей шкале времени (все входы в одном процессе).
    if let Some(theirs) = other["last_errors"].as_array().filter(|a| !a.is_empty()) {
        let mut all: Vec<serde_json::Value> = main["last_errors"].as_array().cloned().unwrap_or_default();
        all.extend(theirs.iter().cloned());
        all.sort_by_key(|e| e["t"].as_u64().unwrap_or(0));
        let keep = all.split_off(all.len().saturating_sub(20));
        main["last_errors"] = keep.into();
    }
}

/// host:port из masked-URL upstream'а ("http://user:****@host:port" | "http://host:port").
fn upstream_addr(url_masked: &str) -> Option<String> {
    let rest = url_masked.split("://").nth(1)?;
    Some(rest.rsplit('@').next()?.trim_end_matches('/').to_string())
}

/// Медиана трёх TCP-коннектов до upstream, мс. Один замер врёт: потерянный
/// SYN даёт «2–3 секунды» при живом прокси. Неудачные попытки не считаются;
/// None — не подключились ни разу.
fn measure_latency_median(addr: &str) -> Option<u128> {
    let mut v: Vec<u128> = (0..3).filter_map(|_| measure_connect_ms(addr)).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Есть ли интернет у самого Mac (мимо прокси)? Если нет — проблема в Wi‑Fi,
/// а не в прокси, и тревожить «прокси не отвечает» нельзя. Два независимых
/// адреса: Cloudflare и Яндекс DNS.
fn internet_ok() -> bool {
    ["1.1.1.1:443", "77.88.8.8:53"].iter().any(|a| {
        a.parse::<SocketAddr>()
            .is_ok_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).is_ok())
    })
}

/// Время TCP-коннекта до upstream, мс. None — не подключились (это отдельно
/// подсветит healthz как unhealthy при реальном использовании).
fn measure_connect_ms(addr: &str) -> Option<u128> {
    let sock = addr.to_socket_addrs().ok()?.next()?;
    let start = Instant::now();
    TcpStream::connect_timeout(&sock, Duration::from_secs(4)).ok()?;
    Some(start.elapsed().as_millis())
}

#[cfg(test)]
mod tests {
    use super::merge_listener;
    use serde_json::json;

    #[test]
    fn overview_counts_every_bridge_listener() {
        let mut main = json!({"service": "clod-bridge", "total": 2, "active": 0, "via_upstream": 0, "via_direct": 2,
            "errors": 1, "last_errors": [{"t": 50, "msg": "a"}], "upstreams": [{"url": "u", "healthy": true}]});
        let claude = json!({"service": "clod-bridge", "total": 3441, "active": 28, "via_upstream": 3439, "via_direct": 0,
            "errors": 2, "last_errors": [{"t": 10, "msg": "b"}, {"t": 90, "msg": "c"}]});
        merge_listener(&mut main, &claude);
        assert_eq!((main["total"].as_u64(), main["active"].as_u64()), (Some(3443), Some(28)));
        assert_eq!((main["via_upstream"].as_u64(), main["via_direct"].as_u64()), (Some(3439), Some(2)));
        assert_eq!(main["errors"].as_u64(), Some(3));
        let order: Vec<u64> = main["last_errors"].as_array().unwrap().iter().map(|e| e["t"].as_u64().unwrap()).collect();
        assert_eq!(order, vec![10, 50, 90], "журнал сбоев — по времени");
        assert_eq!(main["upstreams"].as_array().unwrap().len(), 1, "upstream-ы общие, не дублируются");
        // Чужой сервис на порту моста не примешиваем.
        merge_listener(&mut main, &json!({"service": "nginx", "total": 100}));
        assert_eq!(main["total"].as_u64(), Some(3443));
    }
}
