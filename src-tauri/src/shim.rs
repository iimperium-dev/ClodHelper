//! Управление шимом `local-proxy.py` — статус, старт/стоп/рестарт, ping-тест.
//!
//! Шим — это Python-процесс который слушает 127.0.0.1:8889 и форвардит трафик в
//! купленный HTTP-прокси с автоматической авторизацией. Устанавливается через
//! `claude-desktop-setup.ps1`, автозапускается через .vbs в Startup-папке.
//!
//! Этот модуль читает рантайм-состояние:
//! - какой PID Python-процесса слушает 8889
//! - какой upstream-прокси и listen-порт прокидываются в argv
//! - uptime процесса
//! - живой ли HTTP-ответ через сам шим
//!
//! Старт/стоп используют тот же .vbs-механизм что и инсталлятор — это
//! сохраняет инвариант "после ребута шим запустится так же как сейчас".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Пользователь явно остановил мост (кнопка «Остановить»/меню трея). Пока флаг
/// взведён, watchdog НЕ поднимает мост — иначе ручная остановка жила бы 5 сек
/// и кнопка была бы бесполезной. Любой явный start() снимает флаг; при старте
/// приложения флаг чист → авто-подъём работает как обычно.
static MANUALLY_STOPPED: AtomicBool = AtomicBool::new(false);

/// Мост остановлен пользователем намеренно? (для трей-монитора: не пугать
/// уведомлением «мост упал», когда человек сам нажал «Остановить».)
pub(crate) fn manually_stopped() -> bool {
    MANUALLY_STOPPED.load(Ordering::Relaxed)
}

pub const LISTEN_PORT: u16 = 8889;
/// Легаси python-мост (для миграции/детекта старых установок).
const SHIM_FILENAME: &str = "local-proxy.py";
/// Новый мост — самостоятельный exe (НЕ python.exe). Критично: killswitch
/// блокирует python.exe, а мост на python.exe душил сам себя. exe-мост под
/// своим именем не попадает под блок → killswitch не рубит собственный мост.
#[cfg(target_os = "windows")]
pub const BRIDGE_EXE: &str = "clod-bridge.exe";
#[cfg(not(target_os = "windows"))]
pub const BRIDGE_EXE: &str = "clod-bridge";
const INSTALL_DIR_REL: &str = "ClaudeDesktopProxy";
#[cfg(target_os = "windows")]
const STARTUP_VBS_NAME: &str = "ClaudeDesktopProxy.vbs";

/// Статус шима: жив или нет, и метаданные о процессе если жив.
#[derive(Debug, Clone, Serialize)]
pub struct ShimStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_sec: Option<u64>,
    pub listen: Option<String>,
    /// Upstream URL с замаскированным паролем (`http://login:****@host:port`).
    pub upstream_masked: Option<String>,
    pub upstream_host: Option<String>,
    pub upstream_port: Option<u16>,
}

impl ShimStatus {
    fn down() -> Self {
        Self {
            running: false,
            pid: None,
            uptime_sec: None,
            listen: None,
            upstream_masked: None,
            upstream_host: None,
            upstream_port: None,
        }
    }
}

/// Результат "Тест" — HTTP-запрос через шим до публичного echo-сервиса.
#[derive(Debug, Clone, Serialize)]
pub struct ShimTestResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    /// IP-адрес что виден интернету (после прохождения через шим+upstream).
    /// Помогает увидеть страну выходного прокси.
    pub external_ip: Option<String>,
}

// ─── Status ──────────────────────────────────────────────────────

