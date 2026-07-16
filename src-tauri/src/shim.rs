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
use std::time::Duration;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

pub const LISTEN_PORT: u16 = 8889;
/// Легаси python-мост (для миграции/детекта старых установок).
const SHIM_FILENAME: &str = "local-proxy.py";
/// Новый мост — самостоятельный exe (НЕ python.exe). Критично: killswitch
/// блокирует python.exe, а мост на python.exe душил сам себя. exe-мост под
/// своим именем не попадает под блок → killswitch не рубит собственный мост.
pub const BRIDGE_EXE: &str = "clod-bridge.exe";
const INSTALL_DIR_REL: &str = "ClaudeDesktopProxy";
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
    let mut sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    sys.refresh_processes(ProcessesToUpdate::All, true);

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

        let uptime = System::uptime().saturating_sub(proc.start_time());

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

fn parse_argv(argv: &[String]) -> (Option<String>, Option<String>) {
    let mut upstream = None;
    let mut listen = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--upstream" if i + 1 < argv.len() => {
                upstream = Some(argv[i + 1].clone());
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
    let mut sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    sys.refresh_processes(ProcessesToUpdate::All, true);

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

/// Запустить шим через тот же .vbs что прописан в Startup-папке (молча,
/// без окна консоли). Если .vbs нет — фейл (значит инсталлятор не отработал).
pub fn start() -> Result<(), String> {
    let vbs = startup_vbs_path()
        .ok_or_else(|| "Не нашёл папку Startup".to_string())?;
    if !vbs.exists() {
        return Err(format!(
            "Не найден {}. Запустите установщик заново.",
            vbs.display()
        ));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = std::process::Command::new("wscript.exe");
        cmd.arg(&vbs);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn()
            .map_err(|e| format!("Не смог запустить {}: {e}", vbs.display()))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = vbs;
    }
    // Дать процессу время поднять listener.
    std::thread::sleep(Duration::from_millis(700));
    Ok(())
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
            if status().running {
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

fn startup_vbs_path() -> Option<PathBuf> {
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

fn install_dir() -> Option<PathBuf> {
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

/// Перезаписать `ClaudeDesktopProxy.vbs` с новым upstream-прокси. Не запускает
/// сам мост — вызывающий код должен сделать `stop()` + `start()` после.
///
/// Теперь запускает `clod-bridge.exe` НАПРЯМУЮ (не python) — так мост не
/// попадает под killswitch-блок python.exe. `_python_cmd` сохранён в сигнатуре
/// для обратной совместимости вызовов, но игнорируется.
pub fn write_startup_vbs(
    proxy_url: &str,
    _python_cmd: Option<&str>,
) -> Result<PathBuf, String> {
    let vbs_path = startup_vbs_path()
        .ok_or_else(|| "Не нашёл папку Startup".to_string())?;
    let exe_path = bridge_exe_path()
        .ok_or_else(|| "Не нашёл папку ClaudeDesktopProxy".to_string())?;
    if !exe_path.exists() {
        return Err(format!(
            "Не найден {}. Запустите установщик заново.",
            exe_path.display()
        ));
    }
    let exe_str = exe_path.to_string_lossy().to_string();
    // VBS: запускаем exe напрямую, скрыто (0), не ждём (False).
    let vbs = format!(
        "' Auto-start Clod bridge at Windows login (hidden).\r\n\
         Set sh = CreateObject(\"WScript.Shell\")\r\n\
         sh.Run Chr(34) & \"{path}\" & Chr(34) & \" --upstream \" & Chr(34) & \"{proxy}\" & Chr(34) & \" --listen 127.0.0.1:{port} --quiet\", 0, False\r\n",
        path = exe_str,
        proxy = proxy_url.replace('"', ""),
        port = LISTEN_PORT,
    );
    if let Some(parent) = vbs_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    std::fs::write(&vbs_path, vbs)
        .map_err(|e| format!("write {}: {e}", vbs_path.display()))?;
    Ok(vbs_path)
}

// ─── Test (ping) ─────────────────────────────────────────────────

/// Проверить что шим реально проксирует трафик: запрос на echo-сервис через
/// `http://127.0.0.1:8889` → если получили IP — всё работает.
pub async fn test() -> ShimTestResult {
    let started = std::time::Instant::now();
    let proxy = match reqwest::Proxy::http(format!("http://127.0.0.1:{LISTEN_PORT}")) {
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
    let proxy_https = match reqwest::Proxy::https(format!(
        "http://127.0.0.1:{LISTEN_PORT}"
    )) {
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
