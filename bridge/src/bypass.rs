//! Соединение с прокси мимо VPN (см. bypass_route): сначала через
//! физический интерфейс, а если за BYPASS_HEAD_START он не ответил -
//! параллельно обычным путём (через VPN). Берём первое удачное.
//!
//! Итог запоминается и уходит в /healthz («vpn_bypass»): программа по нему
//! говорит человеку, что VPN обойдён или что он не пускает мимо себя.
//! Если мимо не пускает, а через VPN прокси отвечает, следующие соединения
//! какое-то время идут сразу через VPN, без лишней задержки.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

use crate::bypass_route::{self, Bypass};

/// Сколько ждём соединение мимо VPN, прежде чем параллельно пробовать
/// обычный путь. Физический путь обычно быстрее туннеля.
const BYPASS_HEAD_START: Duration = Duration::from_millis(1500);
/// Как часто заново смотрим таблицу маршрутов.
const ROUTE_TTL: Duration = Duration::from_secs(5);
/// Мимо VPN не пускает, а через VPN прокси отвечает - столько идём сразу
/// через VPN, потом пробуем обход снова (VPN могли переключить).
const PREFER_PLAIN_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
struct Memo {
    route: Option<Bypass>,
    checked: Option<Instant>,
    /// Последний итог соединения мимо VPN для этого туннеля.
    ok: Option<bool>,
    /// Когда обход последний раз удался.
    ok_at: Option<Instant>,
    prefer_plain_until: Option<Instant>,
}

static MEMO: Mutex<Option<Memo>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Memo) -> R) -> R {
    let mut g = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Memo::default))
}

/// Выход мимо VPN сейчас (с кэшем). Туннель сменился - прежний итог забыт.
async fn current() -> Option<Bypass> {
    let fresh = with(|m| m.checked.is_some_and(|t| t.elapsed() < ROUTE_TTL).then(|| m.route.clone()));
    if let Some(r) = fresh {
        return r;
    }
    let route = tokio::task::spawn_blocking(bypass_route::detect).await.ok().flatten();
    with(|m| {
        if m.route.as_ref().map(|b| &b.tunnel) != route.as_ref().map(|b| &b.tunnel) {
            m.ok = None;
            m.ok_at = None;
            m.prefer_plain_until = None;
        }
        m.route = route.clone();
        m.checked = Some(Instant::now());
    });
    route
}

/// Для /healthz: туннель, через какой интерфейс идём мимо и получилось ли.
/// None - VPN не забирает весь трафик, обходить нечего.
pub fn status_json() -> serde_json::Value {
    with(|m| match &m.route {
        Some(b) => serde_json::json!({
            "tunnel": b.tunnel,
            "via": b.iface,
            "ok": m.ok,
            "ok_age_sec": m.ok_at.map(|t| t.elapsed().as_secs()),
        }),
        None => serde_json::Value::Null,
    })
}

/// TCP к прокси: мимо VPN, если он есть, с подстраховкой обычным путём.
pub async fn connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let Some(route) = current().await else {
        return TcpStream::connect((host, port)).await;
    };
    // Адрес - заранее: привязка к интерфейсу только для IPv4.
    let addr = tokio::net::lookup_host((host, port)).await?.find(SocketAddr::is_ipv4);
    let Some(addr) = addr else {
        return TcpStream::connect((host, port)).await;
    };
    if with(|m| m.prefer_plain_until.is_some_and(|t| Instant::now() < t)) {
        return TcpStream::connect(addr).await;
    }

    let bypass = bound(addr, route.index);
    tokio::pin!(bypass);
    // Обход успел сам - лучший случай.
    let early = tokio::time::timeout(BYPASS_HEAD_START, &mut bypass).await;
    let bypass_err = match early {
        Ok(Ok(s)) => {
            note(true, false);
            return Ok(s);
        }
        Ok(Err(e)) => Some(e),
        Err(_) => None,
    };
    // Обход отказал сразу или молчит - пробуем и обычный путь.
    let plain = TcpStream::connect(addr);
    tokio::pin!(plain);
    if let Some(e) = bypass_err {
        return finish_plain(plain.await, e);
    }
    tokio::select! {
        r = &mut bypass => match r {
            Ok(s) => {
                note(true, false);
                Ok(s)
            }
            Err(e) => finish_plain(plain.await, e),
        },
        r = &mut plain => match r {
            // Через VPN ответили раньше, а обход ещё молчит: он, скорее
            // всего, заблокирован. Дождаться его - ещё полтаймаута на
            // каждом соединении, поэтому считаем, что мимо не пускает.
            Ok(s) => {
                note(false, true);
                Ok(s)
            }
            Err(_) => match bypass.await {
                Ok(s) => {
                    note(true, false);
                    Ok(s)
                }
                Err(e) => {
                    note(false, false);
                    Err(e)
                }
            },
        },
    }
}

fn finish_plain(plain: std::io::Result<TcpStream>, bypass_err: std::io::Error) -> std::io::Result<TcpStream> {
    match plain {
        Ok(s) => {
            note(false, true);
            Ok(s)
        }
        Err(_) => {
            note(false, false);
            Err(bypass_err)
        }
    }
}

/// Запомнить итог: `ok` - обход удался; `plain_ok` - не удался, но через VPN
/// прокси ответил (тогда какое-то время ходим сразу через VPN).
fn note(ok: bool, plain_ok: bool) {
    with(|m| {
        m.ok = Some(ok);
        if ok {
            m.ok_at = Some(Instant::now());
            m.prefer_plain_until = None;
        } else if plain_ok {
            m.prefer_plain_until = Some(Instant::now() + PREFER_PLAIN_FOR);
        }
    });
}

/// TCP-коннект с сокетом, привязанным к интерфейсу `index`.
async fn bound(addr: SocketAddr, index: u32) -> std::io::Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    bind_to_interface(&sock, index)?;
    sock.set_nonblocking(true)?;
    let std: std::net::TcpStream = sock.into();
    tokio::net::TcpSocket::from_std_stream(std).connect(addr).await
}

#[cfg(target_os = "macos")]
fn bind_to_interface(sock: &socket2::Socket, index: u32) -> std::io::Result<()> {
    sock.bind_device_by_index_v4(std::num::NonZeroU32::new(index))
}

#[cfg(target_os = "windows")]
fn bind_to_interface(sock: &socket2::Socket, index: u32) -> std::io::Result<()> {
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
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn bind_to_interface(_: &socket2::Socket, _: u32) -> std::io::Result<()> {
    Err(std::io::Error::other("обход VPN не поддерживается"))
}

#[cfg(test)]
mod tests {
    /// Живая привязка к интерфейсу этой машины: cargo test live_bound -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_bound_connect() {
        let phys = crate::bypass_route::live_physical().expect("нет физического выхода");
        let addr: std::net::SocketAddr = "1.1.1.1:443".parse().unwrap();
        let s = super::bound(addr, phys.0).await.expect("привязанный коннект");
        println!("через {} (#{}) - {:?}", phys.1, phys.0, s.local_addr());
        // Несуществующий интерфейс - ошибка, а не тихий обычный путь.
        assert!(super::bound(addr, 999_999).await.is_err());
    }
}
