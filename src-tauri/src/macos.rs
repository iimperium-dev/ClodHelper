//! Общие macOS-утилиты: launchd (LaunchAgent), PATH GUI-процесса, снятие
//! карантина, «ярлыки» в виде крошечных .app-обёрток.
//!
//! Аналоги Windows-механик:
//! - Планировщик задач → LaunchAgent в `~/Library/LaunchAgents` (KeepAlive:
//!   launchd сам перезапускает упавший мост — супервизор не нужен);
//! - `.lnk` на рабочем столе → `<Имя> (proxy).app` со sh-скриптом внутри;
//! - Mark of the Web → атрибут `com.apple.quarantine`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Домен launchd текущего GUI-пользователя: `gui/<uid>`.
pub fn gui_domain() -> String {
    static UID: OnceLock<String> = OnceLock::new();
    let uid = UID.get_or_init(|| {
        Command::new("/usr/bin/id")
            .arg("-u")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "501".into())
    });
    format!("gui/{uid}")
}

pub fn launch_agents_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library").join("LaunchAgents"))
}

/// `launchctl <args>` → (успех, stdout+stderr).
pub fn launchctl(args: &[&str]) -> (bool, String) {
    match Command::new("/bin/launchctl").args(args).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), s)
        }
        Err(e) => (false, e.to_string()),
    }
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Описание LaunchAgent'а.
pub struct Agent<'a> {
    pub label: &'a str,
    pub program_arguments: &'a [String],
    /// Перезапускать при завершении (мост). false — одноразовый запуск при логине.
    pub keep_alive: bool,
    pub stderr_log: Option<&'a Path>,
}

