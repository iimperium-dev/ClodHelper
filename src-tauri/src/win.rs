//! Windows: быстрые нативные запросы вместо PowerShell.
//!
//! Каждый запуск powershell.exe стоит 1–2 секунды (загрузка, проверка
//! антивирусом), а статусы опрашиваются постоянно — интерфейс «грузился»
//! секундами. Здесь то же самое за микро- и миллисекунды:
//!   - TCP-соединения с PID (GetExtendedTcpTable) — статусы «через мост»;
//!   - реестр (winreg): политики Claude и браузеров, правила брандмауэра,
//!     пакеты Store, обработчик ссылок tg://;
//!   - иконки exe в PNG (SHDefExtractIcon + GDI).
//! PowerShell остаётся только для действий с правами администратора.

use std::path::{Path, PathBuf};

use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
use winreg::RegKey;

// ─── TCP-соединения ─────────────────────────────────────────────

/// Установленные TCP-соединения: (PID, удалённый адрес «ip:порт» или
/// «[ipv6]:порт» — как в netstat).
pub fn established_tcp() -> Vec<(u32, String)> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_TCP_STATE_ESTAB,
        TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    let mut out = Vec::new();
    for family in [AF_INET, AF_INET6] {
        // Таблица может вырасти между вызовами — несколько попыток с запасом.
        let mut buf: Vec<u32> = Vec::new();
        let mut size: u32 = 0;
        let mut ok = false;
        for _ in 0..4 {
            buf.resize(size as usize / 4 + 256, 0);
            let mut cap = (buf.len() * 4) as u32;
            // SAFETY: буфер выровнен по u32 и длиной cap байт.
            let rc = unsafe {
                GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut cap, 0, family as u32, TCP_TABLE_OWNER_PID_ALL, 0)
            };
            if rc == 0 {
                ok = true;
                break;
            }
            size = cap;
        }
        if !ok {
            continue;
        }
        let count = buf[0] as usize;
        let estab = MIB_TCP_STATE_ESTAB as u32;
        if family == AF_INET {
            // SAFETY: после dwNumEntries идут count записей таблицы.
            let rows = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().add(1) as *const MIB_TCPROW_OWNER_PID, count)
            };
            for r in rows.iter().filter(|r| r.dwState == estab) {
                let ip = std::net::Ipv4Addr::from(r.dwRemoteAddr.to_ne_bytes());
                out.push((r.dwOwningPid, format!("{ip}:{}", port(r.dwRemotePort))));
            }
        } else {
            // SAFETY: то же для IPv6.
            let rows = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().add(1) as *const MIB_TCP6ROW_OWNER_PID, count)
            };
            for r in rows.iter().filter(|r| r.dwState == estab) {
                let ip = std::net::Ipv6Addr::from(r.ucRemoteAddr);
                out.push((r.dwOwningPid, format!("[{ip}]:{}", port(r.dwRemotePort))));
            }
        }
    }
    out
}

/// Порт в таблице — в сетевом порядке байт в младших 16 битах.
fn port(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

// ─── Реестр ─────────────────────────────────────────────────────

pub use crate::regval::RegValue;

pub fn hklm() -> RegKey {
    RegKey::predef(HKEY_LOCAL_MACHINE)
}

pub fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

/// 64-битное представление — как у PowerShell-скриптов (Registry64).
fn open(root: &RegKey, path: &str) -> std::io::Result<RegKey> {
    root.open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY)
}

/// Значение без раскрытия переменных окружения (как DoNotExpandEnvironmentNames).
pub fn value(root: &RegKey, path: &str, name: &str) -> RegValue {
    use winreg::enums::RegType;
    let Ok(key) = open(root, path) else { return RegValue::Absent };
    let Ok(raw) = key.get_raw_value(name) else { return RegValue::Absent };
    match raw.vtype {
        RegType::REG_SZ => RegValue::Str(utf16_z(&raw.bytes)),
        other => RegValue::Other(format!("{other:?}")),
    }
}

/// Строковое значение (REG_SZ / REG_EXPAND_SZ — как есть) или None.
pub fn string(root: &RegKey, path: &str, name: &str) -> Option<String> {
    open(root, path).ok()?.get_value::<String, _>(name).ok()
}

fn utf16_z(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&w| w != 0)
        .collect();
    String::from_utf16_lossy(&words)
}

