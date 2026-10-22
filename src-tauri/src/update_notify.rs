//! «Доступно обновление»: фоновая проверка и всплывающее окно-уведомление
//! в углу экрана (как у Telegram), со звуком.
//!
//! Системный тост Windows/macOS не годится: по клику на нём Tauri ничего не
//! получает, а нужно открыть окно обновления. Поэтому — своё маленькое окно
//! без рамки поверх всех, которое не забирает фокус (`focusable(false)`):
//! человек печатает в другой программе — уведомление ему не мешает.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::updater::UpdateInfo;

pub const TOAST_LABEL: &str = "toast";
/// Логический размер окна: карточка 340×88 + поля под тень.
const TOAST_W: f64 = 372.0;
const TOAST_H: f64 = 120.0;
/// Отступ от края рабочей области (над панелью задач).
const MARGIN: f64 = 8.0;

/// Первая проверка — не сразу: при входе в систему сеть поднимается не
/// мгновенно. Но и не поздно: рассылку, пропущенную с выключенным
/// компьютером, человек должен увидеть вскоре после запуска.
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

/// Последнее найденное обновление — окно-уведомление и главное окно берут
/// его отсюда, а не проверяют GitHub ещё раз.
static LATEST: Mutex<Option<UpdateInfo>> = Mutex::new(None);

pub fn latest() -> Option<UpdateInfo> {
    LATEST.lock().ok().and_then(|g| g.clone())
}

/// Запомнить результат проверки (её делают и фон, и кнопка «Проверить»).
pub fn remember(info: &Option<UpdateInfo>) {
    if let Ok(mut g) = LATEST.lock() {
        *g = info.clone();
    }
}

pub fn spawn_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            // Всплывает одно уведомление за проверку (самое важное): обновление
            // главнее рассылки; остальное ждёт в колокольчике.
            let mut toast: Option<String> = None;
            match crate::updater::check().await {
                Ok(found) => {
                    remember(&found);
                    if let Some(info) = found {
                        let _ = app.emit("update:available", &info);
                        let notified = crate::settings::load().update_notified;
                        if notified.as_deref() != Some(info.version.as_str()) {
                            toast = Some(crate::notices::update_id(&info.version));
                            let _ = crate::settings::set_update_notified(&info.version);
                        }
                    }
                }
                Err(e) => tracing::info!("фоновая проверка обновлений: {e}"),
            }
            match crate::notices::fetch().await {
                Ok(fresh) => {
                    if let Some(n) = fresh.iter().find(|n| crate::notices::worth_toast(n)) {
                        toast.get_or_insert_with(|| n.id.clone());
                    }
                    let ids: Vec<String> = fresh.iter().map(|n| n.id.clone()).collect();
                    let _ = crate::settings::mark_notices_toasted(&ids);
                }
                Err(e) => tracing::info!("рассылки: {e}"),
            }
            let _ = app.emit("notices:changed", crate::notices::unread());
            if let Some(id) = toast {
                show_toast(&app, &id);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Показать уведомление `id` (обновление «update-X» или рассылка): Windows —
/// справа внизу над панелью задач, macOS — справа вверху под строкой меню.
pub fn show_toast(app: &AppHandle, id: &str) {
    // Старое уведомление (о прошлой версии) — закрыть, новое создаётся с нуля:
    // так анимация появления проигрывается каждый раз.
    if let Some(old) = app.get_webview_window(TOAST_LABEL) {
        let _ = old.destroy();
    }
    // Окно сразу создаётся видимым, на своём месте и без фокуса. Показ потом
    // через show() активировал его (ввод уходил из программы, где человек
    // печатал), а ShowWindow в обход Tauri ломал его состояние — окну
    // возвращались рамка и заголовок. Проверено вживую на Windows 11.
    let (x, y) = corner(app);
    if let Err(e) = tauri::WebviewWindowBuilder::new(
        app,
        TOAST_LABEL,
        tauri::WebviewUrl::App(
            format!("index.html?view=toast&n={}", url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>())
                .into(),
        ),
    )
    .title("Clod Helper")
    .inner_size(TOAST_W, TOAST_H)
    .position(x, y)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(true)
    .build()
    {
        tracing::warn!("не удалось создать окно уведомления: {e}");
        return;
    }
    crate::sound::chime();
}

/// Левый верхний угол окна уведомления (логические px) на основном мониторе.
fn corner(app: &AppHandle) -> (f64, f64) {
    let Some(m) = app.primary_monitor().ok().flatten() else {
        return (100.0, 100.0);
    };
    let scale = m.scale_factor();
    let area = m.work_area();
    let (ax, ay) = (area.position.x as f64 / scale, area.position.y as f64 / scale);
    let (aw, ah) = (area.size.width as f64 / scale, area.size.height as f64 / scale);
    let x = ax + aw - TOAST_W - MARGIN;
    let y = if cfg!(target_os = "macos") { ay + MARGIN } else { ay + ah - TOAST_H - MARGIN };
    (x, y)
}

pub fn close_toast(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(TOAST_LABEL) {
        let _ = w.destroy();
    }
}

/// Клик по уведомлению: главное окно и в нём — окно обновления (для
/// «update-…») или колокольчик с этой рассылкой. Кликнутое — прочитано.
pub fn open_from_toast(app: &AppHandle, id: &str) {
    close_toast(app);
    crate::show_main_window(app);
    let _ = crate::settings::mark_notices_read(&[id.to_string()]);
    let _ = app.emit("notices:changed", crate::notices::unread());
    if id.starts_with("update-") {
        if let Some(info) = latest() {
            let _ = app.emit_to("main", "update:open", &info);
        }
    } else {
        let _ = app.emit_to("main", "notices:open", id);
    }
}
