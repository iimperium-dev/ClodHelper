//! VPN рядом с мостом: включён ли туннель, который забирает весь трафик.
//!
//! Первый совет в FAQ - «выключите VPN»: мост ходит к прокси через него, и
//! прокси «не отвечает» или тормозит. Здесь только распознавание, без обхода.
//!
//! Правило без ложных тревог: VPN - это поднятый адаптер-туннель, через
//! который идёт маршрут по умолчанию (0.0.0.0/0 или пара 0/1 + 128/1, как у
//! WireGuard). Туннель без такого маршрута (корпоративный split-tunnel) не
//! мешает и не считается. Виртуальные карты Hyper-V, WSL, VirtualBox, VMware,
//! Docker и системные туннели Windows (Teredo, IP-HTTPS) - не VPN.
//!
//! Отдельно - SOTA VPN: его служба временами стирает чужие HTTP(S)_PROXY,
//! и терминал с Claude Code молча идут мимо моста.

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct VpnStatus {
    /// Включён туннель с маршрутом по умолчанию.
    pub active: bool,
    /// Как называется VPN («WireGuard»); None - не узнали.
    pub name: Option<String>,
    /// Запущен SOTA VPN (служба sota-daemon).
    pub sota: bool,
}

/// Известные VPN-программы: подстрока (в нижнем регистре) в имени адаптера,
/// его описании или имени процесса - и как её назвать человеку.
const BRANDS: &[(&str, &str)] = &[
    ("wireguard", "WireGuard"),
    ("amnezia", "AmneziaVPN"),
    ("outline", "Outline"),
    ("openvpn", "OpenVPN"),
    ("sing-box", "sing-box"),
    ("singbox", "sing-box"),
    ("hiddify", "Hiddify"),
    ("nekobox", "NekoBox"),
    ("nekoray", "NekoBox"),
    ("v2rayn", "v2rayN"),
    ("v2ray", "v2rayN"),
    ("xray", "Xray"),
    ("clash", "Clash"),
    ("mihomo", "Clash"),
    ("happ", "Happ"),
    ("nordlynx", "NordVPN"),
    ("nordvpn", "NordVPN"),
    ("protonvpn", "Proton VPN"),
    ("proton vpn", "Proton VPN"),
    ("expressvpn", "ExpressVPN"),
    ("surfshark", "Surfshark"),
    ("windscribe", "Windscribe"),
    ("mullvad", "Mullvad"),
    ("anyconnect", "Cisco AnyConnect"),
    ("forticlient", "FortiClient"),
    ("fortinet", "FortiClient"),
    ("kaspersky", "Kaspersky VPN"),
    ("psiphon", "Psiphon"),
    ("tunnelbear", "TunnelBear"),
    ("hotspot shield", "Hotspot Shield"),
    ("cloudflare warp", "Cloudflare WARP"),
    ("cloudflarewarp", "Cloudflare WARP"),
    ("warp-svc", "Cloudflare WARP"),
    ("sota", "SOTA VPN"),
    // Драйвер TAP ставят и другие программы (Outline) - их имя выше.
    ("tap-windows", "OpenVPN"),
];

/// Подстроки адаптеров, которые точно не VPN: виртуальные машины, WSL,
/// системные туннели Windows и наш собственный мост.
/// Разбор адаптеров нужен только на Windows (на Mac - таблица маршрутов).
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const NOT_VPN: &[&str] = &[
    "hyper-v",
    "vethernet",
    "wsl",
    "virtualbox",
    "vmware",
    "docker",
    "loopback",
    "bluetooth",
    "wi-fi direct",
    "teredo",
    "isatap",
    "6to4",
    "ip-https",
    "npcap",
    "kernel debug",
    "clod",
];

/// Подстроки, по которым адаптер похож на туннель, даже без известного имени.
// Встроенный VPN Windows: «WAN Miniport (IKEv2/L2TP/PPTP/SSTP)». PPPoE -
// это домашний интернет у многих провайдеров, его не трогаем.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const TUNNEL_HINTS: &[&str] = &["vpn", "tunnel", "wintun", "tap-", "tun", "ikev2", "l2tp", "pptp", "sstp"];

/// Типы интерфейсов Windows (IANA ifType), которые бывают у VPN. PPP (23) не
/// берём: так выглядит и PPPoE домашнего провайдера.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const IF_TYPE_PROP_VIRTUAL: u32 = 53;
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const IF_TYPE_TUNNEL: u32 = 131;

fn brand_of(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    BRANDS.iter().find(|(k, _)| t.contains(k)).map(|(_, v)| *v)
}

/// Сетевой адаптер - то, что нужно для решения «VPN или нет».
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub if_type: u32,
    pub up: bool,
    /// Через адаптер идёт маршрут по умолчанию (/0 или /1).
    pub default_route: bool,
}

