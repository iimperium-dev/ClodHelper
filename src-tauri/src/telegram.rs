//! Telegram через мост (macOS и Windows).
//!
//! Telegram для macOS (ru.keepcoder.Telegram) не читает HTTP_PROXY и не умеет
//! HTTP-прокси вовсе - только SOCKS5/MTProto. Поэтому мост (≥1.6) держит
//! SOCKS5-вход 127.0.0.1:8891, где всё идёт только через upstream.
//!
//! Записать прокси в настройки Telegram снаружи нельзя (они в его базе), но
//! есть штатная ссылка `tg://socks?server=…&port=…`: Telegram сам показывает
//! окно «Добавить прокси», пользователь жмёт «Подключить». То же понимает и
//! Telegram Desktop (Qt-версия).
//!
//! Факт «идёт ли через мост» берём не из настроек, а из живых соединений
//! процесса (lsof; на Windows - netstat): есть сокеты к 127.0.0.1:8891 -
//! через мост; есть сокеты наружу мимо него - напрямую.

use serde::Serialize;

/// SOCKS5-вход моста для Telegram (см. bridge --socks-listen).
pub const SOCKS_PORT: u16 = 8891;

#[derive(Debug, Serialize, Default)]
pub struct TelegramStatus {
    /// Настройку можно выполнить (macOS - всегда, Windows - найден Telegram Desktop).
    pub supported: bool,
    /// Путь к Telegram.app / Telegram Desktop.app, если установлен.
    pub app_path: Option<String>,
    pub running: bool,
    /// Мост слушает SOCKS-вход (мост ≥1.6).
    pub socks_ready: bool,
    /// Соединений Telegram через мост прямо сейчас.
    pub via_bridge: u32,
    /// Соединений Telegram наружу мимо моста (не loopback).
    pub direct: u32,
    /// Защита от утечек закрывает Telegram прямой выход: без прокси в самом
    /// Telegram он не подключится.
    pub guarded: bool,
}

#[cfg(not(target_os = "windows"))]
pub fn status() -> TelegramStatus {
    let app_path = crate::macos::find_app(&["Telegram.app", "Telegram Desktop.app"])
        .map(|p| p.to_string_lossy().to_string());
    let (running, via_bridge, direct) = connections();
    TelegramStatus {
        supported: true,
        app_path,
        running,
        socks_ready: socks_listening(),
        via_bridge,
        direct,
        guarded: crate::leak_guard::guards("telegram"),
    }
}

/// Windows: Telegram Desktop, соединения - из общего снимка (netstat). Путь -
/// из профиля (его видит и меняет пользователь), иначе детект.
#[cfg(target_os = "windows")]
pub fn status() -> TelegramStatus {
    let profile = crate::settings::load_with_presets()
        .app_profiles
        .into_iter()
        .filter(|p| p.id == crate::apps::TELEGRAM_ID)
        .collect::<Vec<_>>();
    let app_path = profile
        .first()
        .and_then(crate::apps::profile_file)
        .or_else(|| crate::apps::find_telegram_exe().map(|p| p.to_string_lossy().to_string()));
    let t = crate::traffic::snapshot(&profile).into_iter().next().unwrap_or_default();
    TelegramStatus {
        supported: app_path.is_some(),
        app_path,
        running: t.running,
        socks_ready: socks_listening(),
        via_bridge: t.via_bridge,
        direct: t.direct,
        guarded: crate::leak_guard::guards("telegram"),
    }
}

/// Сколько ждать, пока пользователь нажмёт «Подключить» в окне Telegram.
const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(90);

/// «Подключить» (действие пользователя): окно «Добавить прокси» в самом
/// Telegram с нашим SOCKS-входом, ожидание, пока Telegram пойдёт через мост,
/// и только потом защита от утечек.
///
/// Порядок важен: защита закрывает Telegram прямой выход. Включённая раньше,
/// чем в Telegram подтверждён прокси, она оставляла его без связи
/// («Соединение…»), если окно Telegram закрыли или не заметили.
pub fn setup() -> Result<TelegramStatus, String> {
    let st = ready()?;
    if st.via_bridge == 0 {
        open_proxy_window(st.app_path.as_deref().unwrap_or_default())?;
        mark_prompted();
        wait_via_bridge()?;
    }
    // Telegram уже идёт через мост - теперь защита ему не мешает. Одно окно
    // прав администратора; отказ не мешает подключению - защиту можно
    // включить позже в настройках Telegram.
    let _ = crate::leak_guard::add("telegram");
    Ok(status())
}

/// Ждать, пока у Telegram появятся соединения через мост.
fn wait_via_bridge() -> Result<(), String> {
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if status().via_bridge > 0 {
            return Ok(());
        }
    }
    Err(if crate::leak_guard::guards("telegram") {
        // Защита осталась с прошлой настройки: без прокси Telegram без связи.
        "Telegram не подключился к мосту - в его окне «Прокси» нажмите «Подключить». \
         Пока прокси не подключён, Telegram без связи: в его настройках можно выключить «Не пускать мимо прокси»"
    } else {
        "Telegram не подключился к мосту - в его окне «Прокси» нажмите «Подключить». \
         Прямой доступ Telegram не закрыт"
    }
    .into())
}

