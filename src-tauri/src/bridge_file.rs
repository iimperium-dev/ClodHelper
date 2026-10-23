//! Файл моста пропал — вернуть его из копии внутри программы.
//!
//! Мост — неподписанная программа, через которую идёт трафик: антивирусы
//! (чаще всего Защитник Windows) иногда ложно убирают его в карантин. Раньше
//! это заканчивалось строкой «Не найден …clod-bridge.exe», и человек сам
//! искал, что случилось. Теперь перед регистрацией и запуском моста:
//!   - файла нет → копируем встроенный мост из ресурсов программы и
//!     убеждаемся, что антивирус не удалил его сразу;
//!   - удалён снова вскоре после восстановления → больше не воюем с
//!     антивирусом (каждая попытка — новое срабатывание у него), а
//!     показываем, как разрешить файл и добавить исключение;
//!   - копии в программе тоже нет → то же, плюс переустановка.
//! Пока моста нет, трафик Claude стоит: напрямую ничего не уходит.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Удалён снова в течение этого времени после восстановления — значит,
/// антивирус удаляет его систематически.
const AGAIN_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Сколько дать антивирусу на проверку только что записанного файла.
const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Мост пропал, программа вернула его — всё работает (совет про исключение).
    Restored,
    /// Антивирус удаляет мост снова — нужна помощь человека.
    RemovedAgain,
    /// Копии моста в программе тоже нет — разрешить файлы или переустановить.
    NoCopy,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub kind: IssueKind,
    /// Папки, которые стоит добавить в исключения антивируса.
    pub dirs: Vec<String>,
}

#[derive(Default)]
struct State {
    kind: Option<IssueKind>,
    restored_at: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State { kind: None, restored_at: None });
static BUNDLED: OnceLock<PathBuf> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();

/// При запуске программы: где лежит встроенный мост и куда слать события.
pub fn init(app: &AppHandle, bundled: Option<PathBuf>) {
    let _ = APP.set(app.clone());
    if let Some(p) = bundled {
        let _ = BUNDLED.set(p);
    }
}

/// Текущая проблема с файлом моста (None — всё в порядке).
pub fn issue() -> Option<Issue> {
    let kind = kind_of(&STATE)?;
    Some(Issue { kind, dirs: exclusion_dirs() })
}

fn kind_of(state: &Mutex<State>) -> Option<IssueKind> {
    state.lock().ok()?.kind
}

/// «Понятно» на сообщении о восстановлении. Нерешённые проблемы не скрываются.
pub fn dismiss() {
    if let Ok(mut s) = STATE.lock() {
        if s.kind == Some(IssueKind::Restored) {
            s.kind = None;
        }
    }
    notify();
}

fn set(state: &Mutex<State>, kind: Option<IssueKind>) {
    if let Ok(mut s) = state.lock() {
        if s.kind == kind {
            return;
        }
        s.kind = kind;
    }
    notify();
}

fn notify() {
    if let Some(app) = APP.get() {
        let _ = app.emit("bridge-file", issue());
    }
}

