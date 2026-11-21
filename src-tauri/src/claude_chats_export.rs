//! «Инструменты» → архив локальных чатов Claude.
//!
//! Вся переписка, которая хранится на компьютере (вкладка Code и задачи
//! Cowork в Claude Desktop, Claude Code в терминале), - одним ZIP в папку
//! «Загрузки». Внутри README.md: что где лежит и как в этом искать, -
//! чтобы потом попросить агента разобраться в архиве.
//!
//! Используется отдельно (кнопка «Сохранить архив») и перед удалением
//! локальных чатов при очистке: архив не сохранился - чаты не удаляются.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::claude_wipe::{Env, DESKTOP_CHATS};

/// Переписка Claude Code в ~/.claude (без служебного: снимков оболочки,
/// окружения сессий, списка запущенных процессов).
const CODE_CHATS: &[&str] = &["projects", "history.jsonl", "file-history", "todos", "plans", "uploads"];

/// Папки, которые не нужны для чтения переписки и бывают огромными.
const SKIP_DIRS: &[&str] = &["node_modules"];

#[derive(Debug, Clone, Serialize)]
pub struct Archive {
    /// Полный путь к сохранённому ZIP.
    pub path: String,
    /// Размер архива.
    pub bytes: u64,
    /// Сколько файлов вошло.
    pub files: u32,
    /// Сколько файлов не удалось прочитать (заняты другой программой).
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub active: bool,
    /// Обработано байт исходных файлов.
    pub done: u64,
    pub total: u64,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn progress() -> Progress {
    Progress {
        active: ACTIVE.load(Ordering::Relaxed),
        done: DONE.load(Ordering::Relaxed),
        total: TOTAL.load(Ordering::Relaxed),
    }
}

/// Что положить в архив: (путь на диске, путь в архиве).
fn sources(env: &Env) -> Vec<(PathBuf, String)> {
    let mut v = Vec::new();
    for (i, d) in env.desktop_dirs.iter().enumerate() {
        let root = if i == 0 { "Claude Desktop".to_string() } else { format!("Claude Desktop ({})", i + 1) };
        for rel in DESKTOP_CHATS {
            let p = d.join(rel);
            if p.exists() {
                v.push((p, format!("{root}/{rel}")));
            }
        }
    }
    if let Some(dir) = &env.code_dir {
        for rel in CODE_CHATS {
            let p = dir.join(rel);
            if p.exists() {
                v.push((p, format!("Claude Code/{rel}")));
            }
        }
    }
    v
}

/// Ссылка, точка соединения и прочие перенаправления - не идём внутрь:
/// иначе в архив попадёт чужая папка (или сам архив по кругу).
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Все файлы источника: (путь на диске, путь в архиве, размер, время изменения).
fn walk(src: &Path, name: &str, out: &mut Vec<(PathBuf, String, u64, Option<SystemTime>)>) {
    // Сам источник может быть ссылкой (папку перенесли на другой диск) -
    // идём по ней. Внутри ссылки пропускаем: иначе можно уйти по кругу.
    let Ok(meta) = std::fs::metadata(src) else { return };
    if meta.is_file() {
        out.push((src.to_path_buf(), name.to_string(), meta.len(), meta.modified().ok()));
        return;
    }
    let mut stack = vec![(src.to_path_buf(), name.to_string())];
    while let Some((dir, prefix)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
            if is_link(&m) {
                continue;
            }
            let fname = e.file_name().to_string_lossy().to_string();
            let inner = format!("{prefix}/{fname}");
            if m.is_dir() {
                // Папка-кэш со стандартной меткой (target/ сборки Rust и т.п.):
                // в рабочих папках сессий это гигабайты, к переписке отношения нет.
                if !SKIP_DIRS.contains(&fname.as_str()) && !e.path().join("CACHEDIR.TAG").is_file() {
                    stack.push((e.path(), inner));
                }
            } else {
                out.push((e.path(), inner, m.len(), m.modified().ok()));
            }
        }
    }
}

