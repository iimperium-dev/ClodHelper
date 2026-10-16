//! clod-bridge — локальный HTTP-прокси-мост (Rust-реинкарнация local-proxy.py).
//!
//! Слушает 127.0.0.1:8889 без auth и решает, куда отправить каждое соединение:
//!   - AI-домены (Anthropic, OpenAI, …) → купленный upstream-прокси с auth
//!     (Proxy-Authorization впрыскивается автоматически);
//!   - всё остальное → напрямую (DIRECT), не тратя трафик upstream'а.
//!
//! Зачем Rust вместо PyInstaller-python: один статический exe ~1МБ без
//! антивирусных фолспозитивов и медленной распаковки onefile; и мост стал
//! достаточно умным для маршрутизации/failover — на python это уже боль.
//!
//! Возможности сверх старого моста:
//!   - domain-routing (режимы: all — всё через upstream, как раньше;
//!     smart — через upstream только AI-домены);
//!   - несколько --upstream с passive failover (упавший в отлёжке 30с);
//!   - GET /healthz на самом мосте — живые счётчики для Dashboard.
//!
//! CLI совместим со старым мостом: --upstream URL (повторяемый), --listen
//! HOST:PORT, --quiet. Новое: --rules FILE (JSON: mode/proxy_domains/
//! direct_domains). Без rules-файла ведёт себя ровно как старый мост (all).
//! Дополнительно слушает 127.0.0.1:8890 (--strict-listen): этот вход всегда
//! отправляет запросы через upstream, независимо от rules, без DIRECT.
//!
//! --supervise: режим самосупервизии — процесс перезапускает сам себя как
//! воркер и поднимает упавшего через 5с. Задача Планировщика вешается прямо
//! на exe: VBS/wscript-цикл (малварь-паттерн для антивирусов + депрекация
//! VBScript в Win11 24H2+) больше не нужен. windows_subsystem="windows" —
//! окна консоли нет ни у супервизора, ни у воркера.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// Потолок одновременных соединений — защита от пика деплоя (см. историю
/// python-моста: без лимита too-many-threads ронял процесс и SSE-стрим).
const MAX_CONNECTIONS: usize = 256;
/// Максимум байтов заголовков первого запроса.
const HEAD_LIMIT: usize = 65536;
/// Таймаут установки исходящего TCP-соединения (upstream или direct).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Таймаут чтения заголовков от клиента.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Отлёжка упавшего upstream'а перед повторной попыткой.
const UPSTREAM_COOLDOWN: Duration = Duration::from_secs(30);

/// AI-домены по умолчанию для режима smart (матч точный или по суффиксу
/// ".domain"). Лучше перебдеть: лишний домен через upstream — копейки, а
/// пропущенный — обрыв доступа из-за гео-блока.
const DEFAULT_PROXY_DOMAINS: &[&str] = &[
    // Anthropic / Claude
    "anthropic.com",
    "claude.ai",
    "claude.com",
    // OpenAI / ChatGPT / Codex
    "openai.com",
    "chatgpt.com",
    "oaistatic.com",
    "oaiusercontent.com",
    // Google AI (только AI-endpoint'ы, НЕ все googleapis.com)
    "generativelanguage.googleapis.com",
    "ai.google.dev",
    // Прочие частые AI-API
    "x.ai",
    "mistral.ai",
    "openrouter.ai",
    "perplexity.ai",
    "githubcopilot.com",
    "cursor.sh",
    "cursor.com",
];

// ─── Конфигурация ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Всё через upstream (поведение старого моста).
    All,
    /// Через upstream только AI-домены, остальное DIRECT.
    Smart,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::All => "all",
            Mode::Smart => "smart",
        }
    }
}

/// Свойство входного сокета, а не rules-файла: смена пользовательского режима
/// или списка исключений не может ослабить выделенный вход Claude.
#[derive(Clone, Copy, PartialEq)]
enum RoutingPolicy {
    Rules,
    UpstreamOnly,
}

impl RoutingPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::UpstreamOnly => "upstream_only",
        }
    }
}

/// JSON rules-файла (`bridge-rules.json`). Все поля опциональны — отсутствие
/// файла или поля означает дефолты (mode=all, встроенный список доменов).
#[derive(Deserialize, Default)]
struct RulesFile {
    mode: Option<String>,
    /// ДОПОЛНИТЕЛЬНЫЕ домены через upstream (расширяют встроенный список).
    proxy_domains: Option<Vec<String>>,
    /// Домены всегда напрямую (выигрывают у proxy_domains в любом режиме).
    direct_domains: Option<Vec<String>>,
    /// Домен → индекс upstream'а (порядок --upstream в argv): «Claude через
    /// прокси №0, ChatGPT через №1». Матч по суффиксу, самый длинный
    /// выигрывает. Индекс за пределами пула игнорируется.
    ///
    /// Назначение — СТРОГОЕ закрепление: используется ТОЛЬКО назначенный
    /// прокси, без failover на другие. Смена выходного IP опаснее короткого
    /// простоя: AI-сервисы (особенно Anthropic) флагают аккаунты за скачки
    /// IP. Failover работает только для доменов без назначения («Авто»).
    assignments: Option<std::collections::HashMap<String, usize>>,
}

struct Upstream {
    scheme: String,
    host: String,
    port: u16,
    /// base64(login:pass) для Proxy-Authorization, если в URL был auth.
    auth: Option<String>,
    /// URL с замаскированным паролем — для логов и /healthz.
    masked: String,
    ok: AtomicU64,
    fail: AtomicU64,
    /// Отлёжка после фейла коннекта: до этого момента upstream пропускается
    /// (passive failover). None — здоров.
    bad_until: Mutex<Option<Instant>>,
}

impl Upstream {
    fn cooling(&self) -> bool {
        self.bad_until
            .lock()
            .map(|g| g.map(|t| t > Instant::now()).unwrap_or(false))
            .unwrap_or(false)
    }
    fn mark_ok(&self) {
        self.ok.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = self.bad_until.lock() {
            *g = None;
        }
    }
    fn mark_fail(&self) {
        self.fail.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = self.bad_until.lock() {
            *g = Some(Instant::now() + UPSTREAM_COOLDOWN);
        }
    }
}