/// Папки для исключений антивируса: установленный мост и сама программа
/// (там лежит его копия для восстановления).
fn exclusion_dirs() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Some(d) = crate::shim::install_dir() {
        v.push(d.to_string_lossy().into_owned());
    }
    // Папка самой программы (копия моста — в её resources). На macOS
    // программа — .app в «Программах», её антивирусы не трогают.
    let app_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    if let Some(d) = app_dir.filter(|_| cfg!(target_os = "windows") && BUNDLED.get().is_some()) {
        let d = d.to_string_lossy().into_owned();
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

fn removed_again_text() -> String {
    if cfg!(target_os = "windows") {
        "Антивирус удаляет мост (clod-bridge.exe): разрешите его в журнале защиты Windows и добавьте папку в исключения — подробности на «Обзоре»".into()
    } else {
        "Антивирус удаляет мост (clod-bridge): разрешите его и добавьте папку в исключения — подробности на «Обзоре»".into()
    }
}

/// Перед регистрацией и запуском моста: файл на месте или возвращён.
/// `manual` — человек нажал «Проверить снова» (возможно, добавил исключение):
/// пробуем восстановить, даже если антивирус только что удалял.
pub fn ensure(manual: bool) -> Result<(), String> {
    let Some(dst) = crate::shim::bridge_exe_path() else { return Ok(()) };
    // Совсем старая установка на python-мосте (exe ещё не было) — не наш случай.
    if !dst.exists() && crate::shim::shim_script_path().is_some_and(|p| p.exists()) {
        return Ok(());
    }
    let src = BUNDLED.get().map(PathBuf::as_path);
    restore(&STATE, &dst, src, manual, SETTLE)
}

/// Суть ensure — на явных путях и своём состоянии (для тестов).
fn restore(
    state: &Mutex<State>,
    dst: &std::path::Path,
    src: Option<&std::path::Path>,
    manual: bool,
    settle: Duration,
) -> Result<(), String> {
    if dst.exists() {
        // Человек сам вернул файл из карантина — проблема решена.
        if matches!(kind_of(state), Some(IssueKind::RemovedAgain | IssueKind::NoCopy)) {
            set(state, None);
        }
        return Ok(());
    }
    // Удалён вскоре после восстановления — или уже признан «удаляет снова»:
    // сам больше не восстанавливаем (каждая попытка — новое срабатывание
    // антивируса), ждём человека: «Проверить снова» или перезапуск программы.
    let (recently, gave_up) = state
        .lock()
        .map(|s| {
            (
                s.restored_at.is_some_and(|t| t.elapsed() < AGAIN_WINDOW),
                s.kind == Some(IssueKind::RemovedAgain),
            )
        })
        .unwrap_or((false, false));
    if (recently || gave_up) && !manual {
        tracing::warn!("мост удалён снова после восстановления — нужна помощь пользователя");
        set(state, Some(IssueKind::RemovedAgain));
        return Err(removed_again_text());
    }
    let Some(src) = src.filter(|p| p.exists()) else {
        tracing::warn!("мост удалён, а копии в программе нет");
        set(state, Some(IssueKind::NoCopy));
        return Err("Антивирус удалил мост и его копию в программе — разрешите файлы в журнале защиты или переустановите Clod Helper (подробности на «Обзоре»)".into());
    };
    if let Some(dir) = dst.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = crate::shim::install_bridge_binary(src, &dst) {
        // Антивирус не дал даже записать файл.
        tracing::warn!("восстановить мост не удалось: {e}");
        set(state, Some(IssueKind::RemovedAgain));
        return Err(removed_again_text());
    }
    std::thread::sleep(settle);
    if !dst.exists() {
        tracing::warn!("восстановленный мост сразу удалён антивирусом");
        set(state, Some(IssueKind::RemovedAgain));
        return Err(removed_again_text());
    }
    tracing::warn!("мост пропал (антивирус?) — восстановлен из копии в программе");
    if let Ok(mut s) = state.lock() {
        s.restored_at = Some(Instant::now());
    }
    set(state, Some(IssueKind::Restored));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_once_then_waits_for_the_user() {
        let dir = std::env::temp_dir().join(format!("clod-bridge-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let src = dir.join("app").join("clod-bridge.exe");
        let dst = dir.join("installed").join("clod-bridge.exe");
        std::fs::write(&src, b"bridge").unwrap();
        let st = Mutex::new(State::default());
        let quick = Duration::ZERO;

        // Пропал — вернули из копии программы.
        restore(&st, &dst, Some(&src), false, quick).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"bridge");
        assert_eq!(kind_of(&st), Some(IssueKind::Restored));

        // Антивирус удалил снова вскоре после — не воюем, ждём человека.
        std::fs::remove_file(&dst).unwrap();
        assert!(restore(&st, &dst, Some(&src), false, quick).is_err());
        assert!(!dst.exists(), "сам повторно не восстанавливает");
        assert_eq!(kind_of(&st), Some(IssueKind::RemovedAgain));
        // …и не возвращается к попыткам сам, сколько бы раз его ни звали.
        assert!(restore(&st, &dst, Some(&src), false, quick).is_err());
        assert!(!dst.exists());

        // «Проверить снова» (исключение добавлено) — восстанавливает.
        restore(&st, &dst, Some(&src), true, quick).unwrap();
        assert!(dst.exists());
        assert_eq!(kind_of(&st), Some(IssueKind::Restored));

        // Копии в программе нет — честно «нет копии».
        std::fs::remove_file(&dst).unwrap();
        std::fs::remove_file(&src).unwrap();
        assert!(restore(&st, &dst, Some(&src), true, quick).is_err());
        assert_eq!(kind_of(&st), Some(IssueKind::NoCopy));

        // Человек сам вернул файл из карантина — проблема снята.
        std::fs::write(&dst, b"bridge").unwrap();
        restore(&st, &dst, None, false, quick).unwrap();
        assert_eq!(kind_of(&st), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}