/// Найти python-процесс который запущен с `local-proxy.py` и собрать статус.
pub fn status() -> ShimStatus {
    let Ok(mut sys) = process_table().lock() else {
        return ShimStatus::down();
    };
    refresh_process_table(&mut sys);

    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_lowercase();
        // Новый мост: clod-bridge.exe. Легаси: python*.exe с local-proxy.py.
        let is_bridge_exe = name == BRIDGE_EXE;
        let is_legacy_python = name.starts_with("python") || name.starts_with("py.exe");
        if !is_bridge_exe && !is_legacy_python {
            continue;
        }
        let cmdline_parts: Vec<String> = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let cmdline = cmdline_parts.join(" ");
        // exe-мост опознаём по имени; python-мост — по наличию local-proxy.py в argv.
        if is_legacy_python && !cmdline.contains(SHIM_FILENAME) {
            continue;
        }

        // Парсим --upstream и --listen из argv.
        let (upstream_raw, listen) = parse_argv(&cmdline_parts);
        let (upstream_masked, upstream_host, upstream_port) =
            split_upstream(upstream_raw.as_deref());

        // start_time() — секунды с EPOCH; сравнивать надо с текущим epoch-
        // временем. (System::uptime() — секунды с БУТА: разница уходила в
        // минус → saturating 0 → UI вечно показывал «—».)
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let uptime = now_epoch.saturating_sub(proc.start_time());

        return ShimStatus {
            running: true,
            pid: Some(pid.as_u32()),
            uptime_sec: Some(uptime),
            listen: Some(listen.unwrap_or_else(|| format!("127.0.0.1:{LISTEN_PORT}"))),
            upstream_masked,
            upstream_host,
            upstream_port,
        };
    }

    ShimStatus::down()
}

/// Одна таблица процессов на всё приложение. Раньше каждый вызов status()
/// (UI каждые 5с + watchdog каждые 5с) собирал с нуля ВСЁ о каждом процессе
/// системы — CPU, память, диск, окружение, cwd: ~6% CPU в простое на Mac.
fn process_table() -> &'static std::sync::Mutex<System> {
    static TABLE: std::sync::OnceLock<std::sync::Mutex<System>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::sync::Mutex::new(System::new()))
}

/// Обновить только список процессов + argv (argv читается один раз на PID).
fn refresh_process_table(sys: &mut System) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_cmd(UpdateKind::OnlyIfNotSet),
    );
}

/// Мост слушает свой порт? Дешёвый TCP-connect вместо скана процессов —
/// основной сигнал для watchdog'а.
fn port_alive() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], LISTEN_PORT));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

fn parse_argv(argv: &[String]) -> (Option<String>, Option<String>) {
    let mut upstream = None;
    let mut listen = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--upstream" if i + 1 < argv.len() => {
                if upstream.is_none() {
                    upstream = Some(argv[i + 1].clone());
                }
                i += 2;
            }
            // Мост ≥1.5 получает пул файлом (пароль не светится в ps).
            "--upstream-file" if i + 1 < argv.len() => {
                if upstream.is_none() {
                    upstream = std::fs::read_to_string(&argv[i + 1]).ok().and_then(|t| {
                        t.lines()
                            .map(str::trim)
                            .find(|l| !l.is_empty() && !l.starts_with('#'))
                            .map(String::from)
                    });
                }
                i += 2;
            }
            "--listen" if i + 1 < argv.len() => {
                listen = Some(argv[i + 1].clone());
                i += 2;
            }
            _ => i += 1,
        }
    }
    (upstream, listen)
}

fn split_upstream(
    url: Option<&str>,
) -> (Option<String>, Option<String>, Option<u16>) {
    let url = match url {
        Some(s) => s,
        None => return (None, None, None),
    };
    match url::Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("").to_string();
            let port = u.port();
            let user = u.username();
            let masked = if user.is_empty() {
                format!("{}://{}:{}", u.scheme(), host, port.unwrap_or(0))
            } else {
                format!(
                    "{}://{}:****@{}:{}",
                    u.scheme(),
                    user,
                    host,
                    port.unwrap_or(0)
                )
            };
            (Some(masked), Some(host), port)
        }
        Err(_) => (None, None, None),
    }
}

// ─── Stop ────────────────────────────────────────────────────────