/// Правило брандмауэра Windows по имени (InstanceID) — строка вида
/// «v2.33|Action=Block|Active=TRUE|Dir=Out|…». Читается без прав
/// администратора. Err — нет доступа к хранилищу правил.
pub fn firewall_rule(name: &str) -> Result<Option<String>, std::io::Error> {
    let key = open(
        &hklm(),
        r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules",
    )?;
    Ok(key.get_value::<String, _>(name).ok())
}

/// Установленные программы из «Приложений и возможностей» (ключи Uninstall
/// пользователя и машины, включая 32-битные): (имя, DisplayIcon). Системные
/// компоненты и обновления пропускаем.
pub fn uninstall_entries() -> Vec<(String, String)> {
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    const UNINSTALL_32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut out = Vec::new();
    for (root, path) in [(hkcu(), UNINSTALL), (hklm(), UNINSTALL), (hklm(), UNINSTALL_32)] {
        let Ok(list) = open(&root, path) else { continue };
        for sub in list.enum_keys().flatten() {
            let Ok(app) = list.open_subkey(&sub) else { continue };
            if app.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1
                || app.get_value::<String, _>("ParentKeyName").is_ok()
            {
                continue;
            }
            if let (Ok(name), Ok(icon)) = (
                app.get_value::<String, _>("DisplayName"),
                app.get_value::<String, _>("DisplayIcon"),
            ) {
                out.push((name, icon));
            }
        }
    }
    out
}

/// Пакет Store текущего пользователя.
#[derive(Debug, Clone)]
pub struct StorePackage {
    /// PackageFullName: Имя_Версия_Архитектура_Ресурс_Издатель.
    pub full_name: String,
    pub root: PathBuf,
}

impl StorePackage {
    /// PackageFamilyName: Имя_Издатель.
    pub fn family(&self) -> String {
        let parts: Vec<&str> = self.full_name.split('_').collect();
        format!("{}_{}", parts.first().unwrap_or(&""), parts.last().unwrap_or(&""))
    }
}

/// Установленные пакеты Store с этим именем (новые версии первыми) — из
/// репозитория пакетов пользователя в реестре, без Get-AppxPackage.
pub fn store_packages(name: &str) -> Vec<StorePackage> {
    let Ok(repo) = open(
        &hkcu(),
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
    ) else {
        return Vec::new();
    };
    let prefix = format!("{name}_");
    let mut found: Vec<(Vec<u32>, StorePackage)> = repo
        .enum_keys()
        .flatten()
        .filter(|k| k.starts_with(&prefix))
        .filter_map(|full_name| {
            let root: String = repo.open_subkey(&full_name).ok()?.get_value("PackageRootFolder").ok()?;
            let version = full_name.split('_').nth(1).unwrap_or_default();
            let key = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
            Some((key, StorePackage { full_name, root: PathBuf::from(root) }))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Приложение пакета из AppxManifest.xml: (Executable, Id) первого
/// Application, чей exe называется `exe_name`.
pub fn manifest_app(root: &Path, exe_name: &str) -> Option<(String, String)> {
    let xml = std::fs::read_to_string(root.join("AppxManifest.xml")).ok()?;
    manifest_app_in(&xml, exe_name)
}

pub(crate) fn manifest_app_in(xml: &str, exe_name: &str) -> Option<(String, String)> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(tag[start..].split('"').next()?.to_string())
    };
    xml.split("<Application ").skip(1).find_map(|rest| {
        // Пробел впереди — чтобы и первый атрибут искался как « Имя="».
        let tag = format!(" {}", rest.split('>').next()?);
        let tag = tag.as_str();
        let exe = attr(tag, "Executable")?;
        let leaf = exe.rsplit(['\\', '/']).next()?;
        if !leaf.eq_ignore_ascii_case(exe_name) {
            return None;
        }
        Some((exe.clone(), attr(tag, "Id")?))
    })
}

/// Описание файла из ресурса версии (FileDescription): у Telegram и его
/// копий под другим именем (второй аккаунт) — «Telegram Desktop».
pub fn file_description(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: строка с нулём на конце; буфер нужного размера.
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    // Значение из блока версии: указатель внутрь `data` и длина.
    let query = |sub: &str| -> Option<(*const u16, usize)> {
        let key: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        // SAFETY: data — блок версии от GetFileVersionInfoW, ключ с нулём на конце.
        let ok = unsafe { VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut ptr, &mut len) };
        (ok != 0 && !ptr.is_null() && len > 0).then_some((ptr as *const u16, len as usize))
    };
    // Языки строк файла (пары «язык, кодовая страница»), плюс обычные запасные.
    let mut languages: Vec<(u16, u16)> = Vec::new();
    if let Some((ptr, bytes)) = query(r"\VarFileInfo\Translation") {
        // SAFETY: bytes байт внутри data, по два u16 на пару.
        let words = unsafe { std::slice::from_raw_parts(ptr, bytes / 2) };
        languages.extend(words.chunks_exact(2).map(|w| (w[0], w[1])));
    }
    languages.extend([(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)]);
    languages.into_iter().find_map(|(lang, cp)| {
        let (ptr, chars) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\FileDescription"))?;
        // SAFETY: chars символов UTF-16 внутри data.
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, chars) });
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_string())
    })
}