struct State {
    started: Instant,
    listen: String,
    quiet: bool,
    mode: Mode,
    routing_policy: RoutingPolicy,
    proxy_domains: Vec<String>,
    direct_domains: Vec<String>,
    /// (домен, индекс upstream'а) — привязка сервисов к конкретным прокси.
    assignments: Vec<(String, usize)>,
    upstreams: Arc<Vec<Upstream>>,
    total: AtomicU64,
    active: AtomicU64,
    via_upstream: AtomicU64,
    via_direct: AtomicU64,
    errors: AtomicU64,
    /// Последние ошибки (секунды-от-старта, текст) — отдаются в /healthz,
    /// чтобы Dashboard мог показать пользователю ЧТО именно ломалось
    /// (супервизор запускает мост скрыто, stderr некуда смотреть).
    last_errors: Mutex<std::collections::VecDeque<(u64, String)>>,
}

/// Сколько последних ошибок держим для /healthz.
const LAST_ERRORS_CAP: usize = 20;

impl State {
    fn strict_ready(&self) -> bool {
        self.routing_policy == RoutingPolicy::UpstreamOnly
            // В мосте пока нет TLS до самого прокси. Не посылаем учётные
            // данные открытым текстом на якобы HTTPS endpoint.
            && matches!(strict_upstream(self), Pin::Pinned(i) if self.upstreams[i].scheme == "http")
    }

    fn log(&self, msg: &str) {
        if !self.quiet {
            eprintln!("[{:>8.1}s] {msg}", self.started.elapsed().as_secs_f64());
        }
    }
    /// Ошибки видны и в quiet-режиме + попадают в кольцевой буфер /healthz.
    fn log_err(&self, msg: &str) {
        eprintln!(
            "[{:>8.1}s] ERROR {msg}",
            self.started.elapsed().as_secs_f64()
        );
        if let Ok(mut g) = self.last_errors.lock() {
            if g.len() >= LAST_ERRORS_CAP {
                g.pop_front();
            }
            g.push_back((self.started.elapsed().as_secs(), msg.to_string()));
        }
    }
}

/// http://login:pass@ip:port → Upstream. Формат тот же, что у python-моста.
fn parse_upstream(raw: &str) -> Result<Upstream, String> {
    let u = url::Url::parse(raw).map_err(|e| format!("bad upstream URL: {e}"))?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err(format!(
            "upstream scheme must be http/https, got {}",
            u.scheme()
        ));
    }
    let host = u
        .host_str()
        .ok_or("upstream URL must include host")?
        .to_string();
    let port = u.port().ok_or("upstream URL must include port")?;
    let (auth, masked) = if !u.username().is_empty() {
        let creds = format!("{}:{}", u.username(), u.password().unwrap_or(""));
        (
            Some(base64::engine::general_purpose::STANDARD.encode(creds)),
            format!("{}://{}:{}", u.scheme(), host, port),
        )
    } else {
        (None, format!("{}://{}:{}", u.scheme(), host, port))
    };
    Ok(Upstream {
        scheme: u.scheme().to_string(),
        host,
        port,
        auth,
        masked,
        ok: AtomicU64::new(0),
        fail: AtomicU64::new(0),
        bad_until: Mutex::new(None),
    })
}

fn load_rules(path: Option<&str>) -> (Mode, Vec<String>, Vec<String>, Vec<(String, usize)>) {
    let mut mode = Mode::All;
    let mut proxy: Vec<String> = DEFAULT_PROXY_DOMAINS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut direct: Vec<String> = Vec::new();
    let mut assignments: Vec<(String, usize)> = Vec::new();
    let Some(p) = path else {
        return (mode, proxy, direct, assignments);
    };

    // Отсутствие файла — легальный кейс (правил нет). А вот битый/нечитаемый
    // файл дефолтами закрывать НЕЛЬЗЯ: дефолты = пустые assignments, т.е.
    // закреплённый Claude тихо уйдёт в failover на другой выходной IP. Смена
    // IP опаснее простоя, поэтому: ретраи (запись дашборда могла совпасть с
    // нашим стартом), затем fail-fast — супервизор перезапустит с целым файлом.
    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        let text = match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (mode, proxy, direct, assignments);
            }
            Err(e) => {
                last_err = format!("не читается: {e}");
                continue;
            }
        };
        // BOM терпим: Блокнот/PowerShell пишут UTF-8 c BOM, serde_json его не ест.
        match serde_json::from_str::<RulesFile>(text.trim_start_matches('\u{feff}')) {
            Ok(r) => {
                if let Some(m) = r.mode.as_deref() {
                    mode = if m.eq_ignore_ascii_case("smart") {
                        Mode::Smart
                    } else {
                        Mode::All
                    };
                }
                proxy.extend(r.proxy_domains.unwrap_or_default());
                direct.extend(r.direct_domains.unwrap_or_default());
                // Нормализация + детерминированный порядок (длина ↓, имя, индекс):
                // HashMap-порядок случаен, и при дублях-тезках победитель менялся
                // бы от рестарта к рестарту — а это скачок выходного IP.
                let mut v: Vec<(String, usize)> = r
                    .assignments
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(k, i)| (k.trim_end_matches('.').to_ascii_lowercase(), i))
                    .collect();
                v.sort_by(|a, b| {
                    b.0.len()
                        .cmp(&a.0.len())
                        .then_with(|| a.0.cmp(&b.0))
                        .then_with(|| a.1.cmp(&b.1))
                });
                v.dedup_by(|a, b| a.0 == b.0);
                assignments = v;
                return (mode, proxy, direct, assignments);
            }
            Err(e) => last_err = format!("не парсится: {e}"),
        }
    }
    eprintln!(
        "ERROR: rules file {p} {last_err} — отказ запуска: потеря закреплений опаснее простоя"
    );
    std::process::exit(2);
}

// ─── Маршрутизация ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Route {
    Direct,
    Upstream,
}

/// Матч по суффиксу: "api.anthropic.com" матчится на "anthropic.com".
/// Хвостовая точка FQDN ("claude.ai.") нормализуется — иначе канонические
/// имена обходили бы и роутинг, и закрепление.
fn host_matches(host: &str, domain: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let d = domain.trim_end_matches('.').to_ascii_lowercase();
    h == d || h.ends_with(&format!(".{d}"))
}

