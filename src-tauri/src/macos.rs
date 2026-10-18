//! Общие macOS-утилиты: launchd (LaunchAgent), PATH GUI-процесса, снятие
//! карантина, «ярлыки» в виде крошечных .app-обёрток.
//!
//! Аналоги Windows-механик:
//! - Планировщик задач → LaunchAgent в `~/Library/LaunchAgents` (KeepAlive:
//!   launchd сам перезапускает упавший мост — супервизор не нужен);
//! - `.lnk` на рабочем столе → `<Имя> (proxy).app` со sh-скриптом внутри;
//! - Mark of the Web → атрибут `com.apple.quarantine`.

// На Windows модуль компилируется только в тестах — остальное там не нужно.
#![cfg_attr(target_os = "windows", allow(dead_code))]

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

/// Bundle id приложения (tauri.conf.json → identifier).
pub const BUNDLE_ID: &str = "ru.iimperium.clodhelper";

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
    // AssociatedBundleIdentifiers: «Объекты входа» (macOS 13+) показывают
    // агент как часть Clod Helper, а не безымянный процесс. Мосту ещё и лимит
    // дескрипторов: дефолтные 256 у launchd — это ~120 туннелей.
    let extra = if agent.keep_alive {
        "\t<key>SoftResourceLimits</key>\n\t<dict>\n\t\t<key>NumberOfFiles</key>\n\t\t<integer>4096</integer>\n\t</dict>\n"
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
         \t<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t<string>{BUNDLE_ID}</string>\n\t</array>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         {keep_alive}{extra}{log}</dict>\n</plist>\n",
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
///
/// Ретраи: сразу после bootout launchd ещё досносит службу, и bootstrap
/// отвечает «5: Input/output error» — ровно сценарий «Перезапустить».
pub fn bootstrap(label: &str, plist: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..10 {
        if agent_loaded(label) {
            return Ok(());
        }
        let (ok, out) = launchctl(&["bootstrap", &gui_domain(), &plist.to_string_lossy()]);
        if ok || agent_loaded(label) {
            return Ok(());
        }
        last = out;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(format!("launchctl bootstrap: {}", last.trim()))
}

/// Выгрузить агент: launchd завершает процесс и больше его не поднимает
/// (до следующего входа в систему или bootstrap). Ждём фактической выгрузки
/// (до ~3 с): bootout асинхронен. Ошибки глотаем.
pub fn bootout(label: &str) {
    if !agent_loaded(label) {
        return;
    }
    let _ = launchctl(&["bootout", &format!("{}/{label}", gui_domain())]);
    for _ in 0..30 {
        if !agent_loaded(label) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
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
    if let Some(p) = login_shell_path() {
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

/// PATH из интерактивного login-шелла (`-ilc`: nvm/volta обычно живут в
/// ~/.zshrc, не в ~/.zprofile). Жёсткий таймаут 3 с — тяжёлый или зависший
/// конфиг шелла не должен блокировать старт приложения. Маркеры отделяют
/// PATH от того, что шелл печатает при старте (приветствия, oh-my-zsh).
fn login_shell_path() -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;
    const BEGIN: &str = "__CLOD_PATH_BEGIN__";
    const END: &str = "__CLOD_PATH_END__";
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = Command::new(&shell)
        .args(["-ilc", &format!("printf '{BEGIN}%s{END}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    extract_between(&out, BEGIN, END).map(String::from)
}

fn extract_between<'a>(text: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let start = text.rfind(begin)? + begin.len();
    let len = text[start..].find(end)?;
    Some(&text[start..start + len]).filter(|s| !s.is_empty())
}

/// Заменить управляемый блок `begin…end` в тексте конфига (например
/// ~/.zshenv) на `block`, либо убрать его при None. Чужие строки не трогает;
/// незакрытый маркер (пользователь стёр конец) не съедает остаток файла.
/// Идемпотентна: повторный вызов с тем же блоком текст не меняет.
pub fn replace_managed_block(text: &str, begin: &str, end: &str, block: Option<&str>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == begin {
            if let Some(off) = lines[i + 1..].iter().position(|l| l.trim() == end) {
                i += off + 2;
                continue;
            }
        }
        kept.push(lines[i]);
        i += 1;
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = String::new();
    for line in &kept {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(block) = block {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(begin);
        out.push('\n');
        out.push_str(block);
        if !block.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(end);
        out.push('\n');
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "# >>> clod >>>";
    const E: &str = "# <<< clod <<<";

    #[test]
    fn managed_block_is_idempotent_and_keeps_user_lines() {
        let user = "export EDITOR=vim\n# мой конфиг\n";
        let once = replace_managed_block(user, B, E, Some("export A=1\n"));
        assert_eq!(once, format!("{user}\n{B}\nexport A=1\n{E}\n"));
        assert_eq!(replace_managed_block(&once, B, E, Some("export A=1\n")), once);
        // Замена значения — один блок, не два.
        let changed = replace_managed_block(&once, B, E, Some("export A=2"));
        assert_eq!(changed.matches(B).count(), 1);
        assert!(changed.contains("export A=2\n") && !changed.contains("A=1"));
        // Удаление возвращает исходный файл.
        assert_eq!(replace_managed_block(&changed, B, E, None), user);
        // Файл только с нашим блоком → пусто (вызывающий удалит файл).
        let only = replace_managed_block("", B, E, Some("x"));
        assert_eq!(replace_managed_block(&only, B, E, None), "");
    }

    #[test]
    fn managed_block_without_end_marker_keeps_rest_of_file() {
        let broken = format!("a\n{B}\nexport A=1\nexport USER_LINE=1\n");
        let out = replace_managed_block(&broken, B, E, None);
        assert!(out.contains("USER_LINE"), "незакрытый маркер съел строки пользователя");
    }

    #[test]
    fn sh_quote_survives_apostrophes_and_spaces() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("/Users/O'Brien/My App"), r"'/Users/O'\''Brien/My App'");
    }

    #[test]
    fn xml_escape_covers_plist_specials() {
        assert_eq!(
            xml_escape("http://u:p&<>\"@h:1"),
            "http://u:p&amp;&lt;&gt;&quot;@h:1"
        );
    }

    #[test]
    fn login_shell_path_markers_ignore_shell_banners() {
        let out = "Welcome!\n__B__/opt/homebrew/bin:/usr/bin__E__";
        assert_eq!(extract_between(out, "__B__", "__E__"), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(extract_between("no markers", "__B__", "__E__"), None);
        assert_eq!(extract_between("__B____E__", "__B__", "__E__"), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clod-macos-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn agent_plist_escapes_credentials_and_sets_keepalive() {
        let dir = temp_dir("plist");
        let path = dir.join("a.plist");
        let args = vec!["/x/clod-bridge".to_string(), "http://u:p&q@h:1".to_string()];
        write_agent_plist(
            &path,
            &Agent { label: "ru.test", program_arguments: &args, keep_alive: true, stderr_log: None },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("<string>http://u:p&amp;q@h:1</string>"));
        assert!(text.contains("<key>KeepAlive</key>\n\t<true/>"));
        assert!(text.contains("<key>RunAtLoad</key>"));
        assert!(text.contains(&format!("<string>{BUNDLE_ID}</string>")));
        assert!(text.contains("<key>NumberOfFiles</key>\n\t\t<integer>4096</integer>"));
        assert!(!path.with_extension("plist.tmp").exists());
        // Валидный plist: plutil — тот же парсер, что у launchd.
        #[cfg(target_os = "macos")]
        assert!(std::process::Command::new("/usr/bin/plutil")
            .args(["-lint", "-s"])
            .arg(&path)
            .status()
            .unwrap()
            .success());

        write_agent_plist(
            &path,
            &Agent { label: "ru.test", program_arguments: &args, keep_alive: false, stderr_log: None },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("KeepAlive"));
        assert!(!text.contains("SoftResourceLimits"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn app_wrapper_has_bundle_layout() {
        let dir = temp_dir("wrapper");
        let dest = dir.join("VS Code (proxy).app");
        create_app_wrapper(&dest, "VS Code (proxy)", "exec /usr/bin/open -a 'X'", None).unwrap();
        let launcher = std::fs::read_to_string(dest.join("Contents/MacOS/launcher")).unwrap();
        assert!(launcher.starts_with("#!/bin/sh\n"));
        let info = std::fs::read_to_string(dest.join("Contents/Info.plist")).unwrap();
        assert!(info.contains("<string>launcher</string>"));
        assert!(info.contains("ru.iimperium.clodhelper.shortcut.vs-code--proxy-"));
        // Повторное создание заменяет бандл, а не падает.
        create_app_wrapper(&dest, "VS Code (proxy)", "exec true", None).unwrap();
        assert!(std::fs::read_to_string(dest.join("Contents/MacOS/launcher")).unwrap().contains("exec true"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