// ─── Окна ───────────────────────────────────────────────────────

/// Разрешить запускаемой программе вывести своё окно вперёд. Окно, которое
/// пользователь только что нажал, — наше, и право на передний план есть у
/// нас; без передачи Windows может не пустить вперёд уже запущенную
/// программу (только мигнёт её кнопкой на панели задач).
pub fn allow_foreground() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    // SAFETY: без указателей; отказ (мы не на переднем плане) безвреден.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// Окна верхнего уровня этих процессов, и скрытые тоже: у программы,
/// свёрнутой в трей, главное окно спрятано.
fn top_windows(pids: &[u32]) -> Vec<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId};
    struct Ctx<'a> {
        pids: &'a [u32],
        found: Vec<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
        // SAFETY: lparam — указатель на Ctx, живой на время EnumWindows.
        let ctx = unsafe { &mut *(lparam as *mut Ctx) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if ctx.pids.contains(&pid) {
            ctx.found.push(hwnd);
        }
        1
    }
    let mut ctx = Ctx { pids, found: Vec::new() };
    // SAFETY: колбэк не переживает вызов, Ctx живёт до его конца.
    unsafe { EnumWindows(Some(each), &mut ctx as *mut Ctx as LPARAM) };
    ctx.found
}

/// Попросить программу закрыться так же, как это делает Windows, когда
/// закрывает программы для обновления (Restart Manager): WM_QUERYENDSESSION,
/// затем WM_ENDSESSION с ENDSESSION_CLOSEAPP — всем её окнам, и скрытым.
/// В ответ программа сохраняет состояние целиком: Chrome — сессию со всеми
/// окнами (WM_CLOSE по окнам оставлял в ней только последнее закрытое),
/// Telegram выходит, а не прячется в трей. false — программа отказалась
/// закрываться: ответила FALSE, зависла или не ответила за 3 секунды
/// (спрашивает человека о несохранённом).
pub fn end_session(pids: &[u32]) -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_TIMEOUT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, ENDSESSION_CLOSEAPP, SMTO_ABORTIFHUNG, WM_ENDSESSION, WM_QUERYENDSESSION,
    };
    let windows = top_windows(pids);
    for &window in &windows {
        let mut answer: usize = 1;
        // SAFETY: сообщение без указателей; зависшее окно — отказ без ожидания.
        let sent = unsafe {
            SendMessageTimeoutW(
                window,
                WM_QUERYENDSESSION,
                0,
                ENDSESSION_CLOSEAPP as isize,
                SMTO_ABORTIFHUNG,
                3000,
                &mut answer,
            )
        };
        // Окно, исчезнувшее после перечисления, — не отказ.
        let refused = if sent == 0 {
            // SAFETY: просто код ошибки этого потока.
            let error = unsafe { GetLastError() };
            error == ERROR_TIMEOUT
        } else {
            answer == 0
        };
        if refused {
            return false;
        }
    }
    for &window in &windows {
        let mut answer: usize = 0;
        // SAFETY: как выше; окно, чей процесс уже вышел, — просто ошибка.
        unsafe {
            SendMessageTimeoutW(
                window,
                WM_ENDSESSION,
                1,
                ENDSESSION_CLOSEAPP as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut answer,
            )
        };
    }
    true
}

// ─── Иконки ─────────────────────────────────────────────────────

