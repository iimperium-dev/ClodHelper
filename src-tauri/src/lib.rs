mod apps;
mod autostart;
mod claude_desktop;
mod claude_policy;
mod claude_proxy;
mod commands;
#[cfg(target_os = "windows")]
mod elevate;
mod env_proxy;
#[cfg_attr(not(target_os = "windows"), path = "firewall_stub.rs")]
mod firewall;
// В тестах модуль собирается и на Windows: его чистые функции покрыты
// юнит-тестами, а реальный прогон на macOS доступен только в CI.
#[cfg(any(not(target_os = "windows"), test))]
mod macos;
// Разбор ярлыков .lnk: нужен на Windows, тесты — на любой ОС.
#[cfg(any(target_os = "windows", test))]
mod lnk;
mod monitor;
mod proxy;
mod regval;
mod settings;
mod shim;
mod system;
mod tasks;
mod telegram;
mod traffic;
mod browser_ext;
mod browser_policy;
mod leak_guard;
mod privacy;
mod sound;
mod system_pac;
mod update_notify;
#[cfg(target_os = "windows")]
mod update_splash;
mod updater;
mod vscode_config;
#[cfg(target_os = "windows")]
mod win;

use tauri::{
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

/// Служебные CLI-режимы, которые отрабатывают и выходят без GUI
/// (LaunchAgent переменных прокси). None — обычный запуск приложения.
pub fn cli_mode() -> Option<i32> {
    #[cfg(not(target_os = "windows"))]
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(code) = env_proxy::apply_env_cli(&args) {
            return Some(code);
        }
        #[cfg(target_os = "macos")]
        if let Some(code) = autostart::login_item_cli(&args) {
            return Some(code);
        }
    }
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "clod_helper_lib=info".into()),
        )
        .init();

    // Из Dock/Finder приложение получает PATH без Homebrew/nvm — node, npm и
    // claude не нашлись бы. Чиним до любых спавнов.
    #[cfg(not(target_os = "windows"))]
    macos::fix_path_env();

    // Контекст (с ресурсами фронтенда) создаётся один раз: второй вызов
    // generate_context! вшил бы фронтенд в exe повторно.
    let context = tauri::generate_context!();

    // Заставка на время установки обновления — отдельный короткий режим.
    #[cfg(target_os = "windows")]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == update_splash::FLAG) {
            update_splash::run(&args, context);
            return;
        }
    }

    let builder = tauri::Builder::default();
    // Автозапуск: на Windows — плагин (ключ Run), на macOS — SMAppService
    // (src/autostart.rs); плагин там больше не регистрируется.
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--tray"]),
    ));

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::detect_node,
            commands::detect_python,
            commands::parse_proxy,
            commands::check_proxy,
            commands::run_install,
            commands::launch_claude_desktop,
            commands::claude_proxy_status,
            commands::telegram_status,
            commands::apps_traffic,
            commands::apps_open_extension,
            commands::privacy_check,
            commands::update_check,
            commands::update_install,
            commands::update_latest,
            commands::update_toast_open,
            commands::update_toast_close,
            commands::whats_new_take,
            commands::play_chime,
            commands::browser_policy_status,
            commands::browser_policy_set,
            commands::leak_guard_status,
            commands::leak_guard_set,
            commands::ui_log,
            commands::apps_search,
            commands::apps_icons,
            commands::apps_layout_get,
            commands::apps_settings_set,
            commands::apps_layout_set,
            commands::apps_relaunch,
            commands::telegram_setup,
            commands::claude_proxy_enable,
            commands::claude_proxy_disable,
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
            // App profiles
            commands::apps_list,
            commands::apps_set,
            commands::apps_add_custom,
            commands::apps_remove,
            commands::apps_launch,
            commands::apps_create_shortcut,
            // Killswitch
            commands::killswitch_status,
            commands::killswitch_enable,
            commands::killswitch_disable,
            // Global env (главный выключатель)
            commands::global_proxy_env,
            commands::set_global_proxy_env,
            commands::clear_global_proxy_env,
            // Автозапуск моста (Планировщик)
            commands::bridge_task_status,
            commands::bridge_task_register,
            // Мост: здоровье и маршрутизация
            commands::bridge_health,
            commands::bridge_exe_legacy,
            commands::bridge_versions,
            commands::autostart_get,
            commands::autostart_set,
            commands::bridge_set_route_mode,
            commands::bridge_update,
            commands::wizard_install,
            // Прокси-пул
            commands::proxies_get,
            commands::proxies_set,
            commands::proxy_assignments_get,
            commands::proxy_assignments_set,
            commands::sites_get,
            commands::sites_set,
            commands::sites_normalize,
            commands::sites_status,
            commands::sites_hosts,
            commands::sites_peek,
            commands::system_pac_status,
            commands::system_pac_set,
            commands::proxy_labels_get,
            commands::proxy_label_set,
            commands::proxies_ping,
            // Окна (трей-попап)
            commands::show_main_window,
            commands::quit_app,
        ])
        .setup(|app| {
            let handles = setup_tray(app.handle())?;

            // Трей-монитор: живая иконка (зелёная/жёлтая/красная точка),
            // tooltip со счётчиками и системные уведомления о переходах
            // (мост упал/ожил, upstream лежит, высокий пинг).
            monitor::spawn(app.handle().clone(), handles);

            // Watchdog: авто-подъём упавшего шима (корневой фикс «деплой роняет
            // мост → Claude Code теряет связь»). Фоновый поток, переживает любые
            // ошибки, поднимает шим за ~5с после падения. Второй эшелон после
            // RestartOnFailure Планировщика (тот работает и без нашего GUI).
            shim::spawn_watchdog();

            // Миграция старых установок: .vbs в Startup → задача Планировщика
            // с автоперезапуском. Фоново — PS-вызовы медленные, UI не ждёт.
            std::thread::spawn(tasks::migrate_from_startup_vbs);

            // Мост обновляется вместе с приложением: встроенный новее
            // установленного — тихо заменить (перезапуск моста ~1 с).
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    commands::auto_update_bridge(handle).await;
                });
            }

            // Фоновая проверка обновлений: новая версия — уведомление в углу
            // экрана со звуком (одно на версию).
            update_notify::spawn_watcher(app.handle().clone());

            // macOS: агент переменных старого формата (`/bin/sh -c`, виден в
            // «Объектах входа» как безымянный «sh») → `clod-helper --apply-env`.
            #[cfg(not(target_os = "windows"))]
            std::thread::spawn(env_proxy::migrate_legacy_agent);

            // Автозапуск плагина («Clod Helper.plist») → SMAppService.
            std::thread::spawn(autostart::migrate_legacy);

            // «Сайты во всех браузерах»: снять зависшую запись «Подсмотреть»
            // (окно закрыли посреди неё) и освежить метку PAC.
            std::thread::spawn(system_pac::on_startup);

            // Если запущены с флагом --tray (после автозапуска Windows) —
            // прячем главное окно при старте, видна только иконка в трее.
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--tray") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
                set_dock_visible(app.handle(), false);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    // Close-to-tray: крестик прячет окно, не закрывает приложение.
                    // Полный выход — только через меню трея "Выход".
                    api.prevent_close();
                    let _ = window.hide();
                    if window.label() == "main" {
                        set_dock_visible(window.app_handle(), false);
                    }
                }
                // Трей-попап: light dismiss — потерял фокус → спрятался
                // (паттерн Tailscale/WARP).
                WindowEvent::Focused(false) if window.label() == "tray" => {
                    let _ = window.hide();
                    #[cfg(target_os = "macos")]
                    mac_highlight_tray(window.app_handle(), false);
                }
                _ => {}
            }
        })
        .build(context)
        .expect("error while running tauri application")
        .run(|app, event| {
            // macOS: окно спрятано в трей (close-to-tray), а клик по иконке
            // в Dock должен вернуть его, как у любого mac-приложения.
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    show_main_window(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<monitor::TrayHandles> {
    // Нативного меню НЕТ намеренно: Windows не даёт его стилизовать, и оно
    // выглядело «как у старых приложух». Вместо него фирменный попап —
    // паттерн Cloudflare WARP. Вся функциональность меню (статус, тест,
    // тумблеры, рестарт/стоп, выход) живёт в попапе.
    // Windows: правый щелчок — попап, двойной левый — главное окно (привычная
    // схема трея). Одиночный левый ничего не делает: иначе перед главным
    // окном мелькал бы попап от первого щелчка двойного.
    // macOS: значок в строке меню — любой щелчок открывает попап.
    let builder = TrayIconBuilder::with_id("main-tray");
    // macOS: сразу монохромная template-иконка (цветная иконка приложения в
    // строке меню выглядит чужеродно); дальше её ведёт monitor.
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(monitor::MAC_TEMPLATE)?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.icon(app.default_window_icon().unwrap().clone());
    let tray = builder
        .tooltip("Clod Helper")
        .on_tray_icon_event(|tray, event| match event {
            #[cfg(target_os = "windows")]
            TrayIconEvent::DoubleClick {
                button: tauri::tray::MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            #[cfg(target_os = "windows")]
            TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Right,
                button_state: MouseButtonState::Up,
                position,
                rect,
                ..
            } => show_tray_popup(tray.app_handle(), position.x, position.y, rect),
            #[cfg(not(target_os = "windows"))]
            TrayIconEvent::Click {
                button_state: MouseButtonState::Up,
                position,
                rect,
                ..
            } => show_tray_popup(tray.app_handle(), position.x, position.y, rect),
            _ => {}
        })
        .build(app)?;

    Ok(monitor::TrayHandles { tray })
}

/// Показать трей-попап у точки клика (physical px). Позиция: над курсором,
/// правый край примерно под иконкой (таскбар Windows обычно снизу справа);
/// клампится в границы монитора — на случай таскбара сверху/сбоку.
/// macOS: окно попапа — неактивирующая панель, как системные меню строки
/// меню. Обычное окно при показе активирует приложение, а macOS при активации
/// уводит пользователя в пространство с главным окном Clod Helper — поверх
/// полноэкранного приложения меню не появлялось. Поэтому класс окна меняется
/// на подкласс NSPanel (canBecomeKeyWindow = YES), стиль — nonactivatingPanel;
/// плюс: на всех рабочих столах, поверх полноэкранных, уровень всплывающего меню.
#[cfg(target_os = "macos")]
fn mac_popup_window(win: &tauri::WebviewWindow) -> bool {
    use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
    use objc2::{msg_send, sel};
    use std::sync::OnceLock;

    let Ok(ptr) = win.ns_window() else { return false };
    if ptr.is_null() {
        return false;
    }
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const STATIONARY: usize = 1 << 4;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
    const NONACTIVATING_PANEL: usize = 1 << 7;
    const POP_UP_MENU_LEVEL: isize = 101;

    static PANEL: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    let panel = *PANEL.get_or_init(|| {
        extern "C-unwind" fn yes(_: &AnyObject, _: Sel) -> Bool {
            Bool::YES
        }
        let sup = AnyClass::get(c"NSPanel")?;
        let mut b = ClassBuilder::new(c"ClodHelperTrayPanel", sup)?;
        // SAFETY: сигнатура совпадает с -[NSWindow canBecomeKeyWindow].
        unsafe {
            b.add_method(sel!(canBecomeKeyWindow), yes as extern "C-unwind" fn(_, _) -> _);
        }
        Some(b.register())
    });
    let Some(panel) = panel else { return false };

    // SAFETY: ns_window() — живой NSWindow этого окна; вызов на главном потоке
    // (обработчик событий трея исполняется в цикле событий). Подкласс NSPanel
    // не добавляет ivar'ов, поэтому смена класса безопасна (так делает и
    // tauri-nspanel); делегат окна (события фокуса) остаётся прежним.
    unsafe {
        let w = &*(ptr as *const AnyObject);
        if !std::ptr::eq(w.class(), panel) {
            AnyObject::set_class(w, panel);
        }
        let mask: usize = msg_send![w, styleMask];
        let _: () = msg_send![w, setStyleMask: mask | NONACTIVATING_PANEL];
        let _: () = msg_send![w, setHidesOnDeactivate: false];
        let behavior = CAN_JOIN_ALL_SPACES | STATIONARY | FULL_SCREEN_AUXILIARY;
        let _: () = msg_send![w, setCollectionBehavior: behavior];
        let _: () = msg_send![w, setLevel: POP_UP_MENU_LEVEL];
    }
    true
}

/// macOS: значок в строке меню «нажат», пока открыт попап — как у системных
/// меню; вместе с панелью под ним это читается как одно целое.
#[cfg(target_os = "macos")]
fn mac_highlight_tray(app: &tauri::AppHandle, on: bool) {
    let Some(tray) = app.tray_by_id("main-tray") else { return };
    let _ = tray.with_inner_tray_icon(move |t| {
        let Some(item) = t.ns_status_item() else { return };
        // SAFETY: живой NSStatusItem трея; вызов на главном потоке.
        unsafe {
            let item = &*(objc2::rc::Retained::as_ptr(&item) as *const objc2::runtime::AnyObject);
            let button: *const objc2::runtime::AnyObject = objc2::msg_send![item, button];
            if let Some(button) = button.as_ref() {
                let _: () = objc2::msg_send![button, highlight: on];
            }
        }
    });
}

/// macOS: показать панель поверх всего и сделать её ключевой — без
/// активации приложения (никакого переключения рабочих столов).
#[cfg(target_os = "macos")]
fn mac_show_panel(win: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let Ok(ptr) = win.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    // SAFETY: см. mac_popup_window.
    unsafe {
        let w = &*(ptr as *const AnyObject);
        let _: () = msg_send![w, orderFrontRegardless];
        let _: () = msg_send![w, makeKeyWindow];
    }
}

fn show_tray_popup(app: &tauri::AppHandle, cx: f64, cy: f64, icon: tauri::Rect) {
    // Окно объявлено в tauri.conf.json и создаётся при старте; если его нет
    // (создание упало) — создаём на лету, иначе клик по трею «молчит».
    let win = match app.get_webview_window("tray") {
        Some(w) => w,
        None => {
            match tauri::WebviewWindowBuilder::new(
                app,
                "tray",
                tauri::WebviewUrl::App("index.html?view=tray".into()),
            )
            .title("Clod Helper")
            .inner_size(332.0, 404.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .build()
            {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("не удалось создать окно трей-попапа: {e}");
                    return;
                }
            }
        }
    };
    // Повторный клик при открытом попапе: blur уже спрятал его до того, как
    // пришёл Click, так что здесь попап почти всегда скрыт — просто покажем.
    // Позиция над курсором + кламп в границы монитора клика.
    let place = |w: f64, h: f64| -> (f64, f64) {
        let mut x = cx - w + 24.0;
        let mut y = cy - h - 8.0;
        if let Ok(monitors) = win.available_monitors() {
            let m = monitors.iter().find(|m| {
                let p = m.position();
                let s = m.size();
                cx >= p.x as f64
                    && cx < p.x as f64 + s.width as f64
                    && cy >= p.y as f64
                    && cy < p.y as f64 + s.height as f64
            });
            if let Some(m) = m {
                let (mx, my) = (m.position().x as f64, m.position().y as f64);
                let (mw, mh) = (m.size().width as f64, m.size().height as f64);
                x = x.clamp(mx + 8.0, (mx + mw - w - 8.0).max(mx + 8.0));
                // Таскбар сверху: клик в верхней трети экрана → попап под курсором.
                if y < my + 8.0 {
                    y = cy + 16.0;
                }
                y = y.clamp(my + 8.0, (my + mh - h - 8.0).max(my + 8.0));
            }
        }
        (x, y)
    };

    // macOS: значок — в строке меню сверху. Попап висит прямо под ним, по
    // центру значка, как системные меню; поверх всех рабочих столов и
    // полноэкранных приложений (иначе macOS открывал его на «своём» столе).
    #[cfg(target_os = "macos")]
    {
        let _ = (cx, cy, &place);
        let panel = mac_popup_window(&win);
        let scale = win.scale_factor().unwrap_or(2.0);
        let ip: tauri::PhysicalPosition<f64> = icon.position.to_physical(scale);
        let is: tauri::PhysicalSize<f64> = icon.size.to_physical(scale);
        let w = win.outer_size().map(|s| s.width as f64).unwrap_or(332.0 * scale);
        let mut x = ip.x + is.width / 2.0 - w / 2.0;
        // Вплотную к строке меню: значок занимает её по высоте, а зазор в
        // пару точек даёт сама панель (отступ сверху в TrayPopup).
        let y = ip.y + is.height;
        if let Ok(Some(m)) = win.monitor_from_point(ip.x, ip.y) {
            let (mx, mw) = (m.position().x as f64, m.size().width as f64);
            x = x.clamp(mx + 6.0 * scale, (mx + mw - w - 6.0 * scale).max(mx));
        }
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
        if panel {
            mac_show_panel(&win);
            mac_highlight_tray(app, true);
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
    // Windows: трей внизу — попап над курсором, клампится в монитор клика.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = icon;
        let size = win.outer_size().unwrap_or(tauri::PhysicalSize::new(332, 424));
        let (x, y) = place(size.width as f64, size.height as f64);
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
        // Смешанные DPI: перенос окна на монитор с другим масштабом меняет его
        // physical-размер — первый кламп считался по старому. Перемеряем и, если
        // размер сменился, позиционируем ещё раз по фактическому.
        if let Ok(after) = win.outer_size() {
            if after.width != size.width || after.height != size.height {
                let (x2, y2) = place(after.width as f64, after.height as f64);
                let _ = win.set_position(tauri::PhysicalPosition::new(x2, y2));
            }
        }
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// macOS: иконка в Dock — только пока видно главное окно (как у менюбар-
/// утилит). Без этого после автозапуска с --tray в Dock висела иконка без окна.
pub(crate) fn set_dock_visible(app: &tauri::AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_dock_visibility(visible);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible);
    }
}

/// Показать главное окно (меню трея, Dock, кнопка в трей-попапе).
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        set_dock_visible(app, true);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}
