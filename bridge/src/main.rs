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
    proxy_domains: Vec<String>,
    direct_domains: Vec<String>,
    /// (домен, индекс upstream'а) — привязка сервисов к конкретным прокси.
    assignments: Vec<(String, usize)>,
    upstreams: Vec<Upstream>,
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
    fn log(&self, msg: &str) {
        if !self.quiet {
            eprintln!("[{:>8.1}s] {msg}", self.started.elapsed().as_secs_f64());
        }
    }
    /// Ошибки видны и в quiet-режиме + попадают в кольцевой буфер /healthz.
    fn log_err(&self, msg: &str) {
        eprintln!("[{:>8.1}s] ERROR {msg}", self.started.elapsed().as_secs_f64());
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
    let u = url::Url::parse(raw).map_err(|e| format!("bad upstream URL {raw}: {e}"))?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err(format!("upstream scheme must be http/https, got {}", u.scheme()));
    }
    let host = u
        .host_str()
        .ok_or_else(|| format!("upstream URL must include host: {raw}"))?
        .to_string();
    let port = u
        .port()
        .ok_or_else(|| format!("upstream URL must include port: {raw}"))?;
    let (auth, masked) = if !u.username().is_empty() {
        let creds = format!("{}:{}", u.username(), u.password().unwrap_or(""));
        (
            Some(base64::engine::general_purpose::STANDARD.encode(creds)),
            format!("{}://{}:****@{}:{}", u.scheme(), u.username(), host, port),
        )
    } else {
        (None, format!("{}://{}:{}", u.scheme(), host, port))
    };
    Ok(Upstream {
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
    let mut proxy: Vec<String> = DEFAULT_PROXY_DOMAINS.iter().map(|s| s.to_string()).collect();
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
                    mode = if m.eq_ignore_ascii_case("smart") { Mode::Smart } else { Mode::All };
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
    eprintln!("ERROR: rules file {p} {last_err} — отказ запуска: потеря закреплений опаснее простоя");
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
fn rewrite_head(head: &[u8], auth: Option<&str>, strip_proxy_headers: bool, force_close: bool) -> Vec<u8> {
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

/// Одна попытка коннекта к upstream'у №i с учётом статистики и cooldown.
async fn try_upstream(state: &State, i: usize, pinned: bool) -> Option<TcpStream> {
    let u = &state.upstreams[i];
    match timeout(CONNECT_TIMEOUT, TcpStream::connect((u.host.as_str(), u.port))).await {
        Ok(Ok(s)) => {
            u.mark_ok();
            Some(s)
        }
        _ => {
            u.mark_fail();
            state.log_err(&if pinned {
                format!("назначенный прокси {} не отвечает — жду восстановления, IP не меняю", u.masked)
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
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_sec": state.started.elapsed().as_secs(),
        "listen": state.listen,
        "mode": state.mode.as_str(),
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
        ("200 OK", healthz_json(state))
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

    // Назначение сильнее режима: закреплённый домен идёт через свой upstream,
    // даже если smart-режим отправил бы его напрямую (или домен попал в
    // direct_domains) — иначе он утёк бы с реальным IP пользователя.
    let pin = preferred_upstream(&state, &req.host);
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
                    state.log_err(&format!("прямое соединение с {}:{} не удалось", req.host, req.port));
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
    quiet: bool,
    rules: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        upstreams: Vec::new(),
        listen: "127.0.0.1:8888".into(),
        quiet: false,
        rules: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--upstream" => {
                i += 1;
                a.upstreams.push(
                    argv.get(i).ok_or("--upstream requires URL")?.clone(),
                );
            }
            "--listen" => {
                i += 1;
                a.listen = argv.get(i).ok_or("--listen requires HOST:PORT")?.clone();
            }
            "--rules" => {
                i += 1;
                a.rules = Some(argv.get(i).ok_or("--rules requires FILE")?.clone());
            }
            "--quiet" => a.quiet = true,
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
    Ok(a)
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: {e}");
            eprintln!(
                "usage: clod-bridge --upstream http://user:pass@host:port [--upstream …] \
                 [--listen 127.0.0.1:8888] [--rules FILE] [--quiet]"
            );
            std::process::exit(1);
        }
    };

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
        proxy_domains,
        direct_domains,
        assignments,
        upstreams,
        total: AtomicU64::new(0),
        active: AtomicU64::new(0),
        via_upstream: AtomicU64::new(0),
        via_direct: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        last_errors: Mutex::new(std::collections::VecDeque::new()),
    });

    let listener = match TcpListener::bind(&args.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ERROR: cannot bind {} — {e}", args.listen);
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