/// Убить все процессы шима (по argv-match). Возвращает кол-во убитых.
pub fn stop() -> Result<usize, String> {
    // Ручная остановка: watchdog не должен воскрешать мост через 5с.
    MANUALLY_STOPPED.store(true, Ordering::Relaxed);
    // Сначала останавливаем задачу Планировщика (если есть): её runner-
    // супервизор иначе перезапустит убитый мост через 5 секунд.
    crate::tasks::stop_ignore_errors();

    let mut sys = process_table()
        .lock()
        .map_err(|_| "Внутренняя ошибка: таблица процессов".to_string())?;
    refresh_process_table(&mut sys);

    let mut killed = 0usize;
    for (_pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_lowercase();
        let is_bridge_exe = name == BRIDGE_EXE;
        let is_legacy_python = name.starts_with("python") || name.starts_with("py.exe");
        if !is_bridge_exe && !is_legacy_python {
            continue;
        }
        // exe-мост — по имени; python-мост — по local-proxy.py в argv.
        let kill_it = if is_bridge_exe {
            true
        } else {
            let cmdline: String = proc
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" ");
            cmdline.contains(SHIM_FILENAME)
        };
        if kill_it && proc.kill() {
            killed += 1;
        }
    }
    // Дать ОС секунду на освобождение порта.
    std::thread::sleep(Duration::from_millis(500));
    Ok(killed)
}

// ─── Start ───────────────────────────────────────────────────────

/// Запустить мост. Если задача Планировщика устарела (например, ссылается на
/// удалённый runner-vbs), пересоздать её из текущих настроек и попробовать ещё
/// раз. Легаси .vbs из Startup остаётся последним fallback для старых установок.
pub fn start() -> Result<(), String> {
    // Явный запуск снимает ручную остановку — watchdog снова сторожит.
    MANUALLY_STOPPED.store(false, Ordering::Relaxed);

    // Уже запущенный вручную мост тоже считается успехом. Настройку задачи
    // отдельно выполняет кнопка «Исправить» и фоновая миграция.
    if status().running {
        return Ok(());
    }
    // Файл моста пропал (антивирус) — вернуть из копии в программе.
    crate::bridge_file::ensure(false)?;

    let mut task_error = None;
    if crate::tasks::is_registered() {
        match crate::tasks::start() {
            Ok(()) if wait_for_bridge_process() => return Ok(()),
            Ok(()) => {
                task_error = Some(
                    "задача запущена, но процесс моста не появился (возможно, задача ссылается на удалённый файл)".to_string(),
                );
            }
            Err(e) => task_error = Some(e),
        }
        // Остановить зависший wscript/старую задачу перед заменой действия.
        crate::tasks::stop_ignore_errors();
    }

    // Самовосстановление: если старый .vbs удалён или задача устарела,
    // регистрируем актуальное действие из установленного exe и settings.
    if bridge_exe_path().is_some_and(|exe| exe.exists()) {
        if let Some(proxy_url) = crate::settings::load().proxy_url {
            match crate::tasks::register(&proxy_url) {
                Ok(()) => match crate::tasks::start() {
                    Ok(()) if wait_for_bridge_process() => return Ok(()),
                    Ok(()) => {
                        task_error = Some(
                            "новая задача запущена, но процесс моста не появился".to_string(),
                        );
                    }
                    Err(e) => task_error = Some(e),
                },
                Err(e) => task_error = Some(e),
            }
            crate::tasks::stop_ignore_errors();
        }
    }

    // Не маскировать отсутствие самого бинарника под поломанный Startup .vbs.
    // Defender может убрать exe в карантин, и повторный запуск тогда не поможет.
    if let Some(exe) = bridge_exe_path().filter(|path| !path.exists()) {
        if !shim_script_path().is_some_and(|path| path.exists()) {
            #[cfg(target_os = "windows")]
            return Err(format!(
                "Не найден {}. Проверьте журнал защиты Windows: файл мог быть помещён в карантин.",
                exe.display()
            ));
            #[cfg(not(target_os = "windows"))]
            return Err(format!(
                "Не найден {}. Пройдите мастер установки Clod Helper заново.",
                exe.display()
            ));
        }
    }

    start_legacy(task_error)
}