fn decide(state: &State, host: &str) -> Route {
    if state.routing_policy == RoutingPolicy::UpstreamOnly {
        return Route::Upstream;
    }
    if state.direct_domains.iter().any(|d| host_matches(host, d)) {
        return Route::Direct;
    }
    match state.mode {
        Mode::All => Route::Upstream,
        Mode::Smart => {
            if state.proxy_domains.iter().any(|d| host_matches(host, d)) {
                Route::Upstream
            } else {
                Route::Direct
            }
        }
    }
}

// ─── Сеть: помощники ─────────────────────────────────────────────

/// TCP keepalive: NAT/файрвол между нами и upstream не должен тихо убивать
/// idle-соединение посреди долгого SSE-стрима Claude Code (та же логика,
/// что была в python-мосте).
fn enable_keepalive(stream: &TcpStream) {
    use socket2::{SockRef, TcpKeepalive};
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10));
    let _ = SockRef::from(stream).set_tcp_keepalive(&ka);
    let _ = stream.set_nodelay(true);
}

/// Прочитать заголовки первого запроса (до \r\n\r\n, лимит HEAD_LIMIT).
/// Дедлайн один на ВСЮ голову: per-read таймаут позволял клиенту, капающему
/// по байту, держать permit семафора бесконечно.
async fn read_head(client: &mut TcpStream) -> Result<Vec<u8>, String> {
    let read_all = async {
        let mut buf = Vec::with_capacity(2048);
        let mut chunk = [0u8; 8192];
        loop {
            let n = client
                .read(&mut chunk)
                .await
                .map_err(|e| format!("head read: {e}"))?;
            if n == 0 {
                return Err("client closed before full head".to_string());
            }
            buf.extend_from_slice(&chunk[..n]);
            if find_head_end(&buf).is_some() {
                return Ok(buf);
            }
            if buf.len() > HEAD_LIMIT {
                return Err("headers too large".to_string());
            }
        }
    };
    timeout(HEAD_TIMEOUT, read_all)
        .await
        .map_err(|_| "head read timeout".to_string())?
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Убрать hop-by-hop proxy-заголовки; опционально добавить Proxy-Authorization.
/// Работает и для CONNECT, и для обычного запроса — правится только блок
/// заголовков до \r\n\r\n, остальные байты (начало тела) не трогаются.
///
/// `force_close`: мы форвардим ТОЛЬКО первый запрос соединения, дальше идёт
/// слепой pipe — keep-alive для plain-HTTP через мост опасен (второй запрос
/// ушёл бы без auth и/или на сокет первого хоста). Поэтому не-CONNECT
/// запросы принудительно закрываются: клиент откроет новое соединение.
/// HTTPS (CONNECT-туннели) это не касается.
fn rewrite_head(
    head: &[u8],
    auth: Option<&str>,
    strip_proxy_headers: bool,
    force_close: bool,
) -> Vec<u8> {
    let end = match find_head_end(head) {
        Some(i) => i,
        None => return head.to_vec(),
    };
    let (lines_part, rest) = head.split_at(end);
    let mut out: Vec<&[u8]> = Vec::new();
    for line in lines_part.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let lower: Vec<u8> = line.iter().map(|b| b.to_ascii_lowercase()).collect();
        // Наш Proxy-Authorization всегда заменяет клиентский; Proxy-Connection
        // убираем при DIRECT (origin-серверу он не нужен и вреден).
        if lower.starts_with(b"proxy-authorization:") {
            continue;
        }
        if strip_proxy_headers && lower.starts_with(b"proxy-connection:") {
            continue;
        }
        if force_close
            && (lower.starts_with(b"connection:") || lower.starts_with(b"proxy-connection:"))
        {
            continue;
        }
        out.push(line);
    }
    let mut result = Vec::with_capacity(head.len() + 64);
    for (i, line) in out.iter().enumerate() {
        if i > 0 {
            result.extend_from_slice(b"\r\n");
        }
        result.extend_from_slice(line);
    }
    if let Some(a) = auth {
        result.extend_from_slice(format!("\r\nProxy-Authorization: Basic {a}").as_bytes());
    }
    if force_close {
        result.extend_from_slice(b"\r\nConnection: close");
    }
    result.extend_from_slice(rest); // rest начинается с \r\n\r\n
    result
}

/// Результат поиска назначения для хоста.
#[derive(Clone, Copy, PartialEq)]
enum Pin {
    /// Назначения нет — обычный маршрут и failover.
    None,
    /// Закреплён за живым индексом пула.
    Pinned(usize),
    /// Назначение есть, но индекс за пределами пула (rules и argv разошлись).
    /// По семантике закрепления это жёсткий отказ, а НЕ failover: уйти на
    /// другой прокси = сменить выходной IP сервиса.
    Broken,
}

/// Назначенный прокси для хоста: самый длинный суффикс-матч из assignments.
fn preferred_upstream(state: &State, host: &str) -> Pin {
    let mut best: Option<(usize, usize)> = None; // (длина домена, индекс)
    for (domain, idx) in &state.assignments {
        if host_matches(host, domain) && best.map(|(l, _)| domain.len() > l).unwrap_or(true) {
            best = Some((domain.len(), *idx));
        }
    }
    match best {
        None => Pin::None,
        Some((_, i)) if i < state.upstreams.len() => Pin::Pinned(i),
        Some(_) => Pin::Broken,
    }
}

/// The dedicated Claude listener uses one identity for every destination.
/// Missing assignments pin the primary proxy; conflicting assignments fail closed.
fn strict_upstream(state: &State) -> Pin {
    let mut selected = None;
    for host in ["claude.ai", "claude.com", "anthropic.com"] {
        match preferred_upstream(state, host) {
            Pin::Broken => return Pin::Broken,
            Pin::Pinned(i) => {
                if selected.is_some_and(|previous| previous != i) {
                    return Pin::Broken;
                }
                selected = Some(i);
            }
            Pin::None => {}
        }
    }
    match selected {
        Some(i) => Pin::Pinned(i),
        None if !state.upstreams.is_empty() => Pin::Pinned(0),
        None => Pin::Broken,
    }
}