/// Сдвиг местного времени от UTC в секундах (ZIP и имя архива - по местному).
fn utc_offset() -> i64 {
    // Раз в 10 минут: программа живёт в трее днями, а пояс меняется
    // (летнее время, поездка).
    static OFFSET: std::sync::Mutex<Option<(std::time::Instant, i64)>> = std::sync::Mutex::new(None);
    let mut cache = OFFSET.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, v)) = *cache {
        if at.elapsed() < std::time::Duration::from_secs(600) {
            return v;
        }
    }
    let v = (|| {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
            let (mut l, mut u) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
            unsafe {
                GetLocalTime(&mut l);
                GetSystemTime(&mut u);
            }
            let secs = |t: &windows_sys::Win32::Foundation::SYSTEMTIME| {
                days_from_civil(t.wYear as i64, t.wMonth as u32, t.wDay as u32) * 86_400
                    + t.wHour as i64 * 3600
                    + t.wMinute as i64 * 60
            };
            // Округляем до 15 минут: между двумя вызовами могла смениться минута.
            ((secs(&l) - secs(&u)) as f64 / 900.0).round() as i64 * 900
        }
        #[cfg(not(target_os = "windows"))]
        {
            // «+0300» от date(1).
            std::process::Command::new("/bin/date")
                .arg("+%z")
                .output()
                .ok()
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    let sign = if s.starts_with('-') { -1 } else { 1 };
                    let n: i64 = s.trim_start_matches(['+', '-']).parse().ok()?;
                    Some(sign * ((n / 100) * 3600 + (n % 100) * 60))
                })
                .unwrap_or(0)
        }
    })();
    *cache = Some((std::time::Instant::now(), v));
    v
}

/// Местное время (секунды «как будто UTC»), для разбора на дату.
fn local_secs(t: SystemTime) -> u64 {
    let s = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    (s + utc_offset()).max(0) as u64
}

/// (год, месяц, день) → дни от 1970-01-01 (Хиннант, «days_from_civil»).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Дата для записи ZIP (местное время: ZIP хранит время без пояса).
fn zip_time(t: Option<SystemTime>) -> DateTime {
    let (y, m, d, hh, mm, ss) = civil(local_secs(t.unwrap_or(UNIX_EPOCH)));
    DateTime::from_date_and_time(y.clamp(1980, 2107) as u16, m as u8, d as u8, hh as u8, mm as u8, ss as u8)
        .unwrap_or_default()
}

/// Секунды UNIX → (год, месяц, день, часы, минуты, секунды) по UTC.
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Алгоритм Хиннанта «days_from_civil» в обратную сторону.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

/// Уже сжатое - без повторного сжатия (быстрее, архив не больше).
fn stored(name: &str) -> bool {
    let n = name.to_lowercase();
    [".zip", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".mov", ".pdf", ".gz", ".7z", ".woff2"]
        .iter()
        .any(|e| n.ends_with(e))
}

/// Свободное имя в папке: «Чаты Claude 2026-11-16.zip», «… (2).zip».
fn free_name(dir: &Path) -> PathBuf {
    let (y, m, d, ..) = civil(local_secs(SystemTime::now()));
    let base = format!("Чаты Claude {y:04}-{m:02}-{d:02}");
    let mut p = dir.join(format!("{base}.zip"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{base} ({i}).zip"));
        i += 1;
    }
    p
}

/// Читает файл и считает прогресс.
struct Counting<R> {
    inner: R,
}
impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        DONE.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Сбросить флаг «идёт выгрузка» при любом выходе.
struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Relaxed);
    }
}

/// Есть ли что сохранять.
pub fn has_chats(env: &Env) -> bool {
    let mut files = Vec::new();
    for (src, name) in sources(env) {
        walk(&src, &name, &mut files);
        if !files.is_empty() {
            return true;
        }
    }
    false
}