/// Совместимость с совсем старыми установками, у которых ещё нет exe или
/// сохранённого proxy_url для регистрации задачи: Startup .vbs.
#[cfg(target_os = "windows")]
fn start_legacy(task_error: Option<String>) -> Result<(), String> {
    let legacy_vbs = startup_vbs_path();
    if let Some(error) = task_error.as_ref() {
        if legacy_vbs.as_ref().map(|path| path.exists()) != Some(true) {
            return Err(format!("Не удалось запустить мост: {error}"));
        }
    }
    let vbs = legacy_vbs.ok_or_else(|| "Не нашёл папку Startup".to_string())?;
    if !vbs.exists() {
        return Err(format!(
            "Не найден {}. Перенастройте автозапуск в Clod Helper.",
            vbs.display()
        ));
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("wscript.exe");
    cmd.arg(&vbs);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map_err(|e| format!("Не смог запустить {}: {e}", vbs.display()))?;
    // Дать процессу время поднять listener.
    std::thread::sleep(Duration::from_millis(700));
    Ok(())
}

/// На macOS легаси-автозапуска через .vbs не бывает: задача (LaunchAgent) —
/// единственный путь запуска.
#[cfg(not(target_os = "windows"))]
fn start_legacy(task_error: Option<String>) -> Result<(), String> {
    Err(match task_error {
        Some(e) => format!("Не удалось запустить мост: {e}"),
        None => "Автозапуск моста не настроен — пройдите мастер установки".into(),
    })
}

/// Дождаться появления процесса после запуска через Планировщик. Его команда
/// может вернуть успех даже когда вложенный wscript не нашёл runner-vbs.
fn wait_for_bridge_process() -> bool {
    for _ in 0..20 {
        if status().running {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    status().running
}

pub fn restart() -> Result<(), String> {
    let _ = stop()?;
    start()
}

// ─── Watchdog (авто-подъём упавшего шима) ────────────────────────

/// Интервал опроса живости шима. 5с — баланс: быстро поднять после падения,
/// но не дёргать sysinfo слишком часто.
const WATCHDOG_INTERVAL_SEC: u64 = 5;

/// Фоновый супервайзер: раз в WATCHDOG_INTERVAL_SEC проверяет, жив ли шим, и
/// если упал — поднимает через start(). Это закрывает корневую боль: при пике
/// деплоя (ssh/scp/docker через тот же upstream-прокси) python-процесс мог
/// падать, и связь Claude Code пропадала НАМЕРТВО до ручного «Перезапустить».
/// Теперь упавший шим поднимается сам за ~5с, деплой/стрим не «гаснут».
///
/// Запускается в отдельном OS-потоке (не async) — sysinfo синхронный, а нам
/// нужен надёжный бесконечный цикл, переживающий любые ошибки start().
/// Анти-флаппинг: после успешного подъёма ждём дольше, чтобы не штормить
/// рестартами, если шим падает сразу (например, занят порт / битый upstream).
pub fn spawn_watchdog() {
    std::thread::spawn(|| {
        // дать инсталлятору/автозапуску время на первый старт, чтобы watchdog
        // не пытался поднять параллельно поднимающийся при логине шим.
        std::thread::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SEC));
        let mut consecutive_revivals: u32 = 0;
        loop {
            // Уважаем ручную остановку: «Остановить» значит остановить,
            // а не «поживи 5 секунд».
            if MANUALLY_STOPPED.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SEC));
                continue;
            }
            // Порт отвечает — мост жив, скан процессов не нужен.
            if port_alive() || status().running {
                consecutive_revivals = 0;
            } else {
                // шим мёртв — пытаемся поднять. start() идемпотентен (через .vbs).
                match start() {
                    Ok(()) => {
                        consecutive_revivals += 1;
                        tracing::warn!(
                            "watchdog: шим был мёртв — поднял (попытка #{})",
                            consecutive_revivals
                        );
                    }
                    Err(e) => {
                        tracing::error!("watchdog: не смог поднять шим: {e}");
                    }
                }
                // анти-флаппинг: если шим падает раз за разом (битый конфиг/порт),
                // тормозим, чтобы не штормить wscript-запусками. Иначе обычный темп.
                if consecutive_revivals >= 3 {
                    std::thread::sleep(Duration::from_secs(30));
                }
            }
            std::thread::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SEC));
        }
    });
}

