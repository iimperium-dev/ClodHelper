//! Ярлыки Windows (.lnk, формат MS-SHLLINK) без COM и PowerShell: что
//! запускает ярлык — для поиска программ в меню «Пуск». Разбор занимает
//! микросекунды; WScript.Shell через PowerShell — секунды на каждый запрос.
//!
//! Цель ярлыка, по убыванию надёжности:
//!   1. блок переменных окружения («%ProgramFiles%\…», всегда UTF-16);
//!   2. LinkInfo — локальный путь (UTF-16, если есть, иначе ANSI);
//!   3. относительный путь от папки ярлыка.

use std::path::{Component, Path, PathBuf};

/// Что запускает ярлык.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shortcut {
    pub target: String,
    pub args: String,
}

const HAS_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;
/// EnvironmentVariableDataBlock.
const ENV_BLOCK: u32 = 0xA000_0001;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn utf16(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&w| w != 0)
        .collect();
    String::from_utf16_lossy(&words)
}

/// Строка с нулём на конце в системной ANSI-кодировке.
fn cstr_ansi(d: &[u8], at: usize) -> Option<String> {
    let bytes = d.get(at..)?;
    let end = bytes.iter().position(|&b| b == 0)?;
    Some(decode_ansi(&bytes[..end]))
}

fn cstr_utf16(d: &[u8], at: usize) -> Option<String> {
    Some(utf16(d.get(at..)?))
}

#[cfg(target_os = "windows")]
fn decode_ansi(bytes: &[u8]) -> String {
    crate::win::ansi_to_string(bytes)
}