pub fn export() -> Result<Archive, String> {
    export_env(&crate::claude_wipe::env())
}

pub fn export_env(env: &Env) -> Result<Archive, String> {
    let dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .ok_or("Не нашлась папка «Загрузки»")?;
    export_to(env, &dir)
}

pub fn export_to(env: &Env, dir: &Path) -> Result<Archive, String> {
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("Архив уже сохраняется".into());
    }
    let _guard = ActiveGuard;
    DONE.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);

    let mut files = Vec::new();
    for (src, name) in sources(env) {
        walk(&src, &name, &mut files);
    }
    if files.is_empty() {
        return Err("Локальных чатов на этом компьютере нет".into());
    }
    TOTAL.store(files.iter().map(|f| f.2).sum(), Ordering::Relaxed);

    std::fs::create_dir_all(dir).map_err(|e| format!("Папка «Загрузки» недоступна: {e}"))?;
    let dest = free_name(dir);
    let part = dest.with_extension("zip.part");
    let result = write_zip(&part, &files);
    let (count, skipped) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    std::fs::rename(&part, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("Не удалось сохранить архив: {e}")
    })?;
    let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    tracing::info!("архив локальных чатов Claude: {count} файлов, {bytes} байт");
    Ok(Archive { path: dest.to_string_lossy().into_owned(), bytes, files: count, skipped: skipped.len() as u32 })
}

/// (сколько файлов вошло, какие не прочитались).
fn write_zip(
    part: &Path,
    files: &[(PathBuf, String, u64, Option<SystemTime>)],
) -> Result<(u32, Vec<String>), String> {
    let file = std::fs::File::create(part).map_err(|e| format!("Не удалось создать архив: {e}"))?;
    let mut zw = ZipWriter::new(std::io::BufWriter::with_capacity(1 << 20, file));
    let mut count = 0u32;
    let mut skipped = Vec::new();
    for (src, name, size, modified) in files {
        let mut f = match std::fs::File::open(src) {
            Ok(f) => f,
            Err(_) => {
                DONE.fetch_add(*size, Ordering::Relaxed);
                skipped.push(name.clone());
                continue;
            }
        };
        let opts = SimpleFileOptions::default()
            .compression_method(if stored(name) { CompressionMethod::Stored } else { CompressionMethod::Deflated })
            // Быстрое сжатие: текст переписки и так ужимается в 2-3 раза, а
            // сильное сжатие гигабайтов заняло бы минуты.
            .compression_level(if stored(name) { None } else { Some(1) })
            .last_modified_time(zip_time(*modified))
            .large_file(*size >= 0xFFFF_0000);
        zw.start_file(name.as_str(), opts).map_err(|e| format!("Ошибка архива: {e}"))?;
        let before = DONE.load(Ordering::Relaxed);
        if let Err(e) = std::io::copy(&mut Counting { inner: &mut f }, &mut zw) {
            // Файл перестал читаться посередине: в архиве останется его часть.
            // Диск переполнен - дальше смысла нет.
            if e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(112) {
                return Err("На диске закончилось место".into());
            }
            skipped.push(format!("{name} (частично)"));
        } else {
            count += 1;
        }
        let read = DONE.load(Ordering::Relaxed) - before;
        if read < *size {
            DONE.fetch_add(size - read, Ordering::Relaxed);
        }
    }
    let readme = readme(count, &skipped);
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip_time(Some(SystemTime::now())));
    zw.start_file("README.md", opts).map_err(|e| format!("Ошибка архива: {e}"))?;
    zw.write_all(readme.as_bytes()).map_err(|e| format!("Ошибка архива: {e}"))?;
    let mut inner = zw.finish().map_err(|e| format!("Не удалось дописать архив: {e}"))?;
    inner.flush().map_err(|e| format!("Не удалось дописать архив: {e}"))?;
    Ok((count, skipped))
}

