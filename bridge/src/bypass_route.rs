//! Выход мимо VPN: какой интерфейс у компьютера «настоящий» (Wi-Fi, кабель),
//! когда маршрут по умолчанию забрал VPN-туннель.
//!
//! Мост ходит к прокси обычными TCP-соединениями. Включённый VPN в режиме
//! туннеля забирает весь трафик, и соединение к прокси идёт через сервер
//! VPN: прокси видит чужой IP и часто не отвечает. Если привязать сокет к
//! физическому интерфейсу (macOS - IP_BOUND_IF, Windows - IP_UNICAST_IF),
//! система отправит его по маршруту этого интерфейса, в обход туннеля.
//! Строгие VPN (kill switch, «весь трафик только через меня») такое
//! соединение режут - это видно по результату, а не угадывается заранее.
//!
//! Модуль только находит интерфейс; привязку сокета делает вызывающий.
//! Общий для моста и программы (программа подключает его через #[path]):
//! только std и windows-sys, без других зависимостей.

/// Куда ходить мимо VPN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bypass {
    /// Номер физического интерфейса (if_index).
    pub index: u32,
    /// Его имя («en0», «Wi-Fi») - для подписи в программе.
    pub iface: String,
    /// Имя туннеля VPN («utun4», «happ-tun»).
    pub tunnel: String,
}

/// Маршрут по умолчанию (0.0.0.0/0 или половинка 0/1, 128/1).
#[derive(Debug, Clone)]
pub struct DefaultRoute {
    pub iface: String,
    pub index: u32,
    /// Интерфейс - туннель VPN.
    pub tunnel: bool,
    /// Маршрут на весь интернет (/0), а не половинка.
    pub whole: bool,
    /// Через шлюз (у физического выхода в интернет он есть).
    pub gateway: bool,
    /// Чем меньше, тем главнее.
    pub metric: u32,
}

/// Есть туннель с маршрутом по умолчанию и рядом физический выход - вернуть
/// физический. Нет туннеля - обходить нечего (None).
pub fn pick(routes: &[DefaultRoute]) -> Option<Bypass> {
    let tunnel = routes.iter().find(|r| r.tunnel)?;
    let phys = routes
        .iter()
        .filter(|r| !r.tunnel && r.whole && r.gateway && r.index != 0)
        .min_by_key(|r| r.metric)?;
    Some(Bypass { index: phys.index, iface: phys.iface.clone(), tunnel: tunnel.iface.clone() })
}

/// Найти выход мимо VPN сейчас. Блокирующий вызов: на Mac запускает netstat.
pub fn detect() -> Option<Bypass> {
    pick(&backend::default_routes())
}

/// Физический выход по умолчанию (номер, имя) - для живых тестов.
#[cfg(test)]
pub fn live_physical() -> Option<(u32, String)> {
    backend::default_routes().into_iter().find(|r| !r.tunnel && r.whole).map(|r| (r.index, r.iface))
}

/// Разбор `netstat -rn -f inet` (macOS). Колонки: Destination Gateway Flags Netif.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_netstat(text: &str, index_of: impl Fn(&str) -> u32) -> Vec<DefaultRoute> {
    // ppp не туннель: так выглядит и PPPoE домашнего провайдера.
    const TUN: &[&str] = &["utun", "ipsec", "tun", "tap"];
    text.lines()
        .enumerate()
        .filter_map(|(n, line)| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let dest = *cols.first()?;
            let whole = dest == "default" || dest == "0.0.0.0/0";
            if !whole && !matches!(dest, "0/1" | "128.0/1" | "0.0.0.0/1" | "128.0.0.0/1") {
                return None;
            }
            let gateway = cols.get(2).is_some_and(|f| f.contains('G'));
            let iface = (*cols.get(3)?).to_string();
            let tunnel = TUN.iter().any(|t| iface.starts_with(t));
            Some(DefaultRoute { index: index_of(&iface), tunnel, whole, gateway, metric: n as u32, iface })
        })
        .collect()
}

