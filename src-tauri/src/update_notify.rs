//! «Доступно обновление»: фоновая проверка и всплывающее окно-уведомление
//! в углу экрана (как у Telegram), со звуком.
//!
//! Системный тост Windows/macOS не годится: по клику на нём Tauri ничего не
//! получает, а нужно открыть окно обновления. Поэтому - своё маленькое окно
//! без рамки поверх всех, которое не забирает фокус (`focusable(false)`):
//! человек печатает в другой программе - уведомление ему не мешает.

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

/// Первая проверка - не сразу: при входе в систему сеть поднимается не
/// мгновенно. Но и не поздно: рассылку, пропущенную с выключенным
/// компьютером, человек должен увидеть вскоре после запуска.
const FIRST_CHECK: Duration = Duration::from_secs(15);
/// Первая проверка обновлений - со случайной задержкой 0-10 минут: при
/// большой аудитории все не должны прийти на GitHub в одну минуту после
/// входа в систему.
const FIRST_UPDATE_SPREAD: u64 = 10 * 60;
/// Обновления - каждые 30 минут ±5 (разброс по тем же причинам).
/// В 0.8.4 было 6 часов, и вышедшую версию ждали полдня.
const CHECK_EVERY: u64 = 30 * 60;
const CHECK_SPREAD: u64 = 5 * 60;
/// По открытию окна обновления проверяем не чаще раза в 10 минут.
const UPDATE_MIN_GAP: Duration = Duration::from_secs(10 * 60);
/// Рассылки - при старте и дальше раз в 3 часа ±20 минут: при росте
/// аудитории запрос каждые 10 минут от каждого был бы лишней нагрузкой.
/// Открыл окно - проверка по фокусу (не чаще NOTICES_MIN_GAP).
const NOTICES_EVERY: u64 = 3 * 3600;
const NOTICES_SPREAD: u64 = 20 * 60;
/// Не чаще раза в минуту по фокусу окна (открыл-закрыл - не повод качать).
const NOTICES_MIN_GAP: Duration = Duration::from_secs(60);

/// Последнее найденное обновление - окно-уведомление и главное окно берут
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

/// Случайное число секунд в [base - spread, base + spread] из случайного u64.
fn jittered_secs(base: u64, spread: u64, rnd: u64) -> u64 {
    base - spread + rnd % (2 * spread + 1)
}

/// Случайная пауза base ± spread (секунды).
fn jitter(base: u64, spread: u64) -> Duration {
    let rnd = u64::from_str_radix(&crate::updater::random_hex()[..16], 16).unwrap_or(0);
    Duration::from_secs(jittered_secs(base, spread, rnd))
}

pub fn spawn_watcher(app: AppHandle) {
    // Обновления: первая проверка через 0-10 минут, дальше раз в 30 ±5 минут.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let first = u64::from_str_radix(&crate::updater::random_hex()[..16], 16).unwrap_or(0) % (FIRST_UPDATE_SPREAD + 1);
        tokio::time::sleep(Duration::from_secs(first)).await;
        loop {
            check_update(&handle, true).await;
            tokio::time::sleep(jitter(CHECK_EVERY, CHECK_SPREAD)).await;
        }
    });
    // Рассылки: при старте, дальше раз в 3 часа ±20 минут (и по открытию
    // окна - refresh_notices).
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK + Duration::from_secs(20)).await;
        loop {
            refresh_notices(&app, true).await;
            tokio::time::sleep(jitter(NOTICES_EVERY, NOTICES_SPREAD)).await;
        }
    });
}