fn request_upstream(state: &State, host: &str) -> Pin {
    if state.routing_policy == RoutingPolicy::UpstreamOnly {
        strict_upstream(state)
    } else {
        preferred_upstream(state, host)
    }
}

/// Одна попытка коннекта к upstream'у №i с учётом статистики и cooldown.
async fn try_upstream(state: &State, i: usize, pinned: bool) -> Option<TcpStream> {
    let u = &state.upstreams[i];
    match timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((u.host.as_str(), u.port)),
    )
    .await
    {
        Ok(Ok(s)) => {
            u.mark_ok();
            Some(s)
        }
        _ => {
            u.mark_fail();
            state.log_err(&if pinned {
                format!(
                    "назначенный прокси {} не отвечает — жду восстановления, IP не меняю",
                    u.masked
                )
            } else {
                format!("прокси {} не отвечает — пробую следующий", u.masked)
            });
            None
        }
    }
}

/// Выбрать живой upstream и подключиться.
/// `pinned` (назначенный домену прокси) — СТРОГО: пробуем только его, на
/// другие НЕ уходим — выходной IP сервиса не должен прыгать.
/// Без назначения — passive failover: первый проход пропускает остывающих,
/// второй шанс даётся ТОЛЬКО пропущенным (повторять только что упавших —
/// удвоение таймаута для клиента без шанса на успех).
async fn connect_upstream(state: &State, pinned: Option<usize>) -> Option<(TcpStream, usize)> {
    let n = state.upstreams.len();
    let (order, is_pin): (Vec<usize>, bool) = match pinned {
        Some(p) if p < n => (vec![p], true),
        Some(_) => return None, // битый индекс — сюда не должен дойти (Pin::Broken отсекается раньше)
        None => ((0..n).collect(), false),
    };
    let mut skipped: Vec<usize> = Vec::new();
    for &i in &order {
        if state.upstreams[i].cooling() {
            skipped.push(i);
            continue;
        }
        if let Some(s) = try_upstream(state, i, is_pin).await {
            return Some((s, i));
        }
    }
    for &i in &skipped {
        if let Some(s) = try_upstream(state, i, is_pin).await {
            return Some((s, i));
        }
    }
    None
}

// ─── /healthz ────────────────────────────────────────────────────

fn healthz_json(state: &State) -> String {
    // serde_json вместо ручной сборки: в last_errors живой текст (имена хостов,
    // ошибки ОС) — экранирование должно быть железным.
    let ups: Vec<serde_json::Value> = state
        .upstreams
        .iter()
        .map(|u| {
            serde_json::json!({
                "url": u.masked,
                "healthy": !u.cooling(),
                "ok": u.ok.load(Ordering::Relaxed),
                "fail": u.fail.load(Ordering::Relaxed),
            })
        })
        .collect();
    let errors: Vec<serde_json::Value> = state
        .last_errors
        .lock()
        .map(|g| {
            g.iter()
                .map(|(t, msg)| serde_json::json!({ "t": t, "msg": msg }))
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "status": if state.routing_policy == RoutingPolicy::UpstreamOnly && !state.strict_ready() { "error" } else { "ok" },
        "service": "clod-bridge",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_sec": state.started.elapsed().as_secs(),
        "listen": state.listen,
        "port": state.listen.parse::<SocketAddr>().ok().map(|addr| addr.port()),
        "mode": if state.routing_policy == RoutingPolicy::UpstreamOnly { "all" } else { state.mode.as_str() },
        "routing_policy": state.routing_policy.as_str(),
        "direct_allowed": state.routing_policy != RoutingPolicy::UpstreamOnly,
        "strict_ready": state.strict_ready(),
        "strict_upstream_index": if state.routing_policy == RoutingPolicy::UpstreamOnly {
            match strict_upstream(state) { Pin::Pinned(i) => Some(i), _ => None }
        } else { None },
        "strict_failover_allowed": false,
        "active": state.active.load(Ordering::Relaxed),
        "total": state.total.load(Ordering::Relaxed),
        "via_upstream": state.via_upstream.load(Ordering::Relaxed),
        "via_direct": state.via_direct.load(Ordering::Relaxed),
        "errors": state.errors.load(Ordering::Relaxed),
        "upstreams": ups,
        "last_errors": errors,
    })
    .to_string()
}

async fn serve_local(client: &mut TcpStream, path: &str, state: &State) {
    let (code, body) = if path == "/healthz" || path == "/stats" {
        let code = if state.routing_policy == RoutingPolicy::UpstreamOnly && !state.strict_ready() {
            "503 Service Unavailable"
        } else {
            "200 OK"
        };
        (code, healthz_json(state))
    } else {
        ("404 Not Found", "{\"error\":\"not found\"}".to_string())
    };
    let resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    let _ = client.write_all(resp.as_bytes()).await;
}

// ─── Обработка соединения ────────────────────────────────────────

struct Parsed {
    is_connect: bool,
    /// origin-form запрос к самому мосту (например GET /healthz).
    local_path: Option<String>,
    host: String,
    port: u16,
}

fn parse_request(head: &[u8]) -> Result<Parsed, String> {
    let first_line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .ok_or("no request line")?;
    let line = String::from_utf8_lossy(&head[..first_line_end]);
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or("empty request line")?.to_string();
    let target = parts.next().ok_or("no target in request line")?.to_string();

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port_s) = target.rsplit_once(':').ok_or("CONNECT without port")?;
        let port: u16 = port_s.parse().map_err(|_| "bad CONNECT port")?;
        // IPv6-литерал приходит в скобках ("[::1]:443") — для connect() их надо снять.
        let host = host.trim_start_matches('[').trim_end_matches(']');
        return Ok(Parsed {
            is_connect: true,
            local_path: None,
            host: host.to_string(),
            port,
        });
    }
    if target.starts_with('/') {
        // origin-form → запрос к самому мосту (healthz).
        return Ok(Parsed {
            is_connect: false,
            local_path: Some(target),
            host: String::new(),
            port: 0,
        });
    }
    // absolute-form: GET http://host:port/path
    let u = url::Url::parse(&target).map_err(|e| format!("bad absolute-URI: {e}"))?;
    // host_str() для IPv6 отдаёт хост СО скобками — для connect() их надо снять.
    let host = u
        .host_str()
        .ok_or("absolute-URI without host")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = u.port_or_known_default().unwrap_or(80);
    Ok(Parsed {
        is_connect: false,
        local_path: None,
        host,
        port,
    })
}