/// Telegram найден и мост принимает SOCKS - можно открывать окно прокси.
fn ready() -> Result<TelegramStatus, String> {
    let st = status();
    if st.app_path.is_none() {
        return Err(if cfg!(target_os = "windows") {
            "Telegram Desktop не найден. Установите его и повторите."
        } else {
            "Telegram не найден в «Программах». Установите его и повторите."
        }
        .into());
    }
    if !st.socks_ready {
        return Err(
            "Мост ещё не умеет SOCKS для Telegram - нажмите «Обновить мост» в Обзоре.".into(),
        );
    }
    Ok(st)
}

/// Окно «Добавить прокси» в Telegram по штатной ссылке tg://socks.
#[cfg(not(target_os = "windows"))]
fn open_proxy_window(_app: &str) -> Result<(), String> {
    let url = format!("tg://socks?server=127.0.0.1&port={SOCKS_PORT}");
    let ok = std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Не удалось открыть Telegram по ссылке tg://socks".into());
    }
    Ok(())
}

/// Windows: та же ссылка, но аргументом самого Telegram - так ссылки отдаёт
/// ему и система («Telegram.exe -- %1»); запущенному Telegram её передаёт
/// новый процесс. Через `start tg://…` она ушла бы тому, кто последним
/// записался обработчиком tg://, - при втором аккаунте (копия Telegram в своей
/// папке) это копия, а не основной. Запущенные копии получают ссылку тоже:
/// защита от утечек закрывает Telegram мимо моста всем, прокси нужен каждой.
#[cfg(target_os = "windows")]
fn open_proxy_window(main: &str) -> Result<(), String> {
    let url = format!("tg://socks?server=127.0.0.1&port={SOCKS_PORT}");
    crate::win::allow_foreground();
    std::process::Command::new(main)
        .args(["--", &url])
        .spawn()
        .map_err(|e| format!("Не удалось открыть Telegram: {e}"))?;
    for copy in crate::apps::other_running_telegrams(main) {
        let _ = std::process::Command::new(copy).args(["--", &url]).spawn();
    }
    Ok(())
}

/// Запомнить, что окно прокси Telegram уже показывали.
fn mark_prompted() {
    let _ = crate::settings::with_settings_strict(|s| {
        s.telegram_prompted = true;
        Ok(())
    });
}

/// Автонастройка: один раз, когда Telegram впервые замечен идущим напрямую
/// при готовом SOCKS-входе, сами открываем его окно «Добавить прокси». Telegram
/// не позволяет менять прокси снаружи - нажатие «Подключить» остаётся за
/// пользователем, но искать настройку ему не нужно. Дальше не навязываемся.
/// Прав администратора (защита от утечек) здесь не просим: их окно - только
/// по действию пользователя («Подключить»).
///
/// Монитор зовёт её раз в минуту. Проверки - от дешёвых к дорогим: полный
/// статус (детект программ, перебор процессов, таблица соединений) - только
/// когда Telegram установлен и запущен, а мост принимает SOCKS. Вопрос
/// закрыт до конца работы программы, если окно уже показывали или Telegram
/// уже ходит через мост. Telegram не найден - следующая проверка через
/// NOT_FOUND_RECHECK (вдруг его поставят).
pub fn auto_prompt_once() {
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    if AUTO_DONE.load(Ordering::Relaxed) {
        return;
    }
    let mut next = AUTO_NEXT.lock().unwrap_or_else(|e| e.into_inner());
    if next.is_some_and(|t| Instant::now() < t) {
        return;
    }
    *next = None;
    let s = crate::settings::load();
    if s.telegram_prompted {
        AUTO_DONE.store(true, Ordering::Relaxed);
        return;
    }
    let Some(app_path) = installed_path(&s) else {
        *next = Some(Instant::now() + NOT_FOUND_RECHECK);
        return;
    };
    if !socks_listening() || !running_now() {
        return;
    }
    let st = status();
    match prompt_decision(&st) {
        Prompt::Closed => AUTO_DONE.store(true, Ordering::Relaxed),
        Prompt::Wait => {}
        Prompt::Show => {
            let path = st.app_path.unwrap_or(app_path);
            if open_proxy_window(&path).is_ok() {
                mark_prompted();
                AUTO_DONE.store(true, Ordering::Relaxed);
                tracing::info!("Telegram идёт мимо моста - показал окно «Добавить прокси»");
            }
        }
    }
}

/// Telegram не найден: искать снова не раньше чем через столько.
const NOT_FOUND_RECHECK: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// Автопредложение больше не нужно (уже показывали / Telegram через мост).
static AUTO_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Когда проверять снова (Telegram не найден).
static AUTO_NEXT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

#[derive(Debug, PartialEq)]
enum Prompt {
    /// Показать окно «Добавить прокси».
    Show,
    /// Пока не о чем говорить (не запущен, нет соединений, мост без SOCKS).
    Wait,
    /// Telegram уже ходит через мост - предлагать больше нечего.
    Closed,
}