/// Найти включённый VPN среди адаптеров. Some(имя или пусто) - есть.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub fn classify(adapters: &[Adapter]) -> Option<Option<String>> {
    adapters.iter().find_map(|a| {
        if !a.up || !a.default_route {
            return None;
        }
        let text = format!("{} {}", a.name, a.description).to_lowercase();
        if NOT_VPN.iter().any(|k| text.contains(k)) {
            return None;
        }
        let brand = brand_of(&text);
        let tunnel_type = matches!(a.if_type, IF_TYPE_PROP_VIRTUAL | IF_TYPE_TUNNEL);
        let looks_tunnel = TUNNEL_HINTS.iter().any(|k| text.contains(k));
        (brand.is_some() || tunnel_type || looks_tunnel).then(|| brand.map(str::to_string))
    })
}

/// Имя VPN по запущенным процессам (когда адаптер безымянный, например
/// Wintun) и запущен ли SOTA VPN.
fn from_processes(names: &[String]) -> (Option<String>, bool) {
    let sota = names.iter().any(|n| n.to_lowercase().starts_with("sota"));
    let brand = names.iter().find_map(|n| brand_of(n)).map(str::to_string);
    (brand, sota)
}

fn process_names() -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().trim_end_matches(".exe").to_string())
        .collect()
}

/// Проверить сейчас (блокирующий вызов: на Mac запускает netstat).
pub fn status() -> VpnStatus {
    let tunnel = backend::tunnel();
    let procs = process_names();
    let (proc_brand, sota) = from_processes(&procs);
    match tunnel {
        Some(name) => VpnStatus {
            active: true,
            // Адаптер честнее процесса: процесс может быть запущен, но не
            // подключён. Имя по процессу - только если адаптер безымянный.
            name: name.or(proc_brand),
            sota,
        },
        None => VpnStatus { active: false, name: None, sota },
    }
}

#[cfg(target_os = "windows")]
mod backend {
    use super::{classify, Adapter};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetAdaptersAddresses, GetIpForwardTable2, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
        MIB_IPFORWARD_TABLE2,
    };
    use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

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

    /// Номера интерфейсов, через которые идёт маршрут по умолчанию.
    fn default_route_ifaces() -> Vec<u32> {
        let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        // SAFETY: таблицу выделяет и освобождает iphlpapi.
        unsafe {
            if GetIpForwardTable2(AF_UNSPEC, &mut table) != 0 || table.is_null() {
                return Vec::new();
            }
            let n = (*table).NumEntries as usize;
            let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
            let out = rows
                .iter()
                .filter(|r| r.DestinationPrefix.PrefixLength <= 1)
                .map(|r| r.InterfaceIndex)
                .collect();
            FreeMibTable(table as *const _);
            out
        }
    }

    fn adapters() -> Vec<Adapter> {
        let routes = default_route_ifaces();
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        for _ in 0..3 {
            // u64 - выравнивание структуры адаптера.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
            // SAFETY: буфер нужного размера, список живёт, пока жив buf.
            let rc = unsafe { GetAdaptersAddresses(AF_UNSPEC as u32, flags, std::ptr::null(), head, &mut size) };
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
                let idx4 = unsafe { a.Anonymous1.Anonymous.IfIndex };
                out.push(Adapter {
                    name: wide(a.FriendlyName),
                    description: wide(a.Description),
                    if_type: a.IfType,
                    // IfOperStatusUp = 1.
                    up: a.OperStatus == 1,
                    default_route: routes.iter().any(|&r| r != 0 && (r == idx4 || r == a.Ipv6IfIndex)),
                });
                p = a.Next;
            }
            return out;
        }
        Vec::new()
    }

    pub fn tunnel() -> Option<Option<String>> {
        classify(&adapters())
    }
}