/// Подстроки адаптеров Windows, которые точно не VPN: виртуальные машины,
/// WSL, системные туннели Windows и наш собственный мост.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const NOT_VPN: &[&str] = &[
    "hyper-v", "vethernet", "wsl", "virtualbox", "vmware", "docker", "loopback", "bluetooth", "wi-fi direct",
    "teredo", "isatap", "6to4", "ip-https", "npcap", "kernel debug", "clod",
];

/// Подстроки, по которым адаптер Windows - туннель VPN. PPPoE домашнего
/// провайдера сюда не попадает.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const TUNNEL_HINTS: &[&str] = &[
    "vpn", "tunnel", "wintun", "tap-", "tun", "ikev2", "l2tp", "pptp", "sstp", "wireguard", "amnezia", "outline",
    "openvpn", "sing-box", "singbox", "hiddify", "nekobox", "v2ray", "xray", "clash", "mihomo", "happ", "nordlynx",
    "warp",
];

/// Адаптер Windows - туннель VPN? ifType 53 (виртуальный) и 131 (туннель)
/// бывают у VPN; виртуальные машины и системные туннели исключаем.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn windows_is_tunnel(name: &str, description: &str, if_type: u32) -> bool {
    let text = format!("{name} {description}").to_lowercase();
    if NOT_VPN.iter().any(|k| text.contains(k)) {
        return false;
    }
    matches!(if_type, 53 | 131) || TUNNEL_HINTS.iter().any(|k| text.contains(k))
}

#[cfg(target_os = "macos")]
mod backend {
    use super::DefaultRoute;

    extern "C" {
        fn if_nametoindex(name: *const std::os::raw::c_char) -> std::os::raw::c_uint;
    }

    fn index_of(name: &str) -> u32 {
        let Ok(c) = std::ffi::CString::new(name) else { return 0 };
        // SAFETY: строка с нулём на конце живёт до конца вызова.
        unsafe { if_nametoindex(c.as_ptr()) }
    }

    pub fn default_routes() -> Vec<DefaultRoute> {
        let Ok(out) = std::process::Command::new("/usr/sbin/netstat").args(["-rn", "-f", "inet"]).output() else {
            return Vec::new();
        };
        super::parse_netstat(&String::from_utf8_lossy(&out.stdout), index_of)
    }
}

#[cfg(target_os = "windows")]
mod backend {
    use super::DefaultRoute;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetAdaptersAddresses, GetIpForwardTable2, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH, MIB_IPFORWARD_TABLE2,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: строки из GetAdaptersAddresses оканчиваются нулём.
        unsafe {
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }

