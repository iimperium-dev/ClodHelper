//! Заставка «Обновляю Clod Helper» на время установки новой версии (Windows).
//!
//! Установщик NSIS в режиме /P показывал своё белое окно — чужеродное и
//! некрасивое, а в тихом режиме /S после выхода приложения несколько секунд
//! не было видно ничего. Поэтому установщик идёт тихо, а пока он работает,
//! висит наша заставка: копия exe приложения, запущенная с `--update-splash`.
//!
//! Копия лежит в папке обновления под другим именем — установщик Tauri
//! закрывает процессы только по имени `clod-helper.exe`, заставку он не
//! тронет. Она закрывается сама, как только стартует новая версия; если
//! установщик завершился, а новой версии нет, — честно говорит об этом.

use std::path::Path;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

pub const FLAG: &str = "--update-splash";
const LABEL: &str = "splash";
/// Сколько ждём новую версию после выхода установщика, прежде чем сказать,
/// что что-то пошло не так.
const START_GRACE: Duration = Duration::from_secs(20);
/// Предел жизни заставки в любом случае.
const MAX_LIFETIME: Duration = Duration::from_secs(180);

/// Запустить заставку рядом с установщиком `setup_pid`.
pub fn spawn(work: &Path, setup_pid: u32, version: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let copy = work.join("clod-helper-updating.exe");
    std::fs::copy(&exe, &copy).map_err(|e| format!("копия для заставки: {e}"))?;
    std::process::Command::new(&copy)
        .args([FLAG, &setup_pid.to_string(), version])
        .spawn()
        .map_err(|e| format!("заставка обновления: {e}"))?;
    Ok(())
}

/// Режим заставки: одно окно, никаких трея, мониторов и мостов.
pub fn run(args: &[String], mut context: tauri::Context<tauri::Wry>) {
    let pos = args.iter().position(|a| a == FLAG).unwrap_or(0);
    let setup_pid: u32 = args.get(pos + 1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let version = args.get(pos + 2).cloned().unwrap_or_default();
    // Окна из tauri.conf.json (главное и трей-попап) заставке не нужны.
    context.config_mut().app.windows.clear();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![splash_close])
        .setup(move |app| {
            // Свой каталог данных WebView2: новая версия стартует, пока
            // заставка ещё на экране, и не должна делить с ней профиль.
            let data_dir = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|d| d.join("splash-webview")));
            let url = format!("index.html?view=updating&v={}", urlencode(&version));
            let mut b = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App(url.into()))
                .title("Обновление Clod Helper")
                .inner_size(380.0, 260.0)
                .center()
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(false);
            if let Some(dir) = data_dir {
                b = b.data_directory(dir);
            }
            b.build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || watch(handle, setup_pid));
            Ok(())
        })
        .run(context)
        .expect("update splash");
}

/// Следить за установщиком и новой версией: стартовала — плавно уйти.
fn watch(app: tauri::AppHandle, setup_pid: u32) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let started = Instant::now();
    let mut setup_done_at: Option<Instant> = None;
    let mut failed = false;
    let mut sys = System::new();
    let app_pids = |sys: &System| -> Vec<u32> {
        sys.processes()
            .iter()
            .filter(|(_, p)| p.name().to_string_lossy().eq_ignore_ascii_case("clod-helper.exe"))
            .map(|(pid, _)| pid.as_u32())
            .collect()
    };
    // Старая версия (её закроет установщик) — не путать с новой.
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    let old = app_pids(&sys);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
        let new_app = app_pids(&sys).iter().any(|pid| !old.contains(pid));
        if new_app {
            let _ = app.emit("splash:state", "started");
            // Новое окно поднимается ~1–2 с — заставка уходит, когда оно уже видно.
            std::thread::sleep(Duration::from_millis(1600));
            app.exit(0);
            return;
        }
        let setup_alive = setup_pid != 0 && sys.process(Pid::from_u32(setup_pid)).is_some();
        if !setup_alive && setup_done_at.is_none() {
            setup_done_at = Some(Instant::now());
            let _ = app.emit("splash:state", "starting");
        }
        if !failed && setup_done_at.is_some_and(|t| t.elapsed() > START_GRACE) {
            failed = true;
            let _ = app.emit("splash:state", "failed");
        }
        if started.elapsed() > MAX_LIFETIME {
            app.exit(0);
            return;
        }
    }
}

#[tauri::command]
fn splash_close(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    app.exit(0);
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