#[cfg(target_os = "macos")]
mod backend {
    /// Туннель из таблицы маршрутов: интерфейс utun/ipsec/tun/tap, через
    /// который идёт default или 0/1, 128.0/1.
    pub fn tunnel() -> Option<Option<String>> {
        let out = std::process::Command::new("/usr/sbin/netstat").args(["-rn", "-f", "inet"]).output().ok()?;
        super::mac_tunnel(&String::from_utf8_lossy(&out.stdout)).map(|_| None)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod backend {
    pub fn tunnel() -> Option<Option<String>> {
        None
    }
}

/// Разбор `netstat -rn -f inet` на Mac: имя туннеля с маршрутом по умолчанию.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn mac_tunnel(netstat: &str) -> Option<String> {
    // ppp не берём: это может быть PPPoE провайдера.
    const TUN: &[&str] = &["utun", "ipsec", "tun", "tap"];
    netstat.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let dest = *cols.first()?;
        if !matches!(dest, "default" | "0/1" | "128.0/1" | "0.0.0.0/1" | "128.0.0.0/1") {
            return None;
        }
        // Интерфейс - четвёртая колонка (Destination Gateway Flags Netif).
        let iface = *cols.get(3)?;
        TUN.iter().any(|t| iface.starts_with(t)).then(|| iface.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(name: &str, desc: &str, if_type: u32, route: bool) -> Adapter {
        Adapter { name: name.into(), description: desc.into(), if_type, up: true, default_route: route }
    }

    #[test]
    fn wireguard_with_default_route_is_vpn() {
        let list = [ad("Wi-Fi", "Intel(R) Wi-Fi 6 AX201", 71, true), ad("wg0", "WireGuard Tunnel", 53, true)];
        assert_eq!(classify(&list), Some(Some("WireGuard".into())));
    }

    #[test]
    fn plain_wifi_and_ethernet_are_not_vpn() {
        let list = [ad("Wi-Fi", "Intel(R) Wi-Fi 6 AX201", 71, true), ad("Ethernet", "Realtek PCIe GbE", 6, true)];
        assert_eq!(classify(&list), None);
    }

    #[test]
    fn hyper_v_and_wsl_are_not_vpn() {
        let list = [
            ad("vEthernet (Default Switch)", "Hyper-V Virtual Ethernet Adapter", 6, true),
            ad("vEthernet (WSL (Hyper-V firewall))", "Hyper-V Virtual Ethernet Adapter #2", 6, true),
            ad("Teredo Tunneling Pseudo-Interface", "Teredo Tunneling Pseudo-Interface", 131, true),
            ad("VirtualBox Host-Only Network", "VirtualBox Host-Only Ethernet Adapter", 6, true),
        ];
        assert_eq!(classify(&list), None);
    }

    #[test]
    fn tunnel_without_default_route_is_not_vpn() {
        // Корпоративный split-tunnel: адаптер есть, весь трафик не забирает.
        let list = [ad("OpenVPN TAP-Windows6", "TAP-Windows Adapter V9", 6, false)];
        assert_eq!(classify(&list), None);
    }

    #[test]
    fn down_adapter_is_not_vpn() {
        let mut a = ad("Outline-TAP", "TAP-Windows Adapter V9 for Outline", 6, true);
        a.up = false;
        assert_eq!(classify(&[a]), None);
    }

    #[test]
    fn nameless_wintun_is_vpn_without_name() {
        let list = [ad("tun0", "Wintun Userspace Tunnel", 53, true)];
        assert_eq!(classify(&list), Some(None));
    }

    #[test]
    fn sing_box_and_outline_get_names() {
        assert_eq!(classify(&[ad("sing-box", "sing-box Tunnel", 53, true)]), Some(Some("sing-box".into())));
        assert_eq!(classify(&[ad("Outline-TAP", "TAP-Windows Adapter V9 for Outline", 6, true)]), Some(Some("Outline".into())));
    }

    #[test]
    fn pppoe_internet_is_not_vpn_but_builtin_vpn_is() {
        assert_eq!(classify(&[ad("Ростелеком", "WAN Miniport (PPPOE)", 23, true)]), None);
        assert_eq!(classify(&[ad("Работа", "WAN Miniport (IKEv2)", 23, true)]), Some(None));
    }

    #[test]
    fn own_bridge_adapter_is_not_vpn() {
        assert_eq!(classify(&[ad("Clod Helper", "Clod Bridge Tunnel", 53, true)]), None);
    }

    #[test]
    fn processes_name_vpn_and_find_sota() {
        let (brand, sota) = from_processes(&["explorer".into(), "sota-daemon".into()]);
        assert_eq!(brand.as_deref(), Some("SOTA VPN"));
        assert!(sota);
        let (brand, sota) = from_processes(&["chrome".into(), "AmneziaVPN".into()]);
        assert_eq!(brand.as_deref(), Some("AmneziaVPN"));
        assert!(!sota);
        assert_eq!(from_processes(&["chrome".into()]), (None, false));
    }

    #[test]
    fn mac_utun_default_route_is_vpn() {
        let full = "Routing tables\n\nInternet:\nDestination        Gateway            Flags               Netif Expire\n0/1                10.2.0.1           UGScg               utun4\ndefault            192.168.1.1        UGScg                 en0\n128.0/1            10.2.0.1           UGSc                utun4\n";
        assert_eq!(mac_tunnel(full).as_deref(), Some("utun4"));
        let default_utun = "Destination  Gateway  Flags  Netif\ndefault      link#22  UCSg   utun3\n";
        assert_eq!(mac_tunnel(default_utun).as_deref(), Some("utun3"));
    }

    /// Живая проверка на этом компьютере: `cargo test --lib vpn::tests::real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_machine() {
        println!("{:?}", status());
    }

    #[test]
    fn mac_system_utun_without_default_route_is_not_vpn() {
        // utun0-3 есть у macOS всегда (iCloud, Handoff), но без маршрута по умолчанию.
        let normal = "Destination  Gateway  Flags  Netif\ndefault      192.168.1.1  UGScg  en0\n169.254      link#6   UCS    en0\nfe80::%utun0 fe80::1  UcI    utun0\n";
        assert_eq!(mac_tunnel(normal), None);
    }
}