fn readme(count: u32, skipped: &[String]) -> String {
    let (y, m, d, hh, mm, _) = civil(local_secs(SystemTime::now()));
    let mut s = format!(
        "# Локальные чаты Claude\n\n\
Архив сохранён {y:04}-{m:02}-{d:02} {hh:02}:{mm:02} программой Clod Helper. \
Файлов: {count}.\n\n\
Здесь вся переписка, которая хранилась на компьютере: вкладка Code и задачи Cowork \
в Claude Desktop и Claude Code в терминале. Чаты из облака (claude.ai) остаются в самом \
аккаунте и в архив не входят.\n\n\
## Что где лежит\n\n\
- `Claude Code/projects/<папка проекта>/<id сессии>.jsonl` - переписка Claude Code, \
в том числе вкладки Code в Claude Desktop. Одна строка - одна запись в JSON: `type` \
(`user`, `assistant`, `summary`…), `message.content` (текст, вызовы инструментов и их \
результаты), `timestamp`, `cwd`, `sessionId`. Имя папки - путь проекта, в котором \
`\\`, `/`, `:` и пробелы заменены на `-`.\n\
- `Claude Code/projects/<папка проекта>/<id сессии>/subagents/` - переписка вложенных агентов.\n\
- `Claude Code/projects/<папка проекта>/memory/` - заметки памяти проекта (Markdown).\n\
- `Claude Code/history.jsonl` - история введённых запросов (`display`, `timestamp`, `project`).\n\
- `Claude Code/file-history/<id сессии>/` - копии файлов до правок Claude.\n\
- `Claude Code/todos/`, `Claude Code/plans/` - списки задач и планы сессий.\n\
- `Claude Code/uploads/` - файлы, приложенные к чатам.\n\
- `Claude Desktop/claude-code-sessions/<аккаунт>/<организация>/local_*.json` - карточки \
сессий вкладки Code: название (`title`), папка (`cwd`), модель, время, и `cliSessionId` - \
id файла переписки в `Claude Code/projects`.\n\
- `Claude Desktop/local-agent-mode-sessions/…` - задачи и сессии Cowork, их файлы и расписания.\n\
- `Claude Desktop/scratch-workspaces/…` - рабочие папки временных сессий (что в них создавал Claude).\n\
- `Claude Desktop/side-session-notes/` - заметки побочных сессий.\n\
- `Claude Desktop/git-worktrees.json` - список рабочих копий git, созданных для сессий.\n\
- `Claude Desktop/git-shadow/` - теневые копии git рабочих папок сессий (для отмены правок).\n\n\
## Как искать\n\n\
Распакуйте архив и попросите агента: «Найди в этом архиве чат про …» или «Собери, что мы \
решили по проекту X». Быстрее всего - искать по тексту во всех `*.jsonl` в \
`Claude Code/projects`, а название сессии брать из карточек в \
`Claude Desktop/claude-code-sessions` по `cliSessionId`.\n\n\
Папки `node_modules` в рабочих папках не сохранены: это установленные зависимости, их \
восстанавливает установка пакетов. Не сохранены и папки-кэши (с файлом `CACHEDIR.TAG`, \
например `target` сборки Rust): их заново создаёт сборка. Ссылки на другие папки внутри тоже не сохранены.\n\n\
Архив содержит переписку и данные прошлого аккаунта - храните его отдельно от нового.\n"
    );
    if skipped.is_empty() {
        s.push_str("\n## Не вошло\n\nВсё скопировано.\n");
    } else {
        s.push_str("\n## Не вошло\n\nЭти файлы держала другая программа, их не удалось прочитать:\n\n");
        for f in skipped.iter().take(500) {
            s.push_str(&format!("- `{f}`\n"));
        }
        if skipped.len() > 500 {
            s.push_str(&format!("- …и ещё {}\n", skipped.len() - 500));
        }
    }
    s
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Архив одновременно только один (общий счётчик прогресса) - тесты по очереди.
    pub(crate) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn exports_chats_with_readme() {
        let _serial = serial();
        let root = std::env::temp_dir().join(format!("clod-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("data");
        let home = root.join("home");
        let w = |p: PathBuf, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(d.join("claude-code-sessions").join("a").join("o").join("local_1.json"), r#"{"title":"Чат"}"#);
        w(d.join("scratch-workspaces").join("a").join("node_modules").join("big.js"), "x");
        // Сборка Rust в рабочей папке: на живом Маке такие target - 18 ГБ.
        w(d.join("scratch-workspaces").join("a").join("target").join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55");
        w(d.join("scratch-workspaces").join("a").join("target").join("debug").join("app"), "бинарник");
        w(d.join("Cache").join("f"), "кэш не нужен");
        w(home.join(".claude").join("projects").join("C--p").join("s.jsonl"), "{\"type\":\"user\"}\n");
        w(home.join(".claude").join("history.jsonl"), "{}\n");
        w(home.join(".claude").join("settings.json"), "{}");
        let env = Env { desktop_dirs: vec![d], code_dir: Some(home.join(".claude")), ..Env::default() };
        let out = root.join("Downloads");

        let a = export_to(&env, &out).unwrap();
        assert_eq!(a.files, 3, "{a:?}");
        assert!(a.path.ends_with(".zip") && Path::new(&a.path).exists());
        let mut z = zip::ZipArchive::new(std::fs::File::open(&a.path).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        for n in [
            "README.md",
            "Claude Desktop/claude-code-sessions/a/o/local_1.json",
            "Claude Code/projects/C--p/s.jsonl",
            "Claude Code/history.jsonl",
        ] {
            assert!(names.iter().any(|x| x == n), "нет {n}: {names:?}");
        }
        assert!(!names.iter().any(|x| x.contains("node_modules") || x.contains("Cache") || x.contains("target/") || x.contains("settings.json")));
        let mut text = String::new();
        z.by_name("Claude Desktop/claude-code-sessions/a/o/local_1.json").unwrap().read_to_string(&mut text).unwrap();
        assert!(text.contains("Чат"));
        assert!(!progress().active);

        // Второй архив в тот же день - новое имя, первый не перезаписан.
        let b = export_to(&env, &out).unwrap();
        assert_ne!(a.path, b.path);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Файл, который держит другая программа, не теряется молча: он в счётчике
    /// пропущенных (по нему очистка отказывается удалять чаты) и в README.
    #[test]
    #[cfg(target_os = "windows")]
    fn locked_file_is_counted_as_skipped() {
        use std::os::windows::fs::OpenOptionsExt;
        let _serial = serial();
        let root = std::env::temp_dir().join(format!("clod-export-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = root.join("home").join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("ok.jsonl"), "{}").unwrap();
        std::fs::write(p.join("busy.jsonl"), "{}").unwrap();
        // Открыт без права совместного чтения - как у занятого файла.
        let _lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(p.join("busy.jsonl")).unwrap();
        let env = Env { code_dir: Some(root.join("home").join(".claude")), ..Env::default() };
        let a = export_to(&env, &root.join("out")).unwrap();
        assert_eq!((a.files, a.skipped), (1, 1), "{a:?}");
        let mut z = zip::ZipArchive::new(std::fs::File::open(&a.path).unwrap()).unwrap();
        let mut readme = String::new();
        z.by_name("README.md").unwrap().read_to_string(&mut readme).unwrap();
        assert!(readme.contains("busy.jsonl"));
        drop(_lock);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn days_roundtrip() {
        for secs in [0u64, 951_782_400, 1_763_300_000, 4_102_444_800] {
            let (y, m, d, ..) = civil(secs);
            assert_eq!(days_from_civil(y, m, d), (secs / 86_400) as i64);
        }
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_763_300_000), (2025, 11, 16, 13, 33, 20));
    }
}