/// RAII-декремент active-счётчика.
struct ActiveGuard<'a>(&'a State);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle(mut client: TcpStream, state: Arc<State>) {
    let head = match read_head(&mut client).await {
        Ok(h) => h,
        Err(e) => {
            state.log(&format!("bad head: {e}"));
            return;
        }
    };
    let req = match parse_request(&head) {
        Ok(r) => r,
        Err(e) => {
            state.log(&format!("bad request: {e}"));
            let _ = client
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
    };

    // Запрос к самому мосту (healthz/stats) — метрики НЕ трогаем: поллинг
    // дашборда раз в 5с иначе рисует тысячи фантомных «соединений» за ночь.
    if let Some(path) = req.local_path {
        serve_local(&mut client, &path, &state).await;
        return;
    }

    state.total.fetch_add(1, Ordering::Relaxed);
    state.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(&state);

    if state.routing_policy == RoutingPolicy::UpstreamOnly && !state.strict_ready() {
        state.errors.fetch_add(1, Ordering::Relaxed);
        state.log_err("строгий вход требует хотя бы один HTTP upstream; HTTPS upstream пока не поддерживается");
        let _ = client
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    }

    // Назначение сильнее режима: закреплённый домен идёт через свой upstream,
    // даже если smart-режим отправил бы его напрямую (или домен попал в
    // direct_domains) — иначе он утёк бы с реальным IP пользователя.
    let pin = request_upstream(&state, &req.host);
    let route = match pin {
        Pin::Pinned(_) => Route::Upstream,
        Pin::Broken => {
            state.errors.fetch_add(1, Ordering::Relaxed);
            state.log_err(&format!(
                "назначение для {} указывает на прокси вне пула — отказываю (IP менять нельзя)",
                req.host
            ));
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
        Pin::None => decide(&state, &req.host),
    };

    // Устанавливаем исходящее соединение. Для Upstream запоминаем auth ИМЕННО
    // того прокси, к которому реально подключились (failover мог выбрать не первый).
    // via_* считаем только по факту успешного коннекта — неудачи идут в errors.
    let (mut server, upstream_auth): (TcpStream, Option<String>) = match route {
        Route::Direct => {
            match timeout(
                CONNECT_TIMEOUT,
                TcpStream::connect((req.host.as_str(), req.port)),
            )
            .await
            {
                Ok(Ok(s)) => {
                    state.via_direct.fetch_add(1, Ordering::Relaxed);
                    (s, None)
                }
                _ => {
                    state.errors.fetch_add(1, Ordering::Relaxed);
                    state.log_err(&format!(
                        "прямое соединение с {}:{} не удалось",
                        req.host, req.port
                    ));
                    let _ = client
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    return;
                }
            }
        }
        Route::Upstream => {
            let pinned = match pin {
                Pin::Pinned(i) => Some(i),
                _ => None,
            };
            match connect_upstream(&state, pinned).await {
                Some((s, i)) => {
                    state.via_upstream.fetch_add(1, Ordering::Relaxed);
                    let auth = state.upstreams[i].auth.clone();
                    (s, auth)
                }
                None => {
                    state.errors.fetch_add(1, Ordering::Relaxed);
                    state.log_err(if pinned.is_some() {
                        "назначенный прокси недоступен — жду восстановления"
                    } else {
                        "все upstream-прокси недоступны"
                    });
                    let _ = client
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                        .await;
                    return;
                }
            }
        }
    };

    enable_keepalive(&client);
    enable_keepalive(&server);

    match route {
        Route::Direct => {
            if req.is_connect {
                // Туннель установлен нами — отвечаем клиенту сами.
                if client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                // Клиент мог оптимистично прислать TLS ClientHello одним write
                // с CONNECT — хвост за \r\n\r\n обязан уйти серверу, иначе
                // TLS-handshake зависнет до таймаута.
                if let Some(end) = find_head_end(&head) {
                    let tail = &head[end + 4..];
                    if !tail.is_empty() && server.write_all(tail).await.is_err() {
                        return;
                    }
                }
            } else {
                // Обычный HTTP напрямую: чистим proxy-заголовки, форвардим.
                let clean = rewrite_head(&head, None, true, true);
                if server.write_all(&clean).await.is_err() {
                    return;
                }
            }
        }
        Route::Upstream => {
            // Форвардим первый запрос (CONNECT или обычный) upstream'у,
            // впрыснув auth. Ответ (включая "200 Connection Established")
            // придёт клиенту от самого upstream'а через pipe ниже.
            let rewritten = rewrite_head(&head, upstream_auth.as_deref(), false, !req.is_connect);
            if server.write_all(&rewritten).await.is_err() {
                state.errors.fetch_add(1, Ordering::Relaxed);
                state.log_err(&format!("upstream оборвал соединение ({})", req.host));
                return;
            }
        }
    }

    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}

// ─── main ────────────────────────────────────────────────────────

struct Args {
    upstreams: Vec<String>,
    listen: String,
    strict_listen: String,
    quiet: bool,
    rules: Option<String>,
    /// Режим супервизора: перезапускать себя-воркера при падении.
    supervise: bool,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1).collect())
}

fn parse_args_from(argv: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        upstreams: Vec::new(),
        listen: "127.0.0.1:8888".into(),
        strict_listen: "127.0.0.1:8890".into(),
        quiet: false,
        rules: None,
        supervise: false,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--upstream" => {
                i += 1;
                a.upstreams
                    .push(argv.get(i).ok_or("--upstream requires URL")?.clone());
            }
            "--listen" => {
                i += 1;
                a.listen = argv.get(i).ok_or("--listen requires HOST:PORT")?.clone();
            }
            "--strict-listen" => {
                i += 1;
                a.strict_listen = argv
                    .get(i)
                    .ok_or("--strict-listen requires IP:PORT")?
                    .clone();
            }
            "--rules" => {
                i += 1;
                a.rules = Some(argv.get(i).ok_or("--rules requires FILE")?.clone());
            }
            "--quiet" => a.quiet = true,
            "--supervise" => a.supervise = true,
            "--version" => {
                println!("clod-bridge {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
        i += 1;
    }
    if a.upstreams.is_empty() {
        return Err("--upstream is required".into());
    }
    let strict_addr = a
        .strict_listen
        .parse::<SocketAddr>()
        .map_err(|_| "--strict-listen requires a literal loopback IP and port")?;
    if !strict_addr.ip().is_loopback() || strict_addr.port() == 0 {
        return Err("--strict-listen must use a loopback IP and a non-zero port".into());
    }
    Ok(a)
}