fn prompt_decision(st: &TelegramStatus) -> Prompt {
    if st.via_bridge > 0 {
        Prompt::Closed
    } else if st.app_path.is_none() || !st.running || !st.socks_ready || st.direct == 0 {
        Prompt::Wait
    } else {
        Prompt::Show
    }
}

/// Путь к Telegram без детекта всех программ: профиль из настроек, иначе
/// поиск самого Telegram (у него свой кэш).
#[cfg(target_os = "windows")]
fn installed_path(s: &crate::settings::Settings) -> Option<String> {
    s.app_profiles
        .iter()
        .find(|p| p.id == crate::apps::TELEGRAM_ID)
        .and_then(crate::apps::profile_file)
        .or_else(|| crate::apps::find_telegram_exe().map(|p| p.to_string_lossy().to_string()))
}

#[cfg(not(target_os = "windows"))]
fn installed_path(_s: &crate::settings::Settings) -> Option<String> {
    crate::macos::find_app(&["Telegram.app", "Telegram Desktop.app"])
        .map(|p| p.to_string_lossy().to_string())
}

/// Запущен ли Telegram - по общему снимку процессов, без своего перебора.
/// Имя то же, по которому его узнаёт полный статус: «Telegram» («.exe» на
/// Windows).
fn running_now() -> bool {
    crate::apps::running_processes().iter().any(|p| is_telegram_process(&p.name))
}

fn is_telegram_process(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.strip_suffix(".exe").unwrap_or(&n) == "telegram"
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn socks_listening() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], SOCKS_PORT));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

/// (запущен?, соединений через мост, соединений мимо моста) по lsof.
#[cfg(not(target_os = "windows"))]
fn connections() -> (bool, u32, u32) {
    // Запущен ли - по общему снимку процессов (раньше - отдельный pgrep).
    if !running_now() {
        return (false, 0, 0);
    }
    // -a: И между -c и -i; -Fn: только поле адреса («n127.0.0.1:5->127.0.0.1:8891»).
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", "-c", "Telegram", "-iTCP", "-sTCP:ESTABLISHED", "-Fn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let (via, direct) = count_connections(&out);
    (true, via, direct)
}

/// Разбор `lsof -Fn`: строки «n<локальный>-><удалённый>».
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn count_connections(lsof_fn: &str) -> (u32, u32) {
    let bridge = format!("127.0.0.1:{SOCKS_PORT}");
    let (mut via, mut direct) = (0, 0);
    for line in lsof_fn.lines() {
        let Some(remote) = line.strip_prefix('n').and_then(|l| l.split_once("->")).map(|(_, r)| r)
        else {
            continue;
        };
        if remote == bridge {
            via += 1;
        } else if !remote.starts_with("127.") && !remote.starts_with("[::1]") {
            direct += 1;
        }
    }
    (via, direct)
}

#[cfg(test)]
mod tests {
    use super::{count_connections, is_telegram_process, prompt_decision, Prompt, TelegramStatus};

    #[test]
    fn prompt_only_when_telegram_goes_around_the_bridge() {
        let base = || TelegramStatus {
            supported: true,
            app_path: Some("Telegram.exe".into()),
            running: true,
            socks_ready: true,
            via_bridge: 0,
            direct: 3,
            guarded: false,
        };
        assert_eq!(prompt_decision(&base()), Prompt::Show);
        // Уже через мост - вопрос закрыт навсегда, даже если есть и прямые.
        assert_eq!(prompt_decision(&TelegramStatus { via_bridge: 2, ..base() }), Prompt::Closed);
        assert_eq!(prompt_decision(&TelegramStatus { direct: 0, ..base() }), Prompt::Wait);
        assert_eq!(prompt_decision(&TelegramStatus { running: false, ..base() }), Prompt::Wait);
        assert_eq!(prompt_decision(&TelegramStatus { socks_ready: false, ..base() }), Prompt::Wait);
        assert_eq!(prompt_decision(&TelegramStatus { app_path: None, ..base() }), Prompt::Wait);
    }

    #[test]
    fn telegram_process_names() {
        for n in ["Telegram.exe", "telegram.exe", "Telegram"] {
            assert!(is_telegram_process(n), "{n}");
        }
        for n in ["Telegram Helper", "Vtoroy-Acc.exe", "TelegramUpdater.exe", ""] {
            assert!(!is_telegram_process(n), "{n}");
        }
    }

    #[test]
    fn counts_bridge_and_direct_connections() {
        let sample = "p10302\nf12\nn127.0.0.1:52001->127.0.0.1:8891\nf13\n\
                      n127.0.0.1:52002->127.0.0.1:8891\nf14\n\
                      n192.168.1.5:52100->149.154.167.51:443\nf15\n\
                      n127.0.0.1:52200->127.0.0.1:8889\n";
        assert_eq!(count_connections(sample), (2, 1));
        assert_eq!(count_connections(""), (0, 0));
    }
}