static UPDATE_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Проверить обновление; новое - всплыть в углу (один раз на версию).
/// `forced` - фоновый цикл; по фокусу окна - не чаще UPDATE_MIN_GAP.
pub async fn check_update(app: &AppHandle, forced: bool) {
    if let Ok(mut at) = UPDATE_AT.lock() {
        if !forced && at.is_some_and(|t| t.elapsed() < UPDATE_MIN_GAP) {
            return;
        }
        *at = Some(std::time::Instant::now());
    }
    match crate::updater::check(crate::updater::Mode::Background).await {
        Ok(found) => {
            remember(&found);
            if let Some(info) = found {
                let _ = app.emit("update:available", &info);
                let s = crate::settings::load();
                // Автообновление включено: версию скачает и поставит
                // crate::auto_update, всплывающее уведомление не нужно.
                // Не получится (три неудачи, ручная установка) - обычное.
                if crate::auto_update::handles_silently(&s, &info, crate::auto_update::now_secs()) {
                    crate::auto_update::on_update_found(app);
                } else if s.update_notified.as_deref() != Some(info.version.as_str()) {
                    let _ = crate::settings::set_update_notified(&info.version);
                    show_toast(app, &crate::notices::update_id(&info.version));
                }
            }
            let _ = app.emit("notices:changed", crate::notices::unread());
        }
        Err(e) => tracing::info!("проверка обновлений: {e}"),
    }
}

/// Последняя проверка рассылок - чтобы фокус окна не качал файл каждый раз.
static NOTICES_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Скачать рассылки; новое свежее - всплыть в углу (если `toast`). По фокусу
/// окна вызывается без всплытия: человек и так смотрит в программу -
/// колокольчик загорится сам.
pub async fn refresh_notices(app: &AppHandle, toast: bool) {
    if let Ok(mut at) = NOTICES_AT.lock() {
        if !toast && at.is_some_and(|t| t.elapsed() < NOTICES_MIN_GAP) {
            return;
        }
        *at = Some(std::time::Instant::now());
    }
    match crate::notices::fetch().await {
        Ok(fresh) => {
            let ids: Vec<String> = fresh.iter().map(|n| n.id.clone()).collect();
            let _ = crate::settings::mark_notices_toasted(&ids);
            let _ = app.emit("notices:changed", crate::notices::unread());
            if toast {
                if let Some(n) = fresh.iter().find(|n| crate::notices::worth_toast(n)) {
                    show_toast(app, &n.id);
                }
            }
        }
        Err(e) => tracing::info!("рассылки: {e}"),
    }
}

/// Показать уведомление `id` (обновление «update-X» или рассылка): Windows -
/// справа внизу над панелью задач, macOS - справа вверху под строкой меню.
pub fn show_toast(app: &AppHandle, id: &str) {
    show_toast_with(app, id, &[]);
}

/// Уведомление со своим текстом (не из колокольчика): заголовок и строка
/// под ним уходят в адрес страницы. Так показываются «vpn-…».
pub fn show_toast_text(app: &AppHandle, id: &str, title: &str, sub: &str) {
    show_toast_with(app, id, &[("t", title), ("s", sub)]);
}

