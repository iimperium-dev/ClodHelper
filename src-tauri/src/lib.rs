mod commands;
mod proxy;
mod settings;
mod shim;
mod system;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clod_helper_lib=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--tray"]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::detect_node,
            commands::detect_python,
            commands::parse_proxy,
            commands::check_proxy,
            commands::run_install,
            commands::launch_claude_desktop,
            commands::launch_claude_code,
            commands::detect_cursor,
            commands::setup_cursor,
            commands::launch_cursor,
            // Shim control
            commands::shim_status,
            commands::shim_start,
            commands::shim_stop,
            commands::shim_restart,
            commands::shim_test,
            commands::shim_script_path,
            // Settings
            commands::settings_get,
            commands::settings_set_proxy,
        ])
        .setup(|app| {
            setup_tray(app.handle())?;

            // Watchdog: авто-подъём упавшего шима (корневой фикс «деплой роняет
            // мост → Claude Code теряет связь»). Фоновый поток, переживает любые
            // ошибки, поднимает шим за ~5с после падения.
            shim::spawn_watchdog();

            // Если запущены с флагом --tray (после автозапуска Windows) —
            // прячем главное окно при старте, видна только иконка в трее.
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--tray") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Close-to-tray: крестик прячет окно, не закрывает приложение.
                // Полный выход — только через меню трея "Выход".
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть Dashboard", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "Перезапустить шим", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Остановить шим", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &restart, &stop, &quit])?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Clod Helper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
            }
            "restart" => {
                let _ = tauri::async_runtime::spawn(async {
                    let _ = tokio::task::spawn_blocking(shim::restart).await;
                });
            }
            "stop" => {
                let _ = tauri::async_runtime::spawn(async {
                    let _ = tokio::task::spawn_blocking(shim::stop).await;
                });
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Левый клик по иконке — открыть Dashboard.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}
