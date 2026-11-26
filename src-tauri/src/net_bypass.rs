//! Проверки прокси мимо VPN - как ходит мост (bridge/src/bypass.rs).
//!
//! Включённый VPN забирает весь трафик, и пинг, «Проверить» и замер до
//! прокси шли бы через сервер VPN: прокси выглядел бы мёртвым, хотя мост
//! (он ходит мимо VPN) через него работает. Здесь тот же поиск физического
//! интерфейса, что у моста, и привязка к нему сокетов и запросов.
//!
//! Строгий VPN мимо себя не пускает - тогда проверка уходит обычным путём,
//! и какое-то время сразу им, без лишнего ожидания.

#[path = "../../bridge/src/bypass_route.rs"]
mod bypass_route;

use std::io;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use bypass_route::Bypass;

const ROUTE_TTL: Duration = Duration::from_secs(5);
/// Сколько ждём соединение мимо VPN, прежде чем идти обычным путём.
const BYPASS_TRY: Duration = Duration::from_millis(1500);
/// Мимо не пускают - столько ходим сразу обычным путём.
const PREFER_PLAIN_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
struct Memo {
    route: Option<Bypass>,
    checked: Option<Instant>,
    prefer_plain_until: Option<Instant>,
}

static MEMO: Mutex<Option<Memo>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Memo) -> R) -> R {
    let mut g = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Memo::default))
}

/// Выход мимо VPN сейчас (с кэшем на 5 с). Блокирующий: на Mac - netstat.
pub fn route() -> Option<Bypass> {
    if let Some(r) = with(|m| m.checked.is_some_and(|t| t.elapsed() < ROUTE_TTL).then(|| m.route.clone())) {
        return r;
    }
    let route = bypass_route::detect();
    with(|m| {
        if m.route.as_ref().map(|b| &b.tunnel) != route.as_ref().map(|b| &b.tunnel) {
            m.prefer_plain_until = None;
        }
        m.route = route.clone();
        m.checked = Some(Instant::now());
    });
    route
}

/// Обходить ли VPN для этого адреса: туннель есть, мимо него недавно не
/// отказывали, адрес в интернете (локальные и свои сети - всегда напрямую).
fn usable_for(ip: IpAddr) -> Option<Bypass> {
    let IpAddr::V4(v4) = ip else { return None };
    if v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() {
        return None;
    }
    if with(|m| m.prefer_plain_until.is_some_and(|t| Instant::now() < t)) {
        return None;
    }
    route()
}

/// Мимо VPN не вышло, а обычным путём - да: какое-то время не пробуем.
pub fn note_blocked() {
    with(|m| m.prefer_plain_until = Some(Instant::now() + PREFER_PLAIN_FOR));
}

/// TCP-коннект с таймаутом: мимо VPN, если он есть, иначе (или если мимо
/// не пускают) - обычным путём.
pub fn connect_timeout(addr: &SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    if let Some(r) = usable_for(addr.ip()) {
        if let Ok(s) = bound_connect(addr, r.index, timeout.min(BYPASS_TRY)) {
            return Ok(s);
        }
        let plain = TcpStream::connect_timeout(addr, timeout);
        if plain.is_ok() {
            note_blocked();
        }
        return plain;
    }
    TcpStream::connect_timeout(addr, timeout)
}

fn bound_connect(addr: &SocketAddr, index: u32, timeout: Duration) -> io::Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    bind_to_interface(&sock, index)?;
    sock.connect_timeout(&(*addr).into(), timeout)?;
    Ok(sock.into())
}

/// Запросы через прокси `proxy_host` - мимо VPN, если он есть. Возвращает,
/// привязан ли клиент (чтобы при отказе повторить обычным путём).
pub fn apply(b: reqwest::ClientBuilder, proxy_host: &str) -> (reqwest::ClientBuilder, bool) {
    let ip = match proxy_host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => ip,
        // Имя: локальным не бывает, проверяем только туннель.
        Err(_) if proxy_host.eq_ignore_ascii_case("localhost") => return (b, false),
        Err(_) => IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1)),
    };
    let Some(r) = usable_for(ip) else { return (b, false) };
    bind_client(b, &r)
}

#[cfg(target_os = "macos")]
fn bind_client(b: reqwest::ClientBuilder, r: &Bypass) -> (reqwest::ClientBuilder, bool) {
    (b.interface(&r.iface), true)
}

/// На Windows у reqwest нет привязки к интерфейсу - привязываем к адресу:
/// система отправит такое соединение через интерфейс, которому он принадлежит.
#[cfg(target_os = "windows")]
fn bind_client(b: reqwest::ClientBuilder, r: &Bypass) -> (reqwest::ClientBuilder, bool) {
    match source_ip(r.index) {
        Some(ip) => (b.local_address(ip), true),
        None => (b, false),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bind_client(b: reqwest::ClientBuilder, _: &Bypass) -> (reqwest::ClientBuilder, bool) {
    (b, false)
}

/// Свой IPv4 интерфейса: UDP-«соединение» через него ничего не отправляет,
/// но система выбирает адрес отправителя.
#[cfg(target_os = "windows")]
fn source_ip(index: u32) -> Option<IpAddr> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).ok()?;
    bind_to_interface(&sock, index).ok()?;
    let to: SocketAddr = "1.1.1.1:53".parse().ok()?;
    sock.connect(&to.into()).ok()?;
    let ip = sock.local_addr().ok()?.as_socket()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

#[cfg(target_os = "macos")]
fn bind_to_interface(sock: &socket2::Socket, index: u32) -> io::Result<()> {
    sock.bind_device_by_index_v4(std::num::NonZeroU32::new(index))
}

#[cfg(target_os = "windows")]
fn bind_to_interface(sock: &socket2::Socket, index: u32) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IP_UNICAST_IF};
    // IP_UNICAST_IF для IPv4 ждёт номер интерфейса в сетевом порядке байт.
    let v: u32 = index.to_be();
    // SAFETY: живой сокет, указатель на u32 и его размер.
    let rc = unsafe {
        setsockopt(
            sock.as_raw_socket() as usize,
            IPPROTO_IP,
            IP_UNICAST_IF,
            &v as *const u32 as *const u8,
            std::mem::size_of::<u32>() as i32,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bind_to_interface(_: &socket2::Socket, _: u32) -> io::Result<()> {
    Err(io::Error::other("обход VPN не поддерживается"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_never_bypass() {
        for ip in ["127.0.0.1", "192.168.1.10", "10.0.0.5", "169.254.1.1", "::1"] {
            assert!(usable_for(ip.parse().unwrap()).is_none(), "{ip}");
        }
    }

    /// Живая проверка на этой машине: cargo test --lib live_net_bypass -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_net_bypass() {
        let phys = bypass_route::live_physical().expect("нет физического выхода");
        let addr: SocketAddr = "1.1.1.1:443".parse().unwrap();
        let s = bound_connect(&addr, phys.0, Duration::from_secs(3)).expect("привязанный коннект");
        println!("через {} - {:?}", phys.1, s.local_addr());
        #[cfg(target_os = "windows")]
        println!("адрес интерфейса: {:?}", source_ip(phys.0));
    }
}