fn show_toast_with(app: &AppHandle, id: &str, extra: &[(&str, &str)]) {
    // «Не беспокоить» в колокольчике: в углу ничего не всплывает, а сама
    // запись остаётся в колокольчике.
    if crate::settings::notices_quiet(&crate::settings::load(), crate::auto_update::now_secs()) {
        return;
    }
    let query = {
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair("view", "toast").append_pair("n", id);
        for (k, v) in extra {
            q.append_pair(k, v);
        }
        q.finish()
    };
    let (x, y) = corner(app);
    // macOS: окно уведомления - подменённая панель (mac_popup_window), и
    // уничтожать его нельзя: при удалении окна с подменённым классом AppKit
    // падает (KVO, «Cannot remove an observer», abort) - так 0.9.3 падал по
    // клику на уведомление. Окно живёт до выхода, как меню значка: прячем
    // и показываем заново с новой страницей - анимация проигрывается снова.
    #[cfg(target_os = "macos")]
    if let Some(w) = app.get_webview_window(TOAST_LABEL) {
        match w.url() {
            Ok(mut u) => {
                u.set_query(Some(&query));
                let _ = w.navigate(u);
                let _ = w.set_position(tauri::LogicalPosition::new(x, y));
                let w2 = w.clone();
                let _ = w.run_on_main_thread(move || crate::mac_order_front(&w2));
                crate::sound::chime();
                report_shown(id);
            }
            Err(e) => tracing::warn!("не удалось показать уведомление: {e}"),
        }
        return;
    }
    // Windows: старое уведомление закрыть, новое создаётся с нуля - так
    // анимация появления проигрывается каждый раз.
    #[cfg(not(target_os = "macos"))]
    if let Some(old) = app.get_webview_window(TOAST_LABEL) {
        let _ = old.destroy();
    }
    // Окно сразу создаётся видимым, на своём месте и без фокуса. Показ потом
    // через show() активировал его (ввод уходил из программы, где человек
    // печатал), а ShowWindow в обход Tauri ломал его состояние - окну
    // возвращались рамка и заголовок. Проверено вживую на Windows 11.
    match tauri::WebviewWindowBuilder::new(app, TOAST_LABEL, tauri::WebviewUrl::App(format!("index.html?{query}").into()))
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
    // macOS: сначала невидимым - класс и уровень окна меняются до показа.
    .visible(!cfg!(target_os = "macos"))
    .build()
    {
        Ok(_win) => {
            // macOS: неактивирующая панель на всех рабочих столах и поверх
            // полноэкранных приложений (как меню значка) - уведомление видно
            // где угодно и не переключает пользователя на стол Clod Helper.
            // AppKit - только из главного потока (показ идёт из фоновой задачи).
            #[cfg(target_os = "macos")]
            {
                let w = _win.clone();
                let _ = _win.run_on_main_thread(move || {
                    crate::mac_popup_window(&w);
                    crate::mac_order_front(&w);
                });
            }
        }
        Err(e) => {
            tracing::warn!("не удалось создать окно уведомления: {e}");
            return;
        }
    }
    crate::sound::chime();
    report_shown(id);
}

/// Статистика рассылок: уведомление всплыло. Обновления и свои уведомления
/// программы («vpn-…») - не рассылки, их не считаем.
fn report_shown(id: &str) {
    if !id.starts_with("vpn-") {
        crate::telemetry::report_notice_event(id, "shown");
    }
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
        // macOS: только спрятать - см. show_toast.
        #[cfg(target_os = "macos")]
        let _ = w.hide();
        #[cfg(not(target_os = "macos"))]
        let _ = w.destroy();
    }
}

/// Клик по уведомлению: главное окно и в нём - окно обновления (для
/// «update-…») или колокольчик с этой рассылкой. Кликнутое - прочитано.
pub fn open_from_toast(app: &AppHandle, id: &str) {
    close_toast(app);
    crate::show_main_window(app);
    // Свои уведомления программы: VPN не пускает мимо себя - окно с тем,
    // как добавить исключение; обошли - просто главное окно.
    if id.starts_with("vpn-") {
        if id == "vpn-blocked" {
            let _ = app.emit_to("main", "vpn:help", ());
        }
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_stays_inside_the_window() {
        for rnd in [0u64, 1, 599, 600, 601, u64::MAX, 123_456_789_012] {
            let s = jittered_secs(CHECK_EVERY, CHECK_SPREAD, rnd);
            assert!((CHECK_EVERY - CHECK_SPREAD..=CHECK_EVERY + CHECK_SPREAD).contains(&s), "{s}");
            let n = jittered_secs(NOTICES_EVERY, NOTICES_SPREAD, rnd);
            assert!((NOTICES_EVERY - NOTICES_SPREAD..=NOTICES_EVERY + NOTICES_SPREAD).contains(&n));
        }
        assert_eq!(jittered_secs(1800, 300, 0), 1500);
        assert_eq!(jittered_secs(1800, 300, 600), 2100);
        // Живой разброс: за 40 бросков не одно и то же значение.
        let set: std::collections::HashSet<u64> = (0..40).map(|_| jitter(CHECK_EVERY, CHECK_SPREAD).as_secs()).collect();
        assert!(set.len() > 10);
    }
}
