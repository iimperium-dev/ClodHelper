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
//! Ещё один вход — SOCKS5 на 127.0.0.1:8891 (--socks-listen, "off" — выкл):
//! для приложений, которые умеют только SOCKS (Telegram для macOS). Всё,
//! что пришло туда, идёт ТОЛЬКО через upstream (как строгий вход), без DIRECT.
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// Потолок одновременных соединений — защита от пика деплоя (см. историю
/// python-моста: без лимита too-many-threads ронял процесс и SSE-стрим).
const MAX_CONNECTIONS: usize = 256;
/// Максимум байтов заголовков первого запроса.
const HEAD_LIMIT: usize = 65536;
/// Таймаут установки прямого (DIRECT) TCP-соединения.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Таймаут TCP-коннекта к upstream'у. Короче DIRECT: живой прокси отвечает
/// за десятки миллисекунд, а зависший SYN раньше съедал 20с — дольше, чем
/// клиент готов ждать, и 502 приходил уже в закрытый сокет.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Сколько ждём ответа upstream'а на CONNECT. Прокси, принявший TCP, но
/// молчащий на CONNECT, — главный источник «зависших на 15с» запросов.
const UPSTREAM_REPLY_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(10)
};
/// Попыток на ОДИН и тот же upstream до отказа. Повтор безопасен: клиенту
/// ещё ничего не отвечено, а выходной IP тот же (прокси тот же).
const UPSTREAM_ATTEMPTS: usize = 2;
/// Подряд идущих отказов, после которых upstream считается нездоровым в
/// /healthz. Одиночный сбой — не повод красить дашборд в красный.
const UNHEALTHY_STREAK: u64 = 3;
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
    /// SOCKS5-вход (Telegram): только upstream; прокси — назначенный группе
    /// «telegram.org», иначе основной (№0).
    Socks,
}

impl RoutingPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::UpstreamOnly => "upstream_only",
            Self::Socks => "socks_upstream_only",
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
    /// Отказов подряд (сбрасывается первым успехом).
    streak: AtomicU64,
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
    fn healthy(&self) -> bool {
        self.streak.load(Ordering::Relaxed) < UNHEALTHY_STREAK
    }
    fn mark_ok(&self) {
        self.ok.fetch_add(1, Ordering::Relaxed);
        self.streak.store(0, Ordering::Relaxed);
        if let Ok(mut g) = self.bad_until.lock() {
            *g = None;
        }
    }
    fn mark_fail(&self) {
        self.fail.fetch_add(1, Ordering::Relaxed);
        self.streak.fetch_add(1, Ordering::Relaxed);
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
    /// Последняя напечатанная ошибка и сколько раз она повторилась с тех пор:
    /// мёртвый прокси иначе пишет одну и ту же строку на каждый запрос.
    last_logged: Mutex<(String, u64)>,
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
            logfile::maybe_rotate();
            eprintln!("[{}] {msg}", wall_clock());
        }
    }
    /// Ошибки видны и в quiet-режиме + попадают в кольцевой буфер /healthz.
    /// Подряд идущие одинаковые строки схлопываются в «повторилось N раз».
    fn log_err(&self, msg: &str) {
        logfile::maybe_rotate();
        if let Ok(mut last) = self.last_logged.lock() {
            if last.0 == msg {
                last.1 += 1;
            } else {
                if last.1 > 0 {
                    eprintln!("[{}] ERROR (предыдущее повторилось ещё {} раз)", wall_clock(), last.1);
                }
                eprintln!("[{}] ERROR {msg}", wall_clock());
                *last = (msg.to_string(), 0);
            }
        }
        if let Ok(mut g) = self.last_errors.lock() {
            if g.len() >= LAST_ERRORS_CAP {
                g.pop_front();
            }
            g.push_back((self.started.elapsed().as_secs(), msg.to_string()));
        }
    }
}

/// Собственный файл журнала (`--log FILE`). launchd пишет stderr в
/// StandardErrorPath бесконечно и сам не ротирует; мост открывает тот же
/// файл, подставляет его на место stderr и при превышении LOG_MAX_BYTES
/// переименовывает в `<файл>.1` (одна предыдущая копия) и начинает новый.
mod logfile {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    pub const LOG_MAX_BYTES: u64 = 1024 * 1024;
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    static LOCK: Mutex<()> = Mutex::new(());

    /// `<файл>.1` рядом с журналом.
    pub fn rotated_path(path: &Path) -> PathBuf {
        let mut p = path.as_os_str().to_owned();
        p.push(".1");
        PathBuf::from(p)
    }