/// Путь легаси-автозапуска (.vbs в Startup). Оставлен для миграции: задача
/// Планировщика (tasks.rs) при регистрации удаляет его.
#[cfg(target_os = "windows")]
pub(crate) fn startup_vbs_path() -> Option<PathBuf> {
    let appdata = dirs::config_dir()?; // %APPDATA% on Windows
    Some(
        appdata
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(STARTUP_VBS_NAME),
    )
}

pub(crate) fn install_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join(INSTALL_DIR_REL))
}

/// Путь к местному clod-bridge.exe (установленная копия).
pub fn bridge_exe_path() -> Option<PathBuf> {
    install_dir().map(|d| d.join(BRIDGE_EXE))
}

/// Путь к local-proxy.py в установленной копии (легаси — для миграции/детекта).
pub fn shim_script_path() -> Option<PathBuf> {
    install_dir().map(|d| d.join(SHIM_FILENAME))
}

/// Скопировать мост из ресурсов в папку установки (мост уже остановлен).
///
/// macOS: перезапись подписанного бинарника НА МЕСТЕ ломает кэш подписи ядра
/// (тот же inode) — следующий запуск убивается SIGKILL. Поэтому копируем во
/// временный файл рядом, ставим +x, снимаем карантин (унаследован от
/// скачанного .app) и атомарно подменяем rename'ом: новый inode, а при сбое
/// копирования старый мост остаётся на месте.
#[cfg(not(target_os = "windows"))]
pub fn install_bridge_binary(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let mut tmp = dst.as_os_str().to_owned();
    tmp.push(".new");
    let tmp = std::path::PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    if let Err(e) = std::fs::copy(src, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    crate::macos::set_mode(&tmp, 0o755);
    crate::macos::strip_quarantine(&tmp);
    std::fs::rename(&tmp, dst)
}

#[cfg(target_os = "windows")]
pub fn install_bridge_binary(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::copy(src, dst).map(|_| ())
}

// Примечание: write_startup_vbs (легаси-автозапуск через Startup) удалён —
// автозапуск теперь регистрирует tasks::register (Планировщик + RestartOnFailure).

// ─── Test (ping) ─────────────────────────────────────────────────

/// Проверить что шим реально проксирует трафик: запрос на echo-сервис через
/// `http://127.0.0.1:8889` → если получили IP — всё работает.
pub async fn test() -> ShimTestResult {
    let started = std::time::Instant::now();
    let proxy = match reqwest::Proxy::http(crate::claude_policy::PROXY_URL) {
        Ok(p) => p,
        Err(e) => {
            return ShimTestResult {
                ok: false,
                latency_ms: None,
                error: Some(format!("invalid proxy: {e}")),
                external_ip: None,
            };
        }
    };
    let proxy_https = match reqwest::Proxy::https(crate::claude_policy::PROXY_URL) {
        Ok(p) => p,
        Err(e) => {
            return ShimTestResult {
                ok: false,
                latency_ms: None,
                error: Some(format!("invalid proxy: {e}")),
                external_ip: None,
            };
        }
    };
    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .proxy(proxy_https)
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ShimTestResult {
                ok: false,
                latency_ms: None,
                error: Some(format!("client build: {e}")),
                external_ip: None,
            };
        }
    };
    // Через строгий вход 8890 (только upstream): в умном режиме общий вход
    // пустил бы ipify напрямую, и тест показывал бы IP пользователя, а не
    // прокси. Проверяем ровно то, что видят AI-сервисы.
    // api.ipify.org возвращает чистый IP, малый, без auth, без CORS-капризов.
    match client.get("https://api.ipify.org").send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let latency = started.elapsed().as_millis() as u64;
            if status.is_success() && !body.trim().is_empty() {
                ShimTestResult {
                    ok: true,
                    latency_ms: Some(latency),
                    error: None,
                    external_ip: Some(body.trim().to_string()),
                }
            } else {
                ShimTestResult {
                    ok: false,
                    latency_ms: Some(latency),
                    error: Some(format!("HTTP {status}")),
                    external_ip: None,
                }
            }
        }
        Err(e) => ShimTestResult {
            ok: false,
            latency_ms: None,
            error: Some(format!("{e}")),
            external_ip: None,
        },
    }
}