#[cfg(not(target_os = "windows"))]
fn decode_ansi(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Разобрать ярлык; `dir` — его папка (для относительного пути цели).
pub fn parse(d: &[u8], dir: &Path) -> Option<Shortcut> {
    if u32_at(d, 0)? != 0x4C {
        return None;
    }
    let flags = u32_at(d, 0x14)?;
    let mut pos = 0x4C;
    if flags & HAS_ID_LIST != 0 {
        pos += 2 + u16_at(d, pos)? as usize;
    }
    let mut link_target = None;
    if flags & HAS_LINK_INFO != 0 {
        let li = pos;
        let size = u32_at(d, li)? as usize;
        let header = u32_at(d, li + 4)?;
        // VolumeIDAndLocalBasePath: путь на локальном диске.
        if u32_at(d, li + 8)? & 1 != 0 {
            let (base, suffix) = if header >= 0x24 {
                (
                    cstr_utf16(d, li + u32_at(d, li + 28)? as usize),
                    cstr_utf16(d, li + u32_at(d, li + 32)? as usize),
                )
            } else {
                (
                    cstr_ansi(d, li + u32_at(d, li + 16)? as usize),
                    cstr_ansi(d, li + u32_at(d, li + 24)? as usize),
                )
            };
            link_target = base
                .filter(|b| !b.is_empty())
                .map(|b| format!("{b}{}", suffix.unwrap_or_default()));
        }
        pos = li + size;
    }
    // StringData: имя, относительный путь, рабочая папка, аргументы, иконка.
    let unicode = flags & IS_UNICODE != 0;
    let mut strings: [Option<String>; 5] = Default::default();
    for (i, flag) in [HAS_NAME, HAS_RELATIVE_PATH, HAS_WORKING_DIR, HAS_ARGUMENTS, HAS_ICON_LOCATION]
        .into_iter()
        .enumerate()
    {
        if flags & flag == 0 {
            continue;
        }
        let count = u16_at(d, pos)? as usize;
        pos += 2;
        let len = if unicode { count * 2 } else { count };
        let bytes = d.get(pos..pos + len)?;
        pos += len;
        strings[i] = Some(if unicode { utf16(bytes) } else { decode_ansi(bytes) });
    }
    // ExtraData: цель с переменными окружения (TargetUnicode — после
    // 260 байт TargetAnsi).
    let mut env_target = None;
    while let Some(size) = u32_at(d, pos) {
        if size < 8 {
            break;
        }
        if u32_at(d, pos + 4) == Some(ENV_BLOCK) {
            env_target = d
                .get(pos + 8 + 260..pos + size as usize)
                .map(utf16)
                .filter(|s| !s.is_empty())
                .or_else(|| cstr_ansi(d, pos + 8).filter(|s| !s.is_empty()));
        }
        pos += size as usize;
    }
    let target = env_target
        .map(|t| expand_env(&t))
        .or(link_target)
        .or_else(|| strings[1].as_ref().map(|rel| normalize(&dir.join(rel))))?;
    Some(Shortcut {
        target,
        args: strings[3].clone().unwrap_or_default(),
    })
}

/// «%ПЕРЕМЕННАЯ%» → значение из окружения; неизвестные остаются как есть.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            out.push_str(after);
            return out;
        };
        let name = &after[..end];
        match std::env::var(name) {
            Ok(v) if !name.is_empty() => out.push_str(&v),
            _ => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// «…\Programs\..\..\App\x.exe» → «…\App\x.exe» без обращения к диску.
fn normalize(p: &Path) -> String {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Синтетический ярлык: заголовок, необязательный LinkInfo (ANSI или
    /// UTF-16), строки StringData и блок переменных окружения.
    fn lnk(link_info: Option<(&str, bool)>, args: Option<&str>, env: Option<&str>) -> Vec<u8> {
        let mut flags = IS_UNICODE;
        if link_info.is_some() {
            flags |= HAS_LINK_INFO;
        }
        if args.is_some() {
            flags |= HAS_ARGUMENTS;
        }
        let mut d = vec![0u8; 0x4C];
        d[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        d[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        if let Some((path, unicode)) = link_info {
            let header: u32 = if unicode { 0x24 } else { 0x1C };
            let mut body = Vec::new();
            let base_off = header as usize;
            if unicode {
                // ANSI-поля пустые, пути — в UTF-16 после них.
                let ansi_base = base_off;
                let ansi_suffix = ansi_base + 1;
                let uni_base = ansi_suffix + 1;
                let wide: Vec<u8> = path.encode_utf16().chain(Some(0)).flat_map(|w| w.to_le_bytes()).collect();
                let uni_suffix = uni_base + wide.len();
                for v in [0, header, 1, 0, ansi_base as u32, 0, ansi_suffix as u32, uni_base as u32, uni_suffix as u32] {
                    body.extend_from_slice(&v.to_le_bytes());
                }
                body.extend_from_slice(&[0, 0]);
                body.extend_from_slice(&wide);
                body.extend_from_slice(&[0, 0]);
            } else {
                let suffix_off = base_off + path.len() + 1;
                for v in [0, header, 1, 0, base_off as u32, 0, suffix_off as u32] {
                    body.extend_from_slice(&v.to_le_bytes());
                }
                body.extend_from_slice(path.as_bytes());
                body.extend_from_slice(&[0, 0]);
            }
            let size = body.len() as u32;
            body[0..4].copy_from_slice(&size.to_le_bytes());
            d.extend_from_slice(&body);
        }
        if let Some(a) = args {
            let wide: Vec<u16> = a.encode_utf16().collect();
            d.extend_from_slice(&(wide.len() as u16).to_le_bytes());
            wide.iter().for_each(|w| d.extend_from_slice(&w.to_le_bytes()));
        }
        if let Some(e) = env {
            let mut block = vec![0u8; 0x314];
            block[0..4].copy_from_slice(&0x314u32.to_le_bytes());
            block[4..8].copy_from_slice(&ENV_BLOCK.to_le_bytes());
            for (i, w) in e.encode_utf16().enumerate() {
                block[8 + 260 + i * 2..8 + 262 + i * 2].copy_from_slice(&w.to_le_bytes());
            }
            d.extend_from_slice(&block);
        }
        d.extend_from_slice(&0u32.to_le_bytes());
        d
    }

    #[test]
    fn link_info_paths_and_arguments() {
        let dir = Path::new("C:\\Start");
        let s = parse(&lnk(Some(("C:\\Apps\\Chrome\\chrome.exe", false)), None, None), dir).unwrap();
        assert_eq!(s.target, "C:\\Apps\\Chrome\\chrome.exe");
        assert_eq!(s.args, "");
        let s = parse(
            &lnk(Some(("C:\\Users\\Пётр\\AppData\\Local\\Discord\\Update.exe", true)), Some("--processStart Discord.exe"), None),
            dir,
        )
        .unwrap();
        assert_eq!(s.target, "C:\\Users\\Пётр\\AppData\\Local\\Discord\\Update.exe", "UTF-16 путь не искажается");
        assert_eq!(s.args, "--processStart Discord.exe");
    }

    #[test]
    fn environment_block_wins_and_expands() {
        std::env::set_var("CLOD_LNK_TEST_ROOT", "D:\\Soft");
        let d = lnk(Some(("C:\\Old\\x.exe", false)), None, Some("%CLOD_LNK_TEST_ROOT%\\X\\x.exe"));
        assert_eq!(parse(&d, Path::new("C:\\Start")).unwrap().target, "D:\\Soft\\X\\x.exe");
        assert_eq!(expand_env("%NO_SUCH_CLOD_VAR%\\a %"), "%NO_SUCH_CLOD_VAR%\\a %");
    }

    #[test]
    fn garbage_is_rejected_without_panics() {
        assert_eq!(parse(b"", Path::new("x")), None);
        assert_eq!(parse(&[0u8; 0x4C], Path::new("x")), None);
        let mut d = lnk(Some(("C:\\a.exe", false)), None, None);
        d.truncate(0x50); // LinkInfo обрезан
        assert_eq!(parse(&d, Path::new("x")), None);
        // Ярлык без цели (только заголовок) — не программа.
        let mut d = vec![0u8; 0x50];
        d[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        assert_eq!(parse(&d, Path::new("x")), None);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn relative_target_is_normalized() {
        assert_eq!(
            normalize(Path::new("C:\\Start\\Programs\\..\\..\\App\\x.exe")),
            Path::new("C:\\App\\x.exe").to_string_lossy()
        );
    }
}