    /// Переименовать журнал в `.1`, если он больше `limit`. true — ротировали.
    pub fn rotate_if_needed(path: &Path, limit: u64) -> std::io::Result<bool> {
        match std::fs::metadata(path) {
            Ok(m) if m.len() > limit => {
                std::fs::rename(path, rotated_path(path))?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Открыть журнал на дозапись и подставить его на место stderr.
    #[cfg(unix)]
    fn redirect_stderr(path: &Path) -> std::io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        // SAFETY: dup2 атомарно заменяет fd 2 копией открытого дескриптора;
        // исходный `f` закрывается при drop, копия живёт до конца процесса.
        if unsafe { libc::dup2(f.as_raw_fd(), 2) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Включить собственный журнал. На Windows stderr моста никуда не пишется
    /// (задача Планировщика без перенаправления) — флаг принимается, но no-op.
    pub fn init(path: &str) -> Result<(), String> {
        #[cfg(unix)]
        {
            let path = PathBuf::from(path);
            let _ = rotate_if_needed(&path, LOG_MAX_BYTES);
            redirect_stderr(&path).map_err(|e| format!("--log {}: {e}", path.display()))?;
            let _ = PATH.set(path);
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    /// Вызывается перед каждой записью в журнал: stat — копейки на фоне сети.
    pub fn maybe_rotate() {
        #[cfg(unix)]
        if let Some(path) = PATH.get() {
            let Ok(_guard) = LOCK.lock() else { return };
            if matches!(rotate_if_needed(path, LOG_MAX_BYTES), Ok(true)) {
                let _ = redirect_stderr(path);
            }
        }
    }
}

/// Текущее время UTC «YYYY-MM-DD HH:MM:SSZ» — без chrono: журнал launchd
/// иначе не сопоставить с жалобой пользователя («отвалилось в 4:20»).
fn wall_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (H. Hinnant), эпоха 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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
        streak: AtomicU64::new(0),
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
                for d in proxy.iter_mut().chain(direct.iter_mut()) {
                    *d = normalize_host(d);
                }
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

/// Хвостовая точка FQDN ("claude.ai.") и регистр нормализуются — иначе
/// канонические имена обходили бы и роутинг, и закрепление.
fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Матч по суффиксу: "api.anthropic.com" матчится на "anthropic.com".
/// Оба аргумента уже нормализованы (normalize_host) — без аллокаций на
/// каждое сравнение: решение принимается на каждом соединении.
fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

fn decide(state: &State, host: &str) -> Route {
    if state.routing_policy != RoutingPolicy::Rules {
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
    // 3 пробы вместо системных 8 (macOS): мёртвый пир виден за ~60с, а не ~110с.
    #[cfg(not(windows))]
    let ka = ka.with_retries(3);
    let _ = SockRef::from(stream).set_tcp_keepalive(&ka);
    let _ = stream.set_nodelay(true);
}

/// Прочитать заголовки первого запроса (до \r\n\r\n, лимит HEAD_LIMIT).
/// Дедлайн один на ВСЮ голову: per-read таймаут позволял клиенту, капающему
/// по байту, держать permit семафора бесконечно.
async fn read_head(client: &mut TcpStream) -> Result<Vec<u8>, String> {
    read_head_within(client, HEAD_TIMEOUT).await
}

/// То же для любого потока и дедлайна (ответ upstream'а на CONNECT).
/// Возвращает всё прочитанное: байты после головы (ServerHello, пришедший
/// одним пакетом с «200») тоже принадлежат клиенту.
async fn read_head_within<S: AsyncRead + Unpin>(
    stream: &mut S,
    deadline: Duration,
) -> Result<Vec<u8>, String> {
    let read_all = async {
        let mut buf = Vec::with_capacity(2048);
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| format!("head read: {e}"))?;
            if n == 0 {
                return Err("peer closed before full head".to_string());
            }
            // Ищем конец только в новом хвосте (+3 байта стыка), а не
            // пересканируем весь буфер на каждом read.
            let from = buf.len().saturating_sub(3);
            buf.extend_from_slice(&chunk[..n]);
            if find_head_end(&buf[from..]).is_some() {
                return Ok(buf);
            }
            if buf.len() > HEAD_LIMIT {
                return Err("headers too large".to_string());
            }
        }
    };
    timeout(deadline, read_all)
        .await
        .map_err(|_| "head read timeout".to_string())?
}

/// Код статуса из первой строки ответа ("HTTP/1.1 200 Connection established").
fn status_code(head: &[u8]) -> Option<u16> {
    let line_end = head.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&head[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
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
    match state.routing_policy {
        RoutingPolicy::UpstreamOnly => strict_upstream(state),
        RoutingPolicy::Socks => socks_upstream(state),
        RoutingPolicy::Rules => preferred_upstream(state, host),
    }
}

/// Telegram ходит по IP дата-центров, а не по доменам — назначение берём для
/// группы целиком (по «telegram.org»), иначе основной прокси. Один прокси на
/// все соединения: Telegram, как и Claude, не любит скачков IP.
fn socks_upstream(state: &State) -> Pin {
    match preferred_upstream(state, "telegram.org") {
        Pin::None if !state.upstreams.is_empty() => Pin::Pinned(0),
        Pin::None => Pin::Broken,
        pin => pin,
    }
}

/// Установленный через upstream канал: сокет + ответ upstream'а на CONNECT
/// (его отдаём клиенту как есть; для plain HTTP ответ придёт через pipe).
struct UpstreamConn {
    stream: TcpStream,
    reply: Option<Vec<u8>>,
}

/// Одна попытка: TCP-коннект, отправка первого запроса с auth и — для
/// CONNECT — ожидание ответа прокси. Ошибка = клиенту ещё ничего не ушло,
/// попытку можно безопасно повторить.
async fn attempt_upstream(u: &Upstream, head: &[u8], is_connect: bool) -> Result<UpstreamConn, String> {
    let mut stream = timeout(
        UPSTREAM_CONNECT_TIMEOUT,
        TcpStream::connect((u.host.as_str(), u.port)),
    )
    .await
    .map_err(|_| "нет ответа на TCP-коннект".to_string())?
    .map_err(|e| format!("коннект: {e}"))?;
    enable_keepalive(&stream);
    let rewritten = rewrite_head(head, u.auth.as_deref(), false, !is_connect);
    stream
        .write_all(&rewritten)
        .await
        .map_err(|e| format!("запись запроса: {e}"))?;
    if !is_connect {
        // Ответ на обычный HTTP — это ответ origin'а, он может думать долго
        // (стрим LLM): таймаут на него вешать нельзя.
        return Ok(UpstreamConn { stream, reply: None });
    }
    let reply = read_head_within(&mut stream, UPSTREAM_REPLY_TIMEOUT)
        .await
        .map_err(|e| {
            if e.contains("timeout") {
                "принял соединение, но молчит на CONNECT".to_string()
            } else {
                format!("ответ на CONNECT: {e}")
            }
        })?;
    Ok(UpstreamConn { stream, reply: Some(reply) })
}

/// Попытки к upstream'у №i с учётом статистики. `retry` — повторить один раз
/// тот же прокси (IP не меняется), прежде чем сдаться.
async fn try_upstream(
    state: &State,
    i: usize,
    pinned: bool,
    retry: bool,
    head: &[u8],
    is_connect: bool,
) -> Option<UpstreamConn> {
    let u = &state.upstreams[i];
    let attempts = if retry { UPSTREAM_ATTEMPTS } else { 1 };
    let mut last_err = String::new();
    for attempt in 0..attempts {
        match attempt_upstream(u, head, is_connect).await {
            Ok(conn) => {
                match conn.reply.as_deref().map(status_code) {
                    // 2xx (или plain HTTP) — прокси жив и пустил нас.
                    None | Some(Some(200..=299)) => u.mark_ok(),
                    // Прокси ответил, но отказал (407 — логин/пароль, 5xx — не
                    // достучался до сайта). Повторяем 5xx один раз; ответ
                    // отдаём клиенту как есть — молча менять IP нельзя.
                    Some(code) => {
                        u.mark_fail();
                        let code = code.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                        if code == "407" {
                            state.log_err(&format!(
                                "прокси {} отклонил логин/пароль (407) — проверьте данные прокси",
                                u.masked
                            ));
                        } else if code.starts_with('5') && attempt + 1 < attempts {
                            last_err = format!("ответил {code}");
                            continue;
                        } else {
                            state.log_err(&format!("прокси {} ответил {code} на CONNECT", u.masked));
                        }
                    }
                }
                if attempt > 0 {
                    state.log(&format!("прокси {}: успех со второй попытки", u.masked));
                }
                return Some(conn);
            }
            Err(e) => last_err = e,
        }
    }
    u.mark_fail();
    state.log_err(&if pinned {
        format!(
            "назначенный прокси {} не отвечает ({last_err}) — жду восстановления, IP не меняю",
            u.masked
        )
    } else {
        format!("прокси {} не отвечает ({last_err}) — пробую следующий", u.masked)
    });
    None
}

/// Выбрать живой upstream и подключиться.
/// `pinned` (назначенный домену прокси) — СТРОГО: пробуем только его, на
/// другие НЕ уходим — выходной IP сервиса не должен прыгать. Зато ему (как и
/// единственному прокси в пуле) положен повтор: зависание одного TCP-коннекта
/// к прокси — частый сбой, и второй коннект обычно проходит.
/// Без назначения — passive failover: первый проход пропускает остывающих,
/// второй шанс даётся ТОЛЬКО пропущенным (повторять только что упавших —
/// удвоение таймаута для клиента без шанса на успех).
async fn connect_upstream(
    state: &State,
    pinned: Option<usize>,
    head: &[u8],
    is_connect: bool,
) -> Option<(UpstreamConn, usize)> {
    let n = state.upstreams.len();
    let (order, is_pin): (Vec<usize>, bool) = match pinned {
        Some(p) if p < n => (vec![p], true),
        Some(_) => return None, // битый индекс — сюда не должен дойти (Pin::Broken отсекается раньше)
        None => ((0..n).collect(), false),
    };
    let retry = is_pin || n == 1;
    if is_pin {
        // Остывание для закреплённого бессмысленно: альтернативы нет.
        return try_upstream(state, order[0], true, retry, head, is_connect)
            .await
            .map(|c| (c, order[0]));
    }
    let mut skipped: Vec<usize> = Vec::new();
    for &i in &order {
        if state.upstreams[i].cooling() {
            skipped.push(i);
            continue;
        }
        if let Some(c) = try_upstream(state, i, false, retry, head, is_connect).await {
            return Some((c, i));
        }
    }
    for &i in &skipped {
        if let Some(c) = try_upstream(state, i, false, retry, head, is_connect).await {
            return Some((c, i));
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
                "healthy": u.healthy(),
                "cooling": u.cooling(),
                "ok": u.ok.load(Ordering::Relaxed),
                "fail": u.fail.load(Ordering::Relaxed),
                "fail_streak": u.streak.load(Ordering::Relaxed),
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
        "direct_allowed": state.routing_policy == RoutingPolicy::Rules,
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

async fn handle(mut client: TcpStream, state: Arc<State>, sem: Arc<Semaphore>) {
    let head = match read_head(&mut client).await {
        Ok(h) => h,
        Err(e) => {
            state.log(&format!("bad head: {e}"));
            return;
        }
    };
    let mut req = match parse_request(&head) {
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

    // Лимит — только на проксируемые соединения: /healthz отвечает даже при
    // забитом пуле, и дашборд видит «мост жив, но перегружен», а не «мёртв».
    let _permit = match sem.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            state.log_err(&format!("лимит {MAX_CONNECTIONS} соединений — отбиваю запрос, сервер жив"));
            let _ = client
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            return;
        }
    };
    req.host = normalize_host(&req.host);

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
    let (mut server, upstream_reply): (TcpStream, Option<Vec<u8>>) = match route {
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
            match connect_upstream(&state, pinned, &head, req.is_connect).await {
                Some((c, _)) => {
                    state.via_upstream.fetch_add(1, Ordering::Relaxed);
                    (c.stream, c.reply)
                }
                None => {
                    state.errors.fetch_add(1, Ordering::Relaxed);
                    // Закреплённый отказ уже залогирован в try_upstream.
                    if pinned.is_none() && state.upstreams.len() > 1 {
                        state.log_err("все upstream-прокси недоступны");
                    }
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
            // Первый запрос с auth уже ушёл upstream'у (connect_upstream).
            // Ответ на CONNECT прочитан там же — отдаём клиенту как есть,
            // вместе с данными, пришедшими следом.
            if let Some(reply) = upstream_reply {
                if client.write_all(&reply).await.is_err() {
                    return;
                }
            }
        }
    }

    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}

// ─── SOCKS5 (Telegram) ───────────────────────────────────────────

/// Код ответа SOCKS5 (RFC 1928 §6).
const SOCKS_OK: u8 = 0x00;
const SOCKS_GENERAL_FAILURE: u8 = 0x01;
const SOCKS_HOST_UNREACHABLE: u8 = 0x04;
const SOCKS_CONNECTION_REFUSED: u8 = 0x05;
const SOCKS_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const SOCKS_ADDRESS_NOT_SUPPORTED: u8 = 0x08;

async fn socks_reply(client: &mut TcpStream, code: u8) -> std::io::Result<()> {
    // BND.ADDR/PORT клиентам не нужны (Telegram их не читает) — нули.
    client.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}

/// Рукопожатие SOCKS5 без аутентификации (вход только на loopback) и запрос
/// CONNECT → (хост, порт). UDP ASSOCIATE/BIND не поддерживаются: звонки
/// Telegram через прокси не пойдут (это же ограничение у HTTP-прокси).
async fn socks_handshake(client: &mut TcpStream) -> Result<(String, u16), String> {
    let mut hdr = [0u8; 2];
    client.read_exact(&mut hdr).await.map_err(|e| format!("greeting: {e}"))?;
    if hdr[0] != 5 {
        return Err(format!("не SOCKS5 (версия {})", hdr[0]));
    }
    let mut methods = vec![0u8; hdr[1] as usize];
    client.read_exact(&mut methods).await.map_err(|e| format!("methods: {e}"))?;
    if !methods.contains(&0) {
        let _ = client.write_all(&[5, 0xFF]).await;
        return Err("клиент требует аутентификацию — вход без пароля".into());
    }
    client.write_all(&[5, 0]).await.map_err(|e| format!("greeting reply: {e}"))?;

    let mut req = [0u8; 4];
    client.read_exact(&mut req).await.map_err(|e| format!("request: {e}"))?;
    if req[0] != 5 {
        return Err("битый запрос SOCKS5".into());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            client.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await.map_err(|e| e.to_string())?;
            let mut name = vec![0u8; len[0] as usize];
            client.read_exact(&mut name).await.map_err(|e| e.to_string())?;
            String::from_utf8(name).map_err(|_| "домен не UTF-8".to_string())?
        }
        4 => {
            let mut a = [0u8; 16];
            client.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        other => {
            let _ = socks_reply(client, SOCKS_ADDRESS_NOT_SUPPORTED).await;
            return Err(format!("тип адреса {other} не поддерживается"));
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await.map_err(|e| e.to_string())?;
    if req[1] != 1 {
        let _ = socks_reply(client, SOCKS_COMMAND_NOT_SUPPORTED).await;
        return Err(format!("команда SOCKS {} не поддерживается (только CONNECT)", req[1]));
    }
    Ok((host, u16::from_be_bytes(port)))
}

async fn handle_socks(mut client: TcpStream, state: Arc<State>, sem: Arc<Semaphore>) {
    let (host, port) = match timeout(HEAD_TIMEOUT, socks_handshake(&mut client)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            state.log(&format!("socks: {e}"));
            return;
        }
        Err(_) => return,
    };
    let Ok(_permit) = sem.try_acquire_owned() else {
        state.log_err(&format!("лимит {MAX_CONNECTIONS} соединений SOCKS — отбиваю запрос"));
        let _ = socks_reply(&mut client, SOCKS_GENERAL_FAILURE).await;
        return;
    };
    state.total.fetch_add(1, Ordering::Relaxed);
    state.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(&state);

    let host = normalize_host(&host);
    let Pin::Pinned(i) = request_upstream(&state, &host) else {
        state.errors.fetch_add(1, Ordering::Relaxed);
        state.log_err("SOCKS: назначенный Telegram прокси вне пула — отказываю (IP менять нельзя)");
        let _ = socks_reply(&mut client, SOCKS_GENERAL_FAILURE).await;
        return;
    };
    if state.upstreams[i].scheme != "http" {
        state.errors.fetch_add(1, Ordering::Relaxed);
        state.log_err("SOCKS: HTTPS upstream пока не поддерживается");
        let _ = socks_reply(&mut client, SOCKS_GENERAL_FAILURE).await;
        return;
    }
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let head = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    let Some((conn, _)) = connect_upstream(&state, Some(i), head.as_bytes(), true).await else {
        state.errors.fetch_add(1, Ordering::Relaxed);
        let _ = socks_reply(&mut client, SOCKS_HOST_UNREACHABLE).await;
        return;
    };
    let reply = conn.reply.unwrap_or_default();
    if !matches!(status_code(&reply), Some(200..=299)) {
        state.errors.fetch_add(1, Ordering::Relaxed);
        let _ = socks_reply(&mut client, SOCKS_CONNECTION_REFUSED).await;
        return;
    }
    state.via_upstream.fetch_add(1, Ordering::Relaxed);
    let mut server = conn.stream;
    if socks_reply(&mut client, SOCKS_OK).await.is_err() {
        return;
    }
    // Байты, пришедшие от сервера вместе с «200» от прокси, — клиенту.
    if let Some(end) = find_head_end(&reply) {
        let tail = &reply[end + 4..];
        if !tail.is_empty() && client.write_all(tail).await.is_err() {
            return;
        }
    }
    enable_keepalive(&client);
    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}

async fn serve_socks_listener(listener: TcpListener, state: Arc<State>, sem: Arc<Semaphore>) {
    loop {
        match listener.accept().await {
            Ok((client, _)) => {
                tokio::spawn(handle_socks(client, state.clone(), sem.clone()));
            }
            Err(e) => {
                state.log_err(&format!("socks accept: {e} — продолжаю слушать"));
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

// ─── main ────────────────────────────────────────────────────────

struct Args {
    upstreams: Vec<String>,
    listen: String,
    strict_listen: String,
    /// SOCKS5-вход для Telegram; None — выключен (`--socks-listen off`).
    socks_listen: Option<String>,
    quiet: bool,
    rules: Option<String>,
    /// Собственный файл журнала с ротацией (см. mod logfile).
    log: Option<String>,
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
        socks_listen: Some("127.0.0.1:8891".into()),
        quiet: false,
        rules: None,
        log: None,
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
            "--upstream-file" => {
                // Файл (0600) вместо argv: аргументы процесса видит любой
                // пользователь машины через ps, а с ними — пароль прокси.
                i += 1;
                let path = argv.get(i).ok_or("--upstream-file requires FILE")?;
                a.upstreams.extend(read_upstream_file(path)?);
            }
            "--listen" => {
                i += 1;
                a.listen = argv.get(i).ok_or("--listen requires HOST:PORT")?.clone();
            }
            "--socks-listen" => {
                i += 1;
                let v = argv.get(i).ok_or("--socks-listen requires IP:PORT or off")?;
                a.socks_listen = (v != "off").then(|| v.clone());
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
            "--log" => {
                i += 1;
                a.log = Some(argv.get(i).ok_or("--log requires FILE")?.clone());
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
    // SOCKS без пароля — только на loopback, иначе это открытый прокси в сеть.
    if let Some(socks) = &a.socks_listen {
        let addr = socks
            .parse::<SocketAddr>()
            .map_err(|_| "--socks-listen requires a literal loopback IP and port")?;
        if !addr.ip().is_loopback() || addr.port() == 0 {
            return Err("--socks-listen must use a loopback IP and a non-zero port".into());
        }
    }
    Ok(a)
}

/// Upstream'ы из файла: по одному URL в строке, пустые строки и `#` игнорируются.
fn read_upstream_file(path: &str) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("--upstream-file {path}: {e}"))?;
    Ok(text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

/// launchd по умолчанию даёт процессу 256 дескрипторов: ~120 туннелей и
/// EMFILE раньше, чем сработает MAX_CONNECTIONS. Поднимаем мягкий лимит.
#[cfg(unix)]
fn raise_fd_limit() {
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit/setrlimit пишут/читают только переданную структуру.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 {
            let want: libc::rlim_t = 4096;
            if lim.rlim_cur < want {
                lim.rlim_cur = want.min(lim.rlim_max);
                libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
            }
        }
    }
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
                 [--rules FILE] [--log FILE] [--quiet] [--supervise]"
            );
            std::process::exit(1);
        }
    };

    if args.supervise {
        run_supervisor();
    }
    #[cfg(unix)]
    raise_fd_limit();
    if let Some(path) = args.log.as_deref() {
        if let Err(e) = logfile::init(path) {
            // Не фатально: пишем в унаследованный stderr, как раньше.
            eprintln!("ERROR: {e}");
        }
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
        last_logged: Mutex::new((String::new(), 0)),
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
        last_logged: Mutex::new((String::new(), 0)),
    });

    let socks_state = Arc::new(State {
        started: state.started,
        listen: args.socks_listen.clone().unwrap_or_default(),
        quiet: args.quiet,
        mode,
        routing_policy: RoutingPolicy::Socks,
        proxy_domains: Vec::new(),
        direct_domains: Vec::new(),
        assignments: state.assignments.clone(),
        upstreams: state.upstreams.clone(),
        total: AtomicU64::new(0),
        active: AtomicU64::new(0),
        via_upstream: AtomicU64::new(0),
        via_direct: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        last_errors: Mutex::new(std::collections::VecDeque::new()),
        last_logged: Mutex::new((String::new(), 0)),
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

    // Семафор — как в python-мосте: лишние соединения не валят процесс.
    // Свой пул у каждого входа: браузерный трафик на общем входе не должен
    // выедать слоты Claude Desktop на строгом. Accept-цикл НИКОГДА не падает.
    let sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let strict_sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    // SOCKS-вход независим от остальных: занятый порт 8891 не мешает Claude.
    if let Some(addr) = args.socks_listen.as_deref() {
        match TcpListener::bind(addr).await {
            Ok(l) => {
                socks_state.log(&format!("SOCKS5 (Telegram) upstream-only listener on {addr}"));
                tokio::spawn(serve_socks_listener(
                    l,
                    socks_state,
                    Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                ));
            }
            Err(e) => state.log_err(&format!(
                "cannot bind SOCKS listener {addr} — {e}; Telegram через мост недоступен"
            )),
        }
    }
    match strict_listener {
        Ok(strict_listener) => {
            strict_state.log(&format!(
                "Claude upstream-only listener on {} | ready={}",
                args.strict_listen,
                strict_state.strict_ready(),
            ));
            tokio::join!(
                serve_listener(listener, state, sem.clone()),
                serve_listener(strict_listener, strict_state, strict_sem),
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
        let (client, _addr) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                state.log_err(&format!("accept: {e} — продолжаю слушать"));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        // Слот берётся внутри handle и без ожидания: раньше accept-цикл
        // стоял до 5с в ожидании слота, и в это время не отвечал даже /healthz.
        tokio::spawn(handle(client, state.clone(), sem.clone()));
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
            last_logged: Mutex::new((String::new(), 0)),
        }
    }

    async fn single_request(state: Arc<State>, request: String) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            handle(client, state, Arc::new(Semaphore::new(MAX_CONNECTIONS))).await;
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
                // Как настоящий прокси: отвечает, дочитав заголовки, а не EOF.
                let mut request = read_head_within(&mut stream, Duration::from_secs(3))
                    .await
                    .unwrap();
                let response = if is_connect {
                    b"HTTP/1.1 200 Connection Established\r\n\r\nupstream tunnel".as_slice()
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nupstream http".as_slice()
                };
                stream.write_all(response).await.unwrap();
                stream.read_to_end(&mut request).await.unwrap();
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
            handle(client, state, Arc::new(Semaphore::new(MAX_CONNECTIONS))).await;
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

    /// Фейковый прокси: на i-е соединение отвечает replies[i] (None — молчит,
    /// держа сокет). Возвращает адрес и счётчик принятых соединений.
    async fn scripted_proxy(replies: Vec<Option<&'static [u8]>>) -> (SocketAddr, Arc<AtomicU64>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicU64::new(0));
        let counter = accepted.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                counter.fetch_add(1, Ordering::Relaxed);
                let _ = read_head_within(&mut stream, Duration::from_secs(3)).await;
                match reply {
                    // Ответил и закрыл — туннель завершится, клиент получит EOF.
                    Some(r) => {
                        let _ = stream.write_all(r).await;
                    }
                    None => held.push(stream),
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        (addr, accepted)
    }

    #[tokio::test]
    async fn silent_upstream_is_retried_on_the_same_proxy() {
        let (addr, accepted) = scripted_proxy(vec![
            None,
            Some(b"HTTP/1.1 200 Connection Established\r\n\r\nhello"),
        ])
        .await;
        let state = Arc::new(test_state(
            RoutingPolicy::UpstreamOnly,
            vec![parse_upstream(&format!("http://u:p@{addr}")).unwrap()],
        ));
        let response = single_request(state.clone(), "CONNECT claude.ai:443 HTTP/1.1\r\n\r\n".into()).await;
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.ends_with("hello"), "данные после головы ответа потеряны: {text}");
        assert_eq!(accepted.load(Ordering::Relaxed), 2);
        let u = &state.upstreams[0];
        assert_eq!(u.ok.load(Ordering::Relaxed), 1);
        assert!(u.healthy());
    }

    #[tokio::test]
    async fn proxy_refusal_is_forwarded_without_retry_or_ip_change() {
        let (addr, accepted) =
            scripted_proxy(vec![Some(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")]).await;
        let state = Arc::new(test_state(
            RoutingPolicy::UpstreamOnly,
            vec![parse_upstream(&format!("http://u:p@{addr}")).unwrap()],
        ));
        let response = single_request(state.clone(), "CONNECT claude.ai:443 HTTP/1.1\r\n\r\n".into()).await;
        assert!(response.starts_with(b"HTTP/1.1 407"));
        assert_eq!(accepted.load(Ordering::Relaxed), 1);
        assert_eq!(state.upstreams[0].fail.load(Ordering::Relaxed), 1);
        assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn silent_upstream_gives_502_and_unhealthy_only_after_a_streak() {
        let (addr, accepted) = scripted_proxy(vec![None; 6]).await;
        let state = Arc::new(test_state(
            RoutingPolicy::UpstreamOnly,
            vec![parse_upstream(&format!("http://{addr}")).unwrap()],
        ));
        let request = "CONNECT claude.ai:443 HTTP/1.1\r\n\r\n";
        let response = single_request(state.clone(), request.into()).await;
        assert!(response.starts_with(b"HTTP/1.1 502"));
        assert_eq!(accepted.load(Ordering::Relaxed), 2, "одна попытка + один повтор");
        assert!(state.upstreams[0].healthy(), "один сбой — ещё не «нездоров»");
        for _ in 0..2 {
            single_request(state.clone(), request.into()).await;
        }
        assert!(!state.upstreams[0].healthy());
        let health: serde_json::Value = serde_json::from_str(&healthz_json(&state)).unwrap();
        assert_eq!(health["upstreams"][0]["fail_streak"], 3);
    }

    #[test]
    fn upstream_file_skips_comments_and_bom() {
        let path = std::env::temp_dir().join(format!("clod-upstreams-{}.txt", std::process::id()));
        std::fs::write(&path, "\u{feff}# основной\nhttp://a:b@127.0.0.1:1\n\n  http://127.0.0.1:2  \n").unwrap();
        let args = parse_args_from(vec!["--upstream-file".into(), path.to_str().unwrap().into()]).unwrap();
        assert_eq!(args.upstreams, vec!["http://a:b@127.0.0.1:1", "http://127.0.0.1:2"]);
        std::fs::remove_file(path).unwrap();
        assert!(parse_args_from(vec!["--upstream-file".into(), "/nonexistent/clod".into()]).is_err());
    }

    #[test]
    fn host_matching_is_suffix_safe() {
        assert!(host_matches("api.anthropic.com", "anthropic.com"));
        assert!(host_matches("anthropic.com", "anthropic.com"));
        assert!(!host_matches("evilanthropic.com", "anthropic.com"));
        assert_eq!(normalize_host("Claude.AI."), "claude.ai");
    }

    #[test]
    fn wall_clock_formats_utc() {
        let t = wall_clock();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.starts_with("20") && t.ends_with('Z'));
    }

    #[test]
    fn log_rotates_only_past_the_limit() {
        let dir = std::env::temp_dir().join(format!("clod-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bridge.log");
        assert!(!logfile::rotate_if_needed(&path, 10).unwrap(), "нет файла — не ошибка");
        std::fs::write(&path, b"0123456789").unwrap();
        assert!(!logfile::rotate_if_needed(&path, 10).unwrap());
        std::fs::write(&path, b"0123456789ab").unwrap();
        assert!(logfile::rotate_if_needed(&path, 10).unwrap());
        assert!(!path.exists());
        let rotated = logfile::rotated_path(&path);
        assert_eq!(rotated.file_name().unwrap(), "bridge.log.1");
        assert_eq!(std::fs::read(&rotated).unwrap(), b"0123456789ab");
        // Вторая ротация затирает старую копию — на диске максимум 2 файла.
        std::fs::write(&path, b"new-content-xx").unwrap();
        assert!(logfile::rotate_if_needed(&path, 10).unwrap());
        assert_eq!(std::fs::read(&rotated).unwrap(), b"new-content-xx");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            parse_args_from(vec!["--upstream".into(), "http://127.0.0.1:1".into(), "--log".into(), "/x/b.log".into()])
                .unwrap()
                .log
                .as_deref(),
            Some("/x/b.log")
        );
    }

    /// Прогнать один SOCKS-диалог: клиент шлёт `script`, ответ читается до EOF.
    async fn socks_session(state: Arc<State>, script: Vec<u8>) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            handle_socks(client, state, Arc::new(Semaphore::new(MAX_CONNECTIONS))).await;
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(&script).await.unwrap();
        let mut out = Vec::new();
        timeout(Duration::from_secs(3), client.read_to_end(&mut out)).await.unwrap().unwrap();
        // Туннель живёт, пока открыты обе стороны — закрываем клиентскую.
        drop(client);
        timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
        out
    }

    fn socks_state(upstreams: Vec<Upstream>) -> State {
        let mut st = test_state(RoutingPolicy::Socks, upstreams);
        st.direct_domains = vec!["149.154.167.51".into()]; // правила SOCKS не касаются
        st
    }

    #[tokio::test]
    async fn socks_connect_goes_through_upstream_only() {
        let (addr, accepted) = scripted_proxy(vec![Some(
            b"HTTP/1.0 200 Connection established\r\n\r\nmtproto",
        )])
        .await;
        let state = Arc::new(socks_state(vec![parse_upstream(&format!("http://u:p@{addr}")).unwrap()]));
        // greeting(no-auth) + CONNECT 149.154.167.51:443 (IPv4)
        let mut script = vec![5, 1, 0, 5, 1, 0, 1, 149, 154, 167, 51];
        script.extend(443u16.to_be_bytes());
        let out = socks_session(state.clone(), script).await;
        assert_eq!(&out[..2], &[5, 0], "выбран метод без пароля");
        assert_eq!(&out[2..4], &[5, SOCKS_OK]);
        assert!(out.ends_with(b"mtproto"), "данные после «200» потеряны");
        assert_eq!(accepted.load(Ordering::Relaxed), 1);
        assert_eq!(state.via_upstream.load(Ordering::Relaxed), 1);
        assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn socks_domain_target_uses_assigned_telegram_proxy() {
        let (addr, accepted) = scripted_proxy(vec![Some(b"HTTP/1.1 200 OK\r\n\r\n")]).await;
        let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut st = socks_state(vec![
            parse_upstream(&format!("http://{}", other.local_addr().unwrap())).unwrap(),
            parse_upstream(&format!("http://{addr}")).unwrap(),
        ]);
        st.assignments = vec![("telegram.org".into(), 1)];
        let state = Arc::new(st);
        let mut script = vec![5, 1, 0, 5, 1, 0, 3, 12];
        script.extend(b"web.telegram");
        script.extend(443u16.to_be_bytes());
        let out = socks_session(state, script).await;
        assert_eq!(&out[2..4], &[5, SOCKS_OK]);
        assert_eq!(accepted.load(Ordering::Relaxed), 1);
        assert_no_connection(&other).await;
    }

    #[tokio::test]
    async fn socks_rejects_udp_auth_and_dead_upstream() {
        let (addr, _) = scripted_proxy(vec![]).await;
        let state = Arc::new(socks_state(vec![parse_upstream(&format!("http://{addr}")).unwrap()]));
        // UDP ASSOCIATE (cmd 3) — не поддерживаем.
        let out = socks_session(state.clone(), vec![5, 1, 0, 5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        assert_eq!(&out[2..4], &[5, SOCKS_COMMAND_NOT_SUPPORTED]);
        // Только логин/пароль (метод 2) — отказ 0xFF.
        let out = socks_session(state, vec![5, 1, 2]).await;
        assert_eq!(out, vec![5, 0xFF]);
        // Мёртвый upstream — host unreachable, никаких прямых соединений.
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        let state = Arc::new(socks_state(vec![parse_upstream(&format!("http://{dead_addr}")).unwrap()]));
        let mut script = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
        script.extend(443u16.to_be_bytes());
        let out = socks_session(state.clone(), script).await;
        assert_eq!(&out[2..4], &[5, SOCKS_HOST_UNREACHABLE]);
        assert_eq!(state.via_direct.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn socks_listen_is_loopback_only_and_can_be_disabled() {
        let parse = |v: &str| {
            parse_args_from(vec![
                "--upstream".into(),
                "http://127.0.0.1:1".into(),
                "--socks-listen".into(),
                v.into(),
            ])
        };
        assert_eq!(
            parse_args_from(vec!["--upstream".into(), "http://127.0.0.1:1".into()])
                .unwrap()
                .socks_listen
                .as_deref(),
            Some("127.0.0.1:8891")
        );
        assert!(parse("off").unwrap().socks_listen.is_none());
        assert!(parse("0.0.0.0:8891").is_err());
        assert!(parse("[::1]:8891").is_ok());
    }
}