/// Иконка exe размером `size`×`size` в PNG (RGBA) — из ресурсов файла, без
/// PowerShell. None — у файла нет своей иконки.
pub fn exe_icon_png(path: &str, size: u32) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::Shell::SHDefExtractIconW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut icon: HICON = std::ptr::null_mut();
    // SAFETY: строка с нулём на конце; нужна только большая иконка.
    let hr = unsafe { SHDefExtractIconW(wide.as_ptr(), 0, 0, &mut icon, std::ptr::null_mut(), size) };
    if hr != 0 || icon.is_null() {
        return None;
    }
    let mut info = ICONINFO::default();
    // SAFETY: icon — действительная иконка; битмапы освобождаются ниже.
    let pixels = unsafe {
        let got = if GetIconInfo(icon, &mut info) != 0 && !info.hbmColor.is_null() {
            let mut bm: BITMAP = std::mem::zeroed();
            GetObjectW(info.hbmColor, std::mem::size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast());
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let read = |hbm, buf: &mut Vec<u8>| {
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // сверху вниз
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                };
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                let lines = GetDIBits(dc, hbm, 0, h as u32, buf.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
                DeleteDC(dc);
                lines == h
            };
            let mut color = vec![0u8; (w * h * 4).max(0) as usize];
            if w > 0 && h > 0 && read(info.hbmColor, &mut color) {
                // Старые иконки без альфа-канала: прозрачность — из маски.
                if color.chunks_exact(4).all(|p| p[3] == 0) {
                    let mut mask = vec![0u8; color.len()];
                    if !info.hbmMask.is_null() && read(info.hbmMask, &mut mask) {
                        for (c, m) in color.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                            c[3] = if m[0] == 0 { 255 } else { 0 };
                        }
                    } else {
                        color.chunks_exact_mut(4).for_each(|c| c[3] = 255);
                    }
                }
                Some((w as u32, h as u32, color))
            } else {
                None
            }
        } else {
            None
        };
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        DestroyIcon(icon);
        got
    };
    let (w, h, mut bgra) = pixels?;
    bgra.chunks_exact_mut(4).for_each(|p| p.swap(0, 2)); // BGRA → RGBA
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&bgra).ok()?;
    }
    Some(png_bytes)
}

// ─── Текст ──────────────────────────────────────────────────────

/// Строка в системной ANSI-кодировке (старые поля ярлыков) → Unicode.
pub fn ansi_to_string(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).to_string();
    }
    let mut wide = vec![0u16; bytes.len() * 2];
    // SAFETY: длины переданы явно, буфер с запасом.
    let n = unsafe {
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), wide.len() as i32)
    };
    String::from_utf16_lossy(&wide[..n.max(0) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_network_order() {
        // Порт 8890 (0x22BA) лежит в памяти сетевым порядком: байты 0x22, 0xBA.
        assert_eq!(port(u32::from(u16::from_ne_bytes([0x22, 0xBA]))), 8890);
    }

    #[test]
    fn package_family_and_manifest_app() {
        let p = StorePackage {
            full_name: "AnthropicPBC.Claude_2.42304.0.0_x64__fnn82j28hfe8t".into(),
            root: PathBuf::new(),
        };
        assert_eq!(p.family(), "AnthropicPBC.Claude_fnn82j28hfe8t");
        let xml = r#"<Package><Applications>
            <Application Id="Helper" Executable="app\helper.exe" EntryPoint="x"/>
            <Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
            </Application></Applications></Package>"#;
        assert_eq!(manifest_app_in(xml, "claude.exe"), Some(("app\\Claude.exe".into(), "Claude".into())));
        assert_eq!(manifest_app_in(xml, "ChatGPT.exe"), None);
    }

    /// Живая проверка: cargo test live_win -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_win() {
        let t = std::time::Instant::now();
        let conns = established_tcp();
        println!("tcp: {} соединений за {:?}", conns.len(), t.elapsed());
        println!("claude: {:?}", store_packages("AnthropicPBC.Claude"));
        println!("firewall sample: {:?}", firewall_rule("NETDIS-UPnPHost-Out-TCP"));
        let t = std::time::Instant::now();
        let png = exe_icon_png("C:\\Windows\\explorer.exe", 64);
        println!("icon: {:?} байт за {:?}", png.as_ref().map(Vec::len), t.elapsed());
    }
}