/// Записать plist агента. Права 0600: в аргументах моста лежат логин/пароль
/// upstream-прокси (как и в задаче Планировщика на Windows).
pub fn write_agent_plist(path: &Path, agent: &Agent) -> Result<(), String> {
    let args: String = agent
        .program_arguments
        .iter()
        .map(|a| format!("\t\t<string>{}</string>\n", xml_escape(a)))
        .collect();
    let keep_alive = if agent.keep_alive {
        // ThrottleInterval 5 — тот же темп перезапуска, что у супервизора моста.
        "\t<key>KeepAlive</key>\n\t<true/>\n\t<key>ThrottleInterval</key>\n\t<integer>5</integer>\n"
    } else {
        ""
    };
    let log = agent
        .stderr_log
        .map(|p| {
            format!(
                "\t<key>StandardErrorPath</key>\n\t<string>{}</string>\n",
                xml_escape(&p.to_string_lossy())
            )
        })
        .unwrap_or_default();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         {keep_alive}{log}</dict>\n</plist>\n",
        label = xml_escape(agent.label),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("plist.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    set_mode(&tmp, 0o600);
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
}

/// Загружен ли агент в launchd прямо сейчас.
pub fn agent_loaded(label: &str) -> bool {
    launchctl(&["print", &format!("{}/{label}", gui_domain())]).0
}

/// Состояние агента для UI: "Running" | "Ready" (загружен, процесс не живёт).
pub fn agent_state(label: &str) -> Option<String> {
    let (ok, out) = launchctl(&["print", &format!("{}/{label}", gui_domain())]);
    if !ok {
        return None;
    }
    let running = out
        .lines()
        .any(|l| l.trim().strip_prefix("state = ").is_some_and(|s| s.trim() == "running"));
    Some(if running { "Running" } else { "Ready" }.into())
}

/// Загрузить агент (с RunAtLoad он сразу стартует). Уже загруженный — не ошибка.
pub fn bootstrap(label: &str, plist: &Path) -> Result<(), String> {
    if agent_loaded(label) {
        return Ok(());
    }
    let (ok, out) = launchctl(&["bootstrap", &gui_domain(), &plist.to_string_lossy()]);
    if ok || agent_loaded(label) {
        Ok(())
    } else {
        Err(format!("launchctl bootstrap: {}", out.trim()))
    }
}

/// Выгрузить агент: launchd завершает процесс и больше его не поднимает
/// (до следующего входа в систему или bootstrap). Ошибки глотаем.
pub fn bootout(label: &str) {
    if agent_loaded(label) {
        let _ = launchctl(&["bootout", &format!("{}/{label}", gui_domain())]);
    }
}

pub fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

/// Снять карантин с файла/бандла. fs::copy на macOS копирует xattr, и мост,
/// скопированный из скачанного .app, иначе блокировал бы Gatekeeper.
pub fn strip_quarantine(path: &Path) {
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .output();
}

/// GUI-приложение, запущенное из Finder/Dock, получает урезанный PATH
/// (`/usr/bin:/bin:/usr/sbin:/sbin`) — node/npm/claude из Homebrew или nvm не
/// находятся. Берём PATH из login-шелла пользователя и добавляем типовые папки.
pub fn fix_path_env() {
    let mut parts: Vec<String> = Vec::new();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if let Ok(out) = Command::new(&shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
    {
        let p = String::from_utf8_lossy(&out.stdout).to_string();
        parts.extend(p.split(':').filter(|s| !s.is_empty()).map(String::from));
    }
    if let Some(home) = dirs::home_dir() {
        for rel in [".local/bin", ".npm-global/bin", ".claude/local", ".volta/bin", ".bun/bin"] {
            parts.push(home.join(rel).to_string_lossy().to_string());
        }
    }
    for p in ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"] {
        parts.push(p.into());
    }
    if let Ok(cur) = std::env::var("PATH") {
        parts.extend(cur.split(':').filter(|s| !s.is_empty()).map(String::from));
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| seen.insert(p.clone()));
    std::env::set_var("PATH", parts.join(":"));
}

/// Экранировать строку для sh в одинарных кавычках.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Найти .app по имени в /Applications и ~/Applications.
pub fn find_app(names: &[&str]) -> Option<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    for root in &roots {
        for name in names {
            let p = root.join(name);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Значение ключа из Info.plist бандла (через PlistBuddy).
fn bundle_info(app: &Path, key: &str) -> Option<String> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Создать «ярлык» — `.app`-обёртку с sh-скриптом `script_body` внутри.
/// Иконку берём у целевого приложения (`icon_from`), если оно есть.
/// Обёртка создаётся локально, поэтому карантина у неё нет и Gatekeeper её
/// не трогает; подпись sh-скрипту не нужна.
pub fn create_app_wrapper(
    dest: &Path,
    display_name: &str,
    script_body: &str,
    icon_from: Option<&Path>,
) -> Result<(), String> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .map_err(|e| format!("Не удалось заменить {}: {e}", dest.display()))?;
    }
    let contents = dest.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(|e| format!("create {}: {e}", macos.display()))?;
    std::fs::create_dir_all(&resources)
        .map_err(|e| format!("create {}: {e}", resources.display()))?;

    let launcher = macos.join("launcher");
    std::fs::write(&launcher, format!("#!/bin/sh\n{script_body}\n"))
        .map_err(|e| format!("write {}: {e}", launcher.display()))?;
    set_mode(&launcher, 0o755);

    let mut icon_key = String::new();
    if let Some(app) = icon_from {
        if let Some(icon) = bundle_info(app, "CFBundleIconFile") {
            let file = if icon.ends_with(".icns") { icon } else { format!("{icon}.icns") };
            let src = app.join("Contents/Resources").join(&file);
            if std::fs::copy(&src, resources.join("AppIcon.icns")).is_ok() {
                icon_key = "\t<key>CFBundleIconFile</key>\n\t<string>AppIcon</string>\n".into();
            }
        }
    }
    let slug: String = display_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let info = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>CFBundleExecutable</key>\n\t<string>launcher</string>\n\
         \t<key>CFBundleIdentifier</key>\n\t<string>ru.iimperium.clodhelper.shortcut.{slug}</string>\n\
         \t<key>CFBundleName</key>\n\t<string>{name}</string>\n\
         \t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
         \t<key>CFBundleVersion</key>\n\t<string>1</string>\n\
         \t<key>LSUIElement</key>\n\t<true/>\n\
         {icon_key}</dict>\n</plist>\n",
        name = xml_escape(display_name),
    );
    std::fs::write(contents.join("Info.plist"), info)
        .map_err(|e| format!("write Info.plist: {e}"))?;
    // Finder кэширует иконки бандлов — «трогаем» бандл, чтобы перечитал.
    let _ = Command::new("/usr/bin/touch").arg(dest).output();
    Ok(())
}

/// Показать файл в Finder.
pub fn reveal_in_finder(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}