    /// Поднятые адаптеры IPv4: номер, имя, туннель ли.
    fn adapters() -> Vec<(u32, String, bool)> {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        for _ in 0..3 {
            // u64 - выравнивание структуры адаптера.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
            // SAFETY: буфер нужного размера, список живёт, пока жив buf.
            let rc = unsafe { GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), head, &mut size) };
            if rc == 111 {
                // ERROR_BUFFER_OVERFLOW - size уже исправлен, пробуем снова.
                continue;
            }
            if rc != 0 {
                return Vec::new();
            }
            let mut out = Vec::new();
            let mut p = head as *const IP_ADAPTER_ADDRESSES_LH;
            while !p.is_null() {
                // SAFETY: элемент списка внутри buf.
                let a = unsafe { &*p };
                // IfOperStatusUp = 1.
                if a.OperStatus == 1 {
                    let idx = unsafe { a.Anonymous1.Anonymous.IfIndex };
                    let name = wide(a.FriendlyName);
                    let tunnel = super::windows_is_tunnel(&name, &wide(a.Description), a.IfType);
                    out.push((idx, name, tunnel));
                }
                p = a.Next;
            }
            return out;
        }
        Vec::new()
    }

    pub fn default_routes() -> Vec<DefaultRoute> {
        let ads = adapters();
        let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        // SAFETY: таблицу выделяет и освобождает iphlpapi.
        unsafe {
            if GetIpForwardTable2(AF_INET, &mut table) != 0 || table.is_null() {
                return Vec::new();
            }
            let n = (*table).NumEntries as usize;
            let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
            let out = rows
                .iter()
                .filter(|r| r.DestinationPrefix.PrefixLength <= 1)
                .filter_map(|r| {
                    // Маршрут через поднятый адаптер - остальные не в счёт.
                    let (_, name, tunnel) = ads.iter().find(|(i, _, _)| *i == r.InterfaceIndex)?;
                    Some(DefaultRoute {
                        iface: name.clone(),
                        index: r.InterfaceIndex,
                        tunnel: *tunnel,
                        whole: r.DestinationPrefix.PrefixLength == 0,
                        gateway: r.NextHop.Ipv4.sin_addr.S_un.S_addr != 0,
                        metric: r.Metric,
                    })
                })
                .collect();
            FreeMibTable(table as *const _);
            out
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod backend {
    pub fn default_routes() -> Vec<super::DefaultRoute> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(name: &str) -> u32 {
        match name {
            "en0" => 6,
            "en1" => 7,
            _ => 20,
        }
    }

    #[test]
    fn happ_on_mac_keeps_wifi_as_bypass() {
        // Туннель забирает весь интернет половинками, Wi-Fi остаётся маршрутом по умолчанию.
        let t = "Routing tables\n\nInternet:\nDestination        Gateway            Flags               Netif Expire\n\
                 0/1                198.18.0.1         UGScg               utun4\n\
                 default            192.168.1.1        UGScg                 en0\n\
                 127                127.0.0.1          UCS                   lo0\n\
                 128.0/1            198.18.0.1         UGSc                utun4\n";
        let b = pick(&parse_netstat(t, idx)).unwrap();
        assert_eq!(b, Bypass { index: 6, iface: "en0".into(), tunnel: "utun4".into() });
    }

    #[test]
    fn vpn_replacing_default_still_finds_scoped_wifi() {
        // Туннель стал главным default, у Wi-Fi остался привязанный к нему (флаг I).
        let t = "default            link#22            UCSg                utun3\n\
                 default            192.168.0.1        UGScIg                en0\n";
        assert_eq!(pick(&parse_netstat(t, idx)).unwrap().iface, "en0");
    }

    #[test]
    fn no_tunnel_means_nothing_to_bypass() {
        let t = "default            192.168.1.1        UGScg                 en0\n\
                 default            10.0.0.1           UGScIg                en1\n";
        assert_eq!(pick(&parse_netstat(t, idx)), None);
    }

    #[test]
    fn tunnel_without_physical_exit_gives_nothing() {
        // Только туннель: мимо него идти некуда.
        let t = "default            link#22            UCSg                utun3\n";
        assert_eq!(pick(&parse_netstat(t, idx)), None);
    }

    #[test]
    fn windows_routes_pick_lowest_metric_physical() {
        let r = |iface: &str, index, tunnel, whole, gateway, metric| DefaultRoute {
            iface: iface.into(),
            index,
            tunnel,
            whole,
            gateway,
            metric,
        };
        let routes = [
            r("happ-tun", 40, true, false, false, 0),
            r("happ-tun", 40, true, false, false, 0),
            r("Ethernet", 12, false, true, true, 25),
            r("Wi-Fi", 9, false, true, true, 35),
        ];
        assert_eq!(pick(&routes), Some(Bypass { index: 12, iface: "Ethernet".into(), tunnel: "happ-tun".into() }));
    }

    #[test]
    fn windows_tunnel_classification() {
        assert!(windows_is_tunnel("happ-tun", "Happ Tunnel", 53));
        assert!(windows_is_tunnel("singbox_tun", "Wintun Userspace Tunnel", 53));
        assert!(windows_is_tunnel("wg0", "WireGuard Tunnel", 53));
        assert!(!windows_is_tunnel("Ethernet", "Realtek PCIe GbE Family Controller", 6));
        assert!(!windows_is_tunnel("Wi-Fi", "Intel(R) Wi-Fi 6 AX201 160MHz", 71));
        assert!(!windows_is_tunnel("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter", 6));
        assert!(!windows_is_tunnel("Teredo Tunneling Pseudo-Interface", "Teredo", 131));
    }

    /// Живая таблица этой машины: cargo test live_routes -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_routes() {
        for r in backend::default_routes() {
            println!("{r:?}");
        }
        println!("bypass: {:?}", detect());
    }
}