/// Цикл самосупервизии: спавним себя-воркера (те же argv минус --supervise),
/// ждём завершения, через 5с поднимаем заново. Замена VBS/wscript-циклу.
/// Один супервизор на exe — эксклюзивный lock-файл (share_mode 0).
fn run_supervisor() -> ! {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: current_exe: {e}");
            std::process::exit(1);
        }
    };
    #[cfg(target_os = "windows")]
    let _lock = {
        use std::os::windows::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .share_mode(0)
            .open(exe.with_extension("supervisor.lock"))
        {
            Ok(f) => f,
            // Лок занят → супервизор уже работает (задача + watchdog GUI
            // могли стартовать одновременно) — тихо уходим, дубли не плодим.
            Err(_) => std::process::exit(0),
        }
    };
    let child_args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--supervise")
        .collect();
    loop {
        match std::process::Command::new(&exe).args(&child_args).status() {
            Ok(s) => eprintln!("supervisor: мост завершился ({s}) — перезапуск через 5с"),
            Err(e) => eprintln!("supervisor: не удалось запустить воркер: {e}"),
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: {e}");
            eprintln!(
                "usage: clod-bridge --upstream http://user:pass@host:port [--upstream …] \
                 [--listen 127.0.0.1:8888] [--strict-listen 127.0.0.1:8890] \
                 [--rules FILE] [--quiet] [--supervise]"
            );
            std::process::exit(1);
        }
    };

    if args.supervise {
        run_supervisor();
    }

    let mut upstreams = Vec::new();
    for raw in &args.upstreams {
        match parse_upstream(raw) {
            Ok(u) => upstreams.push(u),
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        }
    }

    let (mode, proxy_domains, direct_domains, assignments) = load_rules(args.rules.as_deref());

    let state = Arc::new(State {
        started: Instant::now(),
        listen: args.listen.clone(),
        quiet: args.quiet,
        mode,
        routing_policy: RoutingPolicy::Rules,
        proxy_domains,
        direct_domains,
        assignments,
        upstreams: Arc::new(upstreams),
        total: AtomicU64::new(0),
        active: AtomicU64::new(0),
        via_upstream: AtomicU64::new(0),
        via_direct: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        last_errors: Mutex::new(std::collections::VecDeque::new()),
    });

    let strict_state = Arc::new(State {
        started: state.started,
        listen: args.strict_listen.clone(),
        quiet: args.quiet,
        mode,
        routing_policy: RoutingPolicy::UpstreamOnly,
        proxy_domains: state.proxy_domains.clone(),
        direct_domains: state.direct_domains.clone(),
        assignments: state.assignments.clone(),
        upstreams: state.upstreams.clone(),
        total: AtomicU64::new(0),
        active: AtomicU64::new(0),
        via_upstream: AtomicU64::new(0),
        via_direct: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        last_errors: Mutex::new(std::collections::VecDeque::new()),
    });

    // Коллизия строгого порта запрещает запуск Claude, но не должна
    // останавливать сеть остальных приложений на обычном входе.
    // Проверка Claude требует health именно строгого порта.
    let (listener, strict_listener) = match bind_listeners(&args.listen, &args.strict_listen).await
    {
        Ok(listeners) => listeners,
        Err(e) => {
            eprintln!("ERROR: {e}");
            eprintln!("Is another process already listening on this port?");
            std::process::exit(1);
        }
    };

    {
        let ups: Vec<&str> = state.upstreams.iter().map(|u| u.masked.as_str()).collect();
        state.log(&format!(
            "clod-bridge {} listening on {} → {} | mode={}",
            env!("CARGO_PKG_VERSION"),
            args.listen,
            ups.join(", "),
            state.mode.as_str(),
        ));
    }

    // Семафор — как в python-мосте: лишние соединения ждут слот, а не валят
    // процесс. Accept-цикл НИКОГДА не падает.
    let sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    match strict_listener {
        Ok(strict_listener) => {
            strict_state.log(&format!(
                "Claude upstream-only listener on {} | ready={}",
                args.strict_listen,
                strict_state.strict_ready(),
            ));
            tokio::join!(
                serve_listener(listener, state, sem.clone()),
                serve_listener(strict_listener, strict_state, sem),
            );
        }
        Err(error) => {
            state.log_err(&format!(
                "{error}; Claude strict listener unavailable; shared listener remains active"
            ));
            serve_listener(listener, state, sem).await;
        }
    }
}

async fn bind_listeners(
    listen: &str,
    strict_listen: &str,
) -> Result<(TcpListener, Result<TcpListener, String>), String> {
    let regular = TcpListener::bind(listen)
        .await
        .map_err(|e| format!("cannot bind {listen} — {e}"))?;
    let strict = TcpListener::bind(strict_listen)
        .await
        .map_err(|e| format!("cannot bind strict listener {strict_listen} — {e}"));
    Ok((regular, strict))
}

async fn serve_listener(listener: TcpListener, state: Arc<State>, sem: Arc<Semaphore>) {
    loop {
        let (client, addr) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                state.log_err(&format!("accept: {e} — продолжаю слушать"));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let permit = match timeout(Duration::from_secs(5), sem.clone().acquire_owned()).await {
            Ok(Ok(p)) => p,
            _ => {
                state.log_err(&format!(
                    "лимит {MAX_CONNECTIONS} соединений — отбиваю {addr}, сервер жив"
                ));
                drop(client);
                continue;
            }
        };
        let st = state.clone();
        tokio::spawn(async move {
            handle(client, st).await;
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(policy: RoutingPolicy, upstreams: Vec<Upstream>) -> State {
        State {
            started: Instant::now(),
            listen: "127.0.0.1:8890".into(),
            quiet: true,
            mode: Mode::Smart,
            routing_policy: policy,
            proxy_domains: vec!["anthropic.com".into()],
            direct_domains: vec!["127.0.0.1".into(), "example.invalid".into()],
            assignments: Vec::new(),
            upstreams: Arc::new(upstreams),
            total: AtomicU64::new(0),
            active: AtomicU64::new(0),
            via_upstream: AtomicU64::new(0),
            via_direct: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_errors: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    async fn single_request(state: Arc<State>, request: String) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            handle(client, state).await;
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        timeout(Duration::from_secs(3), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        response
    }

    async fn assert_no_connection(listener: &TcpListener) {
        assert!(
            timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "destination was contacted directly"
        );
    }

    #[test]
    fn rules_cannot_downgrade_strict_listener() {
        let mut state = test_state(RoutingPolicy::UpstreamOnly, Vec::new());
        for mode in [Mode::Smart, Mode::All] {
            state.mode = mode;
            for host in [
                "example.invalid",
                "unlisted.invalid",
                "api.anthropic.com",
                "127.0.0.1",
                "::1",
            ] {
                assert!(decide(&state, host) == Route::Upstream);
            }
        }
        state.routing_policy = RoutingPolicy::Rules;
        assert!(decide(&state, "example.invalid") == Route::Direct);
        state.mode = Mode::Smart;
        assert!(decide(&state, "unlisted.invalid") == Route::Direct);
        assert!(decide(&state, "api.anthropic.com") == Route::Upstream);
    }

    #[test]
    fn reloaded_json_rules_cannot_downgrade_strict_listener() {
        let path = std::env::temp_dir().join(format!(
            "clod-bridge-rules-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut state = test_state(RoutingPolicy::UpstreamOnly, Vec::new());
        for rules in [
            r#"{"mode":"all"}"#,
            r#"{"mode":"smart","direct_domains":["example.invalid"],"routing_policy":"rules"}"#,
        ] {
            std::fs::write(&path, rules).unwrap();
            let (mode, proxy, direct, assignments) = load_rules(path.to_str());
            state.mode = mode;
            state.proxy_domains = proxy;
            state.direct_domains = direct;
            state.assignments = assignments;
            assert!(decide(&state, "example.invalid") == Route::Upstream);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn shared_smart_listener_keeps_existing_direct_behavior() {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = origin.local_addr().unwrap();
        let origin_task = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\ndirect")
                .await
                .unwrap();
        });
        let state = Arc::new(test_state(RoutingPolicy::Rules, Vec::new()));
        let response = single_request(
            state.clone(),
            format!("GET http://{target}/ HTTP/1.1\r\n\r\n"),
        )
        .await;
        assert!(response.ends_with(b"direct"));
        assert_eq!(state.via_direct.load(Ordering::Relaxed), 1);
        assert_eq!(state.via_upstream.load(Ordering::Relaxed), 0);
        origin_task.await.unwrap();
    }

    #[tokio::test]
    async fn connect_and_http_always_reach_upstream_even_for_direct_exclusion() {
        for is_connect in [true, false] {
            let other_proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target = origin.local_addr().unwrap();
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_addr = upstream.local_addr().unwrap();
            let upstream_task = tokio::spawn(async move {
                let (mut stream, _) = upstream.accept().await.unwrap();
                let mut request = Vec::new();
                stream.read_to_end(&mut request).await.unwrap();
                let response = if is_connect {
                    b"HTTP/1.1 200 Connection Established\r\n\r\nupstream tunnel".as_slice()
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nupstream http".as_slice()
                };
                stream.write_all(response).await.unwrap();
                request
            });
            let mut state = test_state(
                RoutingPolicy::UpstreamOnly,
                vec![
                    parse_upstream(&format!("http://{}", other_proxy.local_addr().unwrap()))
                        .unwrap(),
                    parse_upstream(&format!("http://test-user:test-password@{upstream_addr}"))
                        .unwrap(),
                ],
            );
            state.assignments = vec![("claude.ai".into(), 1), ("127.0.0.1".into(), 0)];
            let state = Arc::new(state);
            let request = if is_connect {
                format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: Basic untrusted\r\n\r\ntunnel payload")
            } else {
                format!("GET http://{target}/healthz HTTP/1.1\r\nHost: {target}\r\n\r\n")
            };
            let response = single_request(state.clone(), request.clone()).await;
            assert!(String::from_utf8_lossy(&response).contains("upstream "));
            let forwarded = timeout(Duration::from_secs(3), upstream_task)
                .await
                .unwrap()
                .unwrap();
            let forwarded = String::from_utf8(forwarded).unwrap();
            assert!(forwarded.starts_with(request.lines().next().unwrap()));
            assert!(!forwarded.contains("untrusted"));
            assert!(forwarded.contains(&format!(
                "Proxy-Authorization: Basic {}",
                base64::engine::general_purpose::STANDARD.encode("test-user:test-password")
            )));
            if is_connect {
                assert!(forwarded.ends_with("tunnel payload"));
            } else {
                assert!(forwarded.contains("Connection: close"));
            }
            assert_eq!(state.via_upstream.load(Ordering::Relaxed), 1);
            assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
            assert_no_connection(&origin).await;
            assert_no_connection(&other_proxy).await;
        }
    }

    #[test]
    fn dedicated_pin_ignores_destination_and_rejects_conflicts() {
        let mut state = test_state(
            RoutingPolicy::UpstreamOnly,
            vec![
                parse_upstream("http://127.0.0.1:10001").unwrap(),
                parse_upstream("http://127.0.0.1:10002").unwrap(),
            ],
        );
        assert!(strict_upstream(&state) == Pin::Pinned(0));
        state.assignments = vec![("claude.ai".into(), 1), ("openai.com".into(), 0)];
        for host in [
            "claude.ai",
            "api.openai.com",
            "payment.invalid",
            "example.invalid",
        ] {
            assert!(request_upstream(&state, host) == Pin::Pinned(1));
        }
        state.routing_policy = RoutingPolicy::Rules;
        assert!(request_upstream(&state, "api.openai.com") == Pin::Pinned(0));
        assert!(request_upstream(&state, "payment.invalid") == Pin::None);
        state.routing_policy = RoutingPolicy::UpstreamOnly;
        state.assignments.push(("anthropic.com".into(), 0));
        assert!(!state.strict_ready());
        state.assignments = vec![("claude.ai".into(), 99)];
        assert!(!state.strict_ready());
    }

    #[tokio::test]
    async fn dedicated_pin_never_fails_over_to_other_proxy() {
        let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        for assigned in [true, false] {
            let alive = parse_upstream(&format!("http://{}", other.local_addr().unwrap())).unwrap();
            let dead = parse_upstream(&format!("http://{dead_addr}")).unwrap();
            let mut state = test_state(
                RoutingPolicy::UpstreamOnly,
                if assigned {
                    vec![alive, dead]
                } else {
                    vec![dead, alive]
                },
            );
            if assigned {
                state.assignments = vec![("claude.ai".into(), 1)];
            }
            let state = Arc::new(state);
            for request in [
                "CONNECT payment.invalid:443 HTTP/1.1\r\n\r\n",
                "GET http://payment.invalid/ HTTP/1.1\r\n\r\n",
            ] {
                let response = single_request(state.clone(), request.into()).await;
                assert!(response.starts_with(b"HTTP/1.1 502"));
                assert_no_connection(&other).await;
            }
            assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn dead_or_missing_upstream_never_falls_back_to_destination() {
        let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = unused.local_addr().unwrap();
        drop(unused);
        for upstreams in [
            Vec::new(),
            vec![parse_upstream(&format!("http://{dead_addr}")).unwrap()],
        ] {
            let state = Arc::new(test_state(RoutingPolicy::UpstreamOnly, upstreams));
            for is_connect in [true, false] {
                let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let target = origin.local_addr().unwrap();
                let request = if is_connect {
                    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n")
                } else {
                    format!("GET http://{target}/ HTTP/1.1\r\nHost: {target}\r\n\r\n")
                };
                let response = single_request(state.clone(), request).await;
                assert!(response.starts_with(b"HTTP/1.1 502"));
                assert_no_connection(&origin).await;
            }
            assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
            assert_eq!(state.errors.load(Ordering::Relaxed), 2);
        }
    }

    #[tokio::test]
    async fn https_upstream_is_rejected_before_sending_plaintext_credentials() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let state = Arc::new(test_state(
            RoutingPolicy::UpstreamOnly,
            vec![parse_upstream(&format!(
                "https://test-user:test-password@{}",
                upstream.local_addr().unwrap()
            ))
            .unwrap()],
        ));
        let response = single_request(
            state.clone(),
            "CONNECT example.invalid:443 HTTP/1.1\r\n\r\n".into(),
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 502"));
        assert_no_connection(&upstream).await;
        let health = single_request(state, "GET /healthz HTTP/1.1\r\n\r\n".into()).await;
        assert!(health.starts_with(b"HTTP/1.1 503"));
        assert!(String::from_utf8_lossy(&health).contains("\"strict_ready\":false"));
        assert!(!String::from_utf8_lossy(&health).contains("test-password"));
        assert!(!String::from_utf8_lossy(&health).contains("test-user"));
    }

    #[tokio::test]
    async fn local_health_identifies_listener_without_proxy_credentials() {
        let state = Arc::new(test_state(
            RoutingPolicy::UpstreamOnly,
            vec![parse_upstream("http://private-user:private-password@127.0.0.1:9999").unwrap()],
        ));
        let response = single_request(state.clone(), "GET /healthz HTTP/1.1\r\n\r\n".into()).await;
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let body_start = find_head_end(&response).unwrap() + 4;
        let health: serde_json::Value = serde_json::from_slice(&response[body_start..]).unwrap();
        assert_eq!(health["service"], "clod-bridge");
        assert_eq!(health["routing_policy"], "upstream_only");
        assert_eq!(health["direct_allowed"], false);
        assert_eq!(health["strict_ready"], true);
        assert_eq!(health["port"], 8890);
        assert_eq!(health["mode"], "all");
        assert_eq!(health["total"], 0);
        assert!(!String::from_utf8_lossy(&response).contains("private-user"));
        assert!(!String::from_utf8_lossy(&response).contains("private-password"));
    }

    #[tokio::test]
    async fn strict_port_collision_keeps_shared_listener_without_claiming_strict_protection() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (shared, strict) =
            bind_listeners("127.0.0.1:0", &occupied.local_addr().unwrap().to_string())
                .await
                .unwrap();
        assert!(strict.unwrap_err().contains("strict listener"));
        let addr = shared.local_addr().unwrap();
        let state = Arc::new(test_state(RoutingPolicy::Rules, Vec::new()));
        let server = tokio::spawn(async move {
            let (client, _) = shared.accept().await.unwrap();
            handle(client, state).await;
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET /healthz HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        timeout(Duration::from_secs(3), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let body_start = find_head_end(&response).unwrap() + 4;
        let health: serde_json::Value = serde_json::from_slice(&response[body_start..]).unwrap();
        assert_eq!(health["routing_policy"], "rules");
        assert_eq!(health["strict_ready"], false);
        assert_eq!(health["direct_allowed"], true);
        assert_no_connection(&occupied).await;
    }

    #[test]
    fn strict_endpoint_is_loopback_only_and_has_stable_default() {
        let args = |endpoint: Option<&str>| {
            let mut argv = vec!["--upstream".into(), "http://127.0.0.1:9999".into()];
            if let Some(endpoint) = endpoint {
                argv.extend(["--strict-listen".into(), endpoint.into()]);
            }
            parse_args_from(argv)
        };
        assert_eq!(args(None).unwrap().strict_listen, "127.0.0.1:8890");
        assert!(args(Some("[::1]:8890")).is_ok());
        for endpoint in [
            "0.0.0.0:8890",
            "192.0.2.1:8890",
            "[::]:8890",
            "localhost:8890",
            "127.0.0.1:0",
        ] {
            assert!(args(Some(endpoint)).is_err(), "accepted {endpoint}");
        }
    }
}
