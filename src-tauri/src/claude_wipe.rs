//! «Инструменты» → очистка данных аккаунта Claude Desktop.
//!
//! Задача: сменить аккаунт так, чтобы новый ничем не пересекался с прошлым
//! на этом компьютере. Удаляем всё, что несёт отпечаток аккаунта или
//! устройства: вход (куки, токены), постоянные идентификаторы устройства
//! (ant-did, соли, Sentry/Crashpad), хранилище сайта claude.ai, кэш, журналы.
//!
//! Не трогаем:
//!   - чаты и проекты — они на серверах Anthropic, в самом аккаунте;
//!   - локальные сессии вкладки Code и задачи Cowork (claude-code-sessions,
//!     local-agent-mode-sessions, ~/.claude/projects) — они разложены по
//!     папкам аккаунта и вернутся, если снова войти в прошлый;
//!   - настройки MCP и прочие свои настройки (claude_desktop_config.json —
//!     из него убираются только записи, привязанные к аккаунту);
//!   - саму программу и Claude Code внутри неё.
//!
//! Где лежат данные (Electron userData):
//!   - Windows, Store (новые версии): %LOCALAPPDATA%\Claude-Data\<пакет>
//!   - Windows, Store (старые): %LOCALAPPDATA%\Packages\<пакет>\LocalCache\Roaming\Claude
//!   - Windows, установщик: %APPDATA%\Claude
//!   - macOS: ~/Library/Application Support/Claude
//! Claude Code (им пользуется и вкладка Code): ~/.claude.json и ~/.claude
//! (или CLAUDE_CONFIG_DIR).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Вход: куки, токены, сохранённый аккаунт.
    Login,
    /// Постоянные идентификаторы устройства и установки.
    Device,
    /// Хранилище сайта claude.ai внутри программы.
    Web,
    /// Данные, привязанные к аккаунту: лимиты, плагины, удалённый доступ.
    Account,
    /// Журналы, телеметрия, отчёты об ошибках.
    Logs,
    /// Кэш страниц и видеокарты.
    Cache,
    /// Локальные чаты и проекты (вкладка Code, Cowork). Удаляются, только
    /// если это выбрано отдельно: по умолчанию остаются на компьютере.
    Chats,
}

/// Локальные чаты Claude Desktop (в его папке данных).
const DESKTOP_CHATS: &[&str] = &[
    "claude-code-sessions",
    "local-agent-mode-sessions",
    "scratch-workspaces",
    "side-session-notes",
    "git-worktrees.json",
];
/// Переписка и связанное у Claude Code (~/.claude).
const CODE_CHATS: &[&str] =
    &["projects", "file-history", "todos", "sessions", "session-env", "shell-snapshots", "plans", "uploads"];

#[derive(Debug, Clone, Serialize)]
pub struct GroupInfo {
    pub id: Group,
    /// Claude Code (а не Claude Desktop).
    pub code: bool,
    pub bytes: u64,
    pub items: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    /// Найденные папки данных Claude Desktop.
    pub desktop_dirs: Vec<String>,
    /// Найден ли Claude Code (~/.claude.json или ~/.claude).
    pub code_found: bool,
    pub groups: Vec<GroupInfo>,
    /// Claude Desktop сейчас запущен — перед очисткой его закроем.
    pub desktop_running: bool,
    /// Claude Code, запущенный отдельно (терминал, VS Code): он может
    /// записать свои данные обратно — лучше закрыть.
    pub cli_running: u32,
    /// Локальные сессии вкладки Code и Cowork, которые остаются.
    pub kept_sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeResult {
    pub removed_bytes: u64,
    pub removed_items: u32,
    /// Что удалить не удалось (путь — причина).
    pub failed: Vec<String>,
    /// Claude Desktop был запущен и закрыт для очистки.
    pub closed_desktop: bool,
}

#[derive(Debug)]
enum Action {
    /// Удалить файл или папку целиком.
    Remove(PathBuf),
    /// config.json Claude Desktop: оставить только нейтральные ключи.
    DesktopConfig(PathBuf),
    /// claude_desktop_config.json: убрать записи с id аккаунта.
    DesktopPrefs(PathBuf),
    /// ~/.claude.json: убрать аккаунт, идентификаторы и кэш сервера.
    CodeConfig(PathBuf),
    /// macOS: токен Claude Code в Связке ключей.
    #[allow(dead_code)]
    Keychain(&'static str),
}

#[derive(Debug)]
struct Target {
    group: Group,
    code: bool,
    action: Action,
}

/// Где искать: всё явными путями, чтобы проверять на временной папке.
#[derive(Debug, Default, Clone)]
pub struct Env {
    pub desktop_dirs: Vec<PathBuf>,
    /// Папки журналов Claude Desktop вне userData.
    pub desktop_extra: Vec<(Group, PathBuf)>,
    /// ~/.claude (или CLAUDE_CONFIG_DIR).
    pub code_dir: Option<PathBuf>,
    /// ~/.claude.json (или CLAUDE_CONFIG_DIR/.claude.json).
    pub code_json: Option<PathBuf>,
    /// Где лежат .claude.json.backup* (домашняя папка).
    pub code_json_dir: Option<PathBuf>,
    /// Кэш/журналы Claude Code вне ~/.claude.
    pub code_extra: Vec<PathBuf>,
    pub keychain: bool,
}

// ─── Что удалять ────────────────────────────────────────────────────

/// Папки/файлы userData Claude Desktop по группам.
const DESKTOP_REMOVE: &[(Group, &str)] = &[
    // Вход: куки (в т.ч. sessionKey), HSTS и прочее состояние сети.
    (Group::Login, "Network"),
    (Group::Login, "Cookies"),
    (Group::Login, "Cookies-journal"),
    (Group::Login, "ssh-remote-server-state.json"),
    // Секреты сессий Code и доступы, выданные под аккаунт.
    (Group::Login, "ccd-session-secrets"),
    // Куки встроенного браузера и других отдельных сессий программы.
    (Group::Login, "Partitions"),
    // Идентификаторы устройства и установки.
    (Group::Device, "ant-did"),
    (Group::Device, "ant-device-registry.json"),
    (Group::Device, "ccd-ids.json"),
    // Поколение аппаратного ключа устройства (сам ключ в TPM у каждого
    // аккаунта свой: device-registry-<id аккаунта>).
    (Group::Device, "ant-device-key-generation.json"),
    // Соль id камер и микрофонов, которые видит страница (постоянный отпечаток).
    // «Local State» не трогаем: там ключ шифрования секретов расширений,
    // наружу он не уходит, а без него сломаются их настройки.
    (Group::Device, "Preferences"),
    (Group::Device, "DIPS"),
    (Group::Device, "DIPS-wal"),
    (Group::Device, "Trust Tokens"),
    (Group::Device, "Trust Tokens-journal"),
    (Group::Device, "TransportSecurity"),
    // Хранилище сайта claude.ai.
    (Group::Web, "Local Storage"),
    (Group::Web, "Session Storage"),
    (Group::Web, "IndexedDB"),
    (Group::Web, "WebStorage"),
    (Group::Web, "Shared Dictionary"),
    (Group::Web, "SharedStorage"),
    (Group::Web, "SharedStorage-wal"),
    (Group::Web, "Service Worker"),
    (Group::Web, "databases"),
    (Group::Web, "blob_storage"),
    // Привязано к аккаунту.
    (Group::Account, "plan-usage-history.json"),
    (Group::Account, "buddy-tokens.json"),
    (Group::Account, "fcache"),
    (Group::Account, "rpm"),
    (Group::Account, "spaces-present"),
    (Group::Account, "remote-control-state.json"),
    (Group::Account, "cowork-enabled-cli-ops.json"),
    (Group::Account, "bridge-state.json"),
    (Group::Account, "pending-uploads"),
    // Журналы и отчёты об ошибках (в них id пользователя и устройства).
    (Group::Logs, "logs"),
    (Group::Logs, "sentry"),
    (Group::Logs, "Crashpad"),
    (Group::Logs, "dispatch-footprint-census.json"),
    (Group::Logs, "builtin-mcp-incidents.json"),
    (Group::Logs, "edr-integration-runs.json"),
    // Кэш.
    (Group::Cache, "Cache"),
    (Group::Cache, "Code Cache"),
    (Group::Cache, "GPUCache"),
    (Group::Cache, "DawnGraphiteCache"),
    (Group::Cache, "DawnWebGPUCache"),
    (Group::Cache, "DawnCache"),
    (Group::Cache, "GrShaderCache"),
    (Group::Cache, "ShaderCache"),
    (Group::Cache, "GPUPersistentCache"),
    (Group::Cache, "declarative_performance_observer.db"),
    (Group::Cache, "declarative_performance_observer.db-journal"),
];

/// config.json Claude Desktop: всё остальное — токены, id аккаунта,
/// доверенное устройство, дата первого запуска, кэш под аккаунт.
const DESKTOP_CONFIG_KEEP: &[&str] = &[
    "locale",
    "bootFrameLayout",
    "terminalCliPowerShellPath",
    "updaterLastSeenVersion",
    "hostGitConfinedInstall",
    "remote_marketplace_migration_done_v1",
];

/// ~/.claude.json: ключи аккаунта и идентификаторы установки.
const CODE_JSON_DROP: &[&str] = &[
    "oauthaccount",
    "userid",
    "machineid",
    "anonymousid",
    "firststarttime",
    "claudecodefirsttokendate",
    "primaryapikey",
    "seennotifications",
    "feedbacksurveystate",
    "claudeaimcpeverconnected",
    "remotecontrolsurfacesseen",
    "replbridgeplaceholders",
    "passesupsellseencount",
    "hasvisitedpasses",
    "additionalmodeloptionsansweredat",
    "subscriptionnoticecount",
    "hasavailablesubscription",
    "recommendedsubscription",
];

/// …и всё, что сервер прислал под этот аккаунт (флаги, лимиты, модели).
fn code_key_is_account(key: &str) -> bool {
    let k = key.to_lowercase();
    CODE_JSON_DROP.contains(&k.as_str())
        || ["cache", "eligib", "subscription", "overage", "extrausage", "oauth", "account"]
            .iter()
            .any(|w| k.contains(w))
}

fn targets(env: &Env) -> Vec<Target> {
    let mut v = Vec::new();
    for d in &env.desktop_dirs {
        for (group, rel) in DESKTOP_REMOVE {
            v.push(Target { group: *group, code: false, action: Action::Remove(d.join(rel)) });
        }
        for entry in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("host-creds-") {
                v.push(Target { group: Group::Login, code: false, action: Action::Remove(entry.path()) });
            }
        }
        v.push(Target { group: Group::Login, code: false, action: Action::DesktopConfig(d.join("config.json")) });
        v.push(Target {
            group: Group::Account,
            code: false,
            action: Action::DesktopPrefs(d.join("claude_desktop_config.json")),
        });
    }
    for (group, p) in &env.desktop_extra {
        v.push(Target { group: *group, code: false, action: Action::Remove(p.clone()) });
    }
    if let Some(json) = &env.code_json {
        v.push(Target { group: Group::Login, code: true, action: Action::CodeConfig(json.clone()) });
    }
    // Копии ~/.claude.json — с тем же аккаунтом и id.
    let backups = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with(".claude.json.backup") || n.starts_with(".claude.json.tmp")
            })
            .map(|e| e.path())
            .collect()
    };
    if let Some(home) = &env.code_json_dir {
        for p in backups(home) {
            v.push(Target { group: Group::Device, code: true, action: Action::Remove(p) });
        }
    }
    if let Some(dir) = &env.code_dir {
        v.push(Target { group: Group::Login, code: true, action: Action::Remove(dir.join(".credentials.json")) });
        v.push(Target { group: Group::Device, code: true, action: Action::Remove(dir.join("statsig")) });
        for p in backups(&dir.join("backups")) {
            v.push(Target { group: Group::Device, code: true, action: Action::Remove(p) });
        }
        v.push(Target { group: Group::Logs, code: true, action: Action::Remove(dir.join("telemetry")) });
        v.push(Target { group: Group::Logs, code: true, action: Action::Remove(dir.join("debug")) });
    }
    for p in &env.code_extra {
        v.push(Target { group: Group::Logs, code: true, action: Action::Remove(p.clone()) });
    }
    if env.keychain {
        v.push(Target { group: Group::Login, code: true, action: Action::Keychain("Claude Code-credentials") });
    }
    // Локальные чаты — отдельная цель (группа Chats), по умолчанию не трогаем.
    for d in &env.desktop_dirs {
        for rel in DESKTOP_CHATS {
            v.push(Target { group: Group::Chats, code: false, action: Action::Remove(d.join(rel)) });
        }
    }
    if let Some(dir) = &env.code_dir {
        for rel in CODE_CHATS {
            v.push(Target { group: Group::Chats, code: false, action: Action::Remove(dir.join(rel)) });
        }
    }
    v
}

// ─── Где искать ─────────────────────────────────────────────────────

/// Похожа ли папка на userData Claude Desktop (а не на папку программы).
fn looks_like_user_data(d: &Path) -> bool {
    ["Local State", "config.json", "Network", "Local Storage", "claude_desktop_config.json"]
        .iter()
        .any(|n| d.join(n).exists())
}

#[cfg(target_os = "windows")]
fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    let key = |p: &Path| p.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/']).to_string();
    if !v.iter().any(|x| key(x) == key(&p)) {
        v.push(p);
    }
}

pub fn env() -> Env {
    let mut e = Env::default();
    let home = dirs::home_dir();

    #[cfg(target_os = "windows")]
    {
        let local = dirs::data_local_dir();
        let roaming = dirs::data_dir();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(l) = &local {
            // Store, новые версии: данные вынесены из пакета.
            for entry in std::fs::read_dir(l.join("Claude-Data")).into_iter().flatten().flatten() {
                candidates.push(entry.path());
            }
            // Store, старые версии: виртуализированный %APPDATA% пакета.
            for entry in std::fs::read_dir(l.join("Packages")).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("anthropicpbc.claude_") || name.starts_with("claude_") {
                    let pkg = entry.path();
                    candidates.push(pkg.join("LocalCache").join("Roaming").join("Claude"));
                    e.desktop_extra.push((Group::Logs, pkg.join("LocalCache").join("Local").join("Claude").join("logs")));
                }
            }
            // Журналы — в %LOCALAPPDATA%\Claude\logs. Саму папку не трогаем:
            // в ней может стоять программа (старый установщик).
            e.desktop_extra.push((Group::Logs, l.join("Claude").join("logs")));
            e.code_extra.push(l.join("claude-cli-nodejs"));
        }
        if let Some(r) = &roaming {
            candidates.push(r.join("Claude"));
        }
        for c in candidates {
            if c.is_dir() && looks_like_user_data(&c) {
                push_unique(&mut e.desktop_dirs, c);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(h) = &home {
            let lib = h.join("Library");
            let c = lib.join("Application Support").join("Claude");
            if c.is_dir() && looks_like_user_data(&c) {
                e.desktop_dirs.push(c);
            }
            const ID: &str = "com.anthropic.claudefordesktop";
            e.desktop_extra.push((Group::Logs, lib.join("Logs").join("Claude")));
            e.desktop_extra.push((Group::Cache, lib.join("Caches").join(ID)));
            e.desktop_extra.push((Group::Login, lib.join("HTTPStorages").join(ID)));
            e.desktop_extra.push((Group::Login, lib.join("HTTPStorages").join(format!("{ID}.binarycookies"))));
            e.desktop_extra.push((Group::Cache, lib.join("Saved Application State").join(format!("{ID}.savedState"))));
            e.code_extra.push(lib.join("Caches").join("claude-cli-nodejs"));
            e.keychain = true;
        }
    }

    // Claude Code: CLAUDE_CONFIG_DIR переносит и ~/.claude, и .claude.json.
    match std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(dir) => {
            e.code_json = Some(dir.join(".claude.json"));
            e.code_json_dir = Some(dir.clone());
            e.code_dir = Some(dir);
        }
        None => {
            if let Some(h) = &home {
                e.code_json = Some(h.join(".claude.json"));
                e.code_json_dir = Some(h.clone());
                e.code_dir = Some(h.join(".claude"));
            }
        }
    }
    e
}

// ─── Размеры ────────────────────────────────────────────────────────

/// Размер файла/папки без перехода по ссылкам (ссылка — 0).
fn size_of(p: &Path) -> (u64, u32) {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return (0, 0) };
    if meta.file_type().is_symlink() {
        return (0, 1);
    }
    if meta.is_file() {
        return (meta.len(), 1);
    }
    let mut bytes = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else { continue };
            if m.is_dir() && !m.file_type().is_symlink() {
                stack.push(entry.path());
            } else {
                bytes += m.len();
            }
        }
    }
    (bytes, 1)
}

fn read_json(p: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(p).ok()?;
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() }
        })
}

/// id аккаунтов и организаций, которые видны на этом компьютере: папки
/// сессий разложены как <аккаунт>/<организация>, плюс сохранённые значения.
fn account_ids(env: &Env) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut add = |s: &str| {
        if is_uuid(s) {
            ids.insert(s.to_lowercase());
        }
    };
    for d in &env.desktop_dirs {
        for sub in ["claude-code-sessions", "local-agent-mode-sessions", "scratch-workspaces"] {
            for acc in std::fs::read_dir(d.join(sub)).into_iter().flatten().flatten() {
                add(&acc.file_name().to_string_lossy());
                for org in std::fs::read_dir(acc.path()).into_iter().flatten().flatten() {
                    add(&org.file_name().to_string_lossy());
                }
            }
        }
        if let Some(cfg) = read_json(&d.join("config.json")) {
            if let Some(Value::String(s)) = cfg.get("lastKnownAccountUuid") {
                add(s);
            }
            if let Some(Value::String(s)) = cfg.get("bootSideSessions").and_then(|b| b.get("accountUuid")) {
                add(s);
            }
            for k in cfg.keys() {
                // dxt:…:<org>, coworkTrustedDeviceToken:<аккаунт>
                if let Some(tail) = k.rsplit(':').next() {
                    add(tail);
                }
            }
        }
        if let Some(reg) = read_json(&d.join("ant-device-registry.json")) {
            for k in reg.keys() {
                add(k);
            }
        }
        if let Some(Value::String(s)) = read_json(&d.join("cowork-enabled-cli-ops.json")).and_then(|m| m.get("ownerAccountId").cloned()) {
            add(&s);
        }
    }
    if let Some(acc) = env.code_json.as_deref().and_then(read_json).and_then(|m| m.get("oauthAccount").cloned()) {
        for k in ["accountUuid", "organizationUuid"] {
            if let Some(Value::String(s)) = acc.get(k) {
                add(s);
            }
        }
    }
    ids
}

/// Убрать (рекурсивно) ключи, в имени которых есть id аккаунта/организации.
fn scrub_ids(v: &mut Value, ids: &BTreeSet<String>) -> usize {
    let mut removed = 0;
    match v {
        Value::Object(m) => {
            let before = m.len();
            m.retain(|k, _| {
                let k = k.to_lowercase();
                !ids.iter().any(|id| k.contains(id.as_str()))
            });
            removed += before - m.len();
            for val in m.values_mut() {
                removed += scrub_ids(val, ids);
            }
        }
        Value::Array(a) => {
            for val in a {
                removed += scrub_ids(val, ids);
            }
        }
        _ => {}
    }
    removed
}

/// Новое содержимое JSON-файла и сколько записей уходит (None — нечего).
fn json_edit(action: &Action, ids: &BTreeSet<String>) -> Option<(PathBuf, Map<String, Value>, usize)> {
    let (path, keep): (&PathBuf, Box<dyn Fn(&str) -> bool>) = match action {
        Action::DesktopConfig(p) => (p, Box::new(|k: &str| DESKTOP_CONFIG_KEEP.contains(&k))),
        Action::CodeConfig(p) => (p, Box::new(|k: &str| !code_key_is_account(k))),
        Action::DesktopPrefs(p) => {
            let mut map = read_json(p)?;
            let mut v = Value::Object(std::mem::take(&mut map));
            let mut n = scrub_ids(&mut v, ids);
            // Имя устройства для удалённого доступа — сервер видит его у аккаунта.
            if let Some(prefs) = v.get_mut("preferences").and_then(Value::as_object_mut) {
                n += usize::from(prefs.remove("remoteToolsDeviceName").is_some());
            }
            let Value::Object(m) = v else { return None };
            return (n > 0).then(|| (p.clone(), m, n));
        }
        _ => return None,
    };
    let mut map = read_json(path)?;
    let before = map.len();
    map.retain(|k, _| keep(k));
    let mut n = before - map.len();
    // Оставшиеся ключи тоже чистим от записей под аккаунт (на всякий случай).
    let mut v = Value::Object(map);
    n += scrub_ids(&mut v, ids);
    let Value::Object(m) = v else { return None };
    (n > 0).then(|| (path.clone(), m, n))
}

// ─── Скан ───────────────────────────────────────────────────────────

pub fn scan_env(env: &Env) -> Scan {
    let ids = account_ids(env);
    let mut groups: Vec<GroupInfo> = Vec::new();
    let mut add = |group: Group, code: bool, bytes: u64, items: u32| {
        if items == 0 {
            return;
        }
        match groups.iter_mut().find(|g| g.id == group && g.code == code) {
            Some(g) => {
                g.bytes += bytes;
                g.items += items;
            }
            None => groups.push(GroupInfo { id: group, code, bytes, items }),
        }
    };
    for t in targets(env) {
        match &t.action {
            Action::Remove(p) => {
                let (b, n) = size_of(p);
                add(t.group, t.code, b, n);
            }
            Action::Keychain(_) => {}
            other => {
                if let Some((_, _, n)) = json_edit(other, &ids) {
                    add(t.group, t.code, 0, n as u32);
                }
            }
        }
    }
    groups.sort_by_key(|g| (g.code, g.id));
    let kept_sessions = env
        .desktop_dirs
        .iter()
        .map(|d| count_sessions(&d.join("claude-code-sessions")) + count_sessions(&d.join("local-agent-mode-sessions")))
        .sum();
    Scan {
        desktop_dirs: env.desktop_dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect(),
        code_found: env.code_json.as_ref().is_some_and(|p| p.exists())
            || env.code_dir.as_ref().is_some_and(|p| p.exists()),
        groups,
        desktop_running: false,
        cli_running: 0,
        kept_sessions,
    }
}

/// Сессии: файлы local_*.json на уровне <аккаунт>/<организация>.
fn count_sessions(root: &Path) -> u32 {
    let mut n = 0;
    for acc in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for org in std::fs::read_dir(acc.path()).into_iter().flatten().flatten() {
            n += std::fs::read_dir(org.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with("local_") && name.ends_with(".json")
                })
                .count() as u32;
        }
    }
    n
}

// ─── Очистка ────────────────────────────────────────────────────────

/// Удалить с повторами: только что закрытая программа ещё держит файлы.
fn remove(p: &Path) -> Result<(u64, u32), String> {
    let (bytes, n) = size_of(p);
    if n == 0 {
        return Ok((0, 0));
    }
    let mut last = String::new();
    for attempt in 0..8 {
        let meta = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(_) => return Ok((bytes, n)),
        };
        let r = if meta.is_dir() && !meta.file_type().is_symlink() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        };
        match r {
            Ok(()) => return Ok((bytes, n)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((bytes, n)),
            Err(e) => last = e.to_string(),
        }
        if attempt < 7 {
            std::thread::sleep(Duration::from_millis(350));
        }
    }
    Err(last)
}

/// Записать JSON через временный файл (обрыв не оставит половину файла).
fn write_json(p: &Path, m: Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("clod-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn wipe_env(env: &Env, include_desktop: bool, include_code: bool, include_chats: bool) -> WipeResult {
    let ids = account_ids(env);
    let mut res = WipeResult { removed_bytes: 0, removed_items: 0, failed: Vec::new(), closed_desktop: false };
    let wanted = |t: &Target| match (t.group, t.code) {
        (Group::Chats, _) => include_chats,
        (_, true) => include_code,
        (_, false) => include_desktop,
    };
    for t in targets(env).into_iter().filter(wanted) {
        match &t.action {
            Action::Remove(p) => match remove(p) {
                Ok((b, n)) => {
                    res.removed_bytes += b;
                    res.removed_items += n;
                }
                Err(e) => res.failed.push(format!("{} — {e}", p.display())),
            },
            Action::Keychain(service) => keychain_delete(service),
            other => {
                if let Some((path, map, n)) = json_edit(other, &ids) {
                    match write_json(&path, map) {
                        Ok(()) => res.removed_items += n as u32,
                        Err(e) => res.failed.push(format!("{} — {e}", path.display())),
                    }
                }
            }
        }
    }
    res
}

#[cfg(target_os = "macos")]
fn keychain_delete(service: &str) {
    // Записей может быть несколько — удаляем, пока находятся.
    for _ in 0..5 {
        let ok = std::process::Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", service])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            break;
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn keychain_delete(_service: &str) {}

// ─── Процессы ───────────────────────────────────────────────────────

/// (Claude Desktop запущен, сколько отдельных Claude Code запущено).
#[cfg(target_os = "windows")]
fn running(env: &Env) -> (Vec<u32>, u32) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let desktop_exe = crate::claude_desktop::detect().map(|i| i.exe_path.to_lowercase());
    let data: Vec<String> = env.desktop_dirs.iter().map(|d| d.to_string_lossy().to_lowercase()).collect();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut desktop = Vec::new();
    let mut cli = 0;
    for (pid, p) in sys.processes() {
        if !p.name().to_string_lossy().eq_ignore_ascii_case("claude.exe") {
            continue;
        }
        let exe = p.exe().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let is_desktop = desktop_exe.as_deref().is_some_and(|d| d == exe) || exe.contains("\\windowsapps\\anthropicpbc.claude_");
        // Claude Code, запущенный самим Desktop (вкладка Code), — из его userData.
        let is_desktop_child = data.iter().any(|d| exe.starts_with(d.as_str()));
        if is_desktop {
            desktop.push(pid.as_u32());
        } else if !is_desktop_child {
            cli += 1;
        }
    }
    (desktop, cli)
}

#[cfg(target_os = "macos")]
fn running(_env: &Env) -> (Vec<u32>, u32) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut desktop = Vec::new();
    let mut cli = 0;
    for (pid, p) in sys.processes() {
        let exe = p.exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        if exe.contains("/Claude.app/Contents/MacOS/") {
            desktop.push(pid.as_u32());
        } else if p.name().to_string_lossy() == "claude" && !exe.contains("/Application Support/Claude/") {
            cli += 1;
        }
    }
    (desktop, cli)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn running(_env: &Env) -> (Vec<u32>, u32) {
    (Vec::new(), 0)
}

/// Закрыть Claude Desktop, как это делает Windows перед обновлением, —
/// он сохраняет состояние и выходит; не вышел — завершаем.
#[cfg(target_os = "windows")]
fn close_desktop(env: &Env) -> Result<bool, String> {
    use std::time::Instant;
    let pids = || running(env).0;
    let first = pids();
    if first.is_empty() {
        return Ok(false);
    }
    let wait = |secs: u64| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !pids().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
        }
        pids().is_empty()
    };
    let _ = crate::win::end_session(&first);
    if !wait(6) {
        crate::apps::kill_pids(&pids());
        if !wait(5) {
            return Err("Claude Desktop не закрылся — закройте его вручную (в трее тоже) и нажмите «Очистить» снова".into());
        }
    }
    // Claude Code вкладки Code, оставшийся от Desktop, держит файлы.
    kill_desktop_children(env);
    Ok(true)
}

#[cfg(target_os = "windows")]
fn kill_desktop_children(env: &Env) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let data: Vec<String> = env.desktop_dirs.iter().map(|d| d.to_string_lossy().to_lowercase()).collect();
    if data.is_empty() {
        return;
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    for p in sys.processes().values() {
        let exe = p.exe().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if data.iter().any(|d| exe.starts_with(d.as_str())) {
            p.kill();
        }
    }
    std::thread::sleep(Duration::from_millis(600));
}

#[cfg(target_os = "macos")]
fn close_desktop(env: &Env) -> Result<bool, String> {
    if running(env).0.is_empty() {
        return Ok(false);
    }
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"Claude\" to quit"])
        .status();
    for _ in 0..40 {
        if running(env).0.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Claude Desktop не закрылся — закройте его вручную и нажмите «Очистить» снова".into())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn close_desktop(_env: &Env) -> Result<bool, String> {
    Ok(false)
}

// ─── Для команд ─────────────────────────────────────────────────────

pub fn scan() -> Scan {
    let env = env();
    let mut s = scan_env(&env);
    let (desktop, cli) = running(&env);
    s.desktop_running = !desktop.is_empty();
    s.cli_running = cli;
    s
}

pub fn wipe(include_desktop: bool, include_code: bool, include_chats: bool) -> Result<WipeResult, String> {
    let env = env();
    if !include_desktop && !include_code && !include_chats {
        return Err("Не выбрано, что очищать".into());
    }
    if include_desktop && env.desktop_dirs.is_empty() && !include_code {
        return Err("Данные Claude Desktop на этом компьютере не найдены".into());
    }
    // Claude Desktop закрываем, только если чистим его данные.
    // Claude Desktop держит свои файлы и список чатов — закрываем, если трогаем их.
    let closed = if include_desktop || include_chats { close_desktop(&env)? } else { false };
    tracing::info!(
        "очистка данных Claude (Desktop: {include_desktop}, Code: {include_code}, чаты: {include_chats})"
    );
    let mut res = wipe_env(&env, include_desktop, include_code, include_chats);
    res.closed_desktop = closed;
    if !res.failed.is_empty() {
        tracing::warn!("очистка Claude: не удалось удалить {} шт.", res.failed.len());
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACC: &str = "21bfa578-1aae-41f1-87f2-b57c42b5d8d2";
    const ORG: &str = "e25c5de5-c130-4294-9dab-623b0f5e7624";

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn wipes_account_traces_and_keeps_sessions() {
        let root = std::env::temp_dir().join(format!("clod-wipe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("Claude-Data").join("anthropicpbc.claude_x");
        let home = root.join("home");

        // Claude Desktop.
        write(&d.join("Network").join("Cookies"), "cookies");
        write(&d.join("Local Storage").join("leveldb").join("000001.log"), "ls");
        write(&d.join("IndexedDB").join("x").join("y"), "idb");
        write(&d.join("Cache").join("Cache_Data").join("f"), "0123456789");
        write(&d.join("ant-did"), "device");
        write(&d.join("ant-device-key-generation.json"), "{}");
        write(&d.join("host-creds-abc.json"), "{}");
        write(&d.join("Partitions").join("browser").join("Network").join("Cookies"), "c");
        write(&d.join("Local State"), r#"{"os_crypt":{"encrypted_key":"k"}}"#);
        write(&d.join("sentry").join("scope_v3.json"), "{}");
        write(&d.join("logs").join("main.log"), "user@example.com");
        write(
            &d.join("config.json"),
            &format!(
                r#"{{"locale":"ru-RU","oauth:tokenCacheV2":"secret","lastKnownAccountUuid":"{ACC}","coworkTrustedDeviceToken:{ACC}":"t","dxt:allowlistEnabled:{ORG}":true,"bootFrameLayout":{{"sidebarWidth":300}}}}"#
            ),
        );
        write(
            &d.join("claude_desktop_config.json"),
            &format!(
                r#"{{"mcpServers":{{"fs":{{"command":"npx"}}}},"preferences":{{"sidebarMode":"default","remoteToolsDeviceName":"MYPC","bypassPermissionsGateByAccount":{{"{ACC}":true}},"epitaxyPrefs":{{"x.{ACC}":true,"keep":1}}}}}}"#
            ),
        );
        // Сессии Code — остаются.
        let sess = d.join("claude-code-sessions").join(ACC).join(ORG).join("local_1.json");
        write(&sess, "{}");
        let cowork = d.join("local-agent-mode-sessions").join(ACC).join(ORG).join("scheduled-tasks.json");
        write(&cowork, "[]");
        write(&d.join("window-state.json"), "{}");
        write(&d.join("claude-code").join("2.1").join("claude.exe"), "bin");

        // Claude Code.
        let code_dir = home.join(".claude");
        let code_json = home.join(".claude.json");
        write(
            &code_json,
            &format!(
                r#"{{"numStartups":3,"userID":"u","machineID":"m","oauthAccount":{{"accountUuid":"{ACC}","emailAddress":"a@b"}},"cachedGrowthBookFeatures":{{}},"projects":{{"C:/p":{{"allowedTools":[]}}}},"hasCompletedOnboarding":true}}"#
            ),
        );
        write(&home.join(".claude.json.backup"), "old");
        write(&code_dir.join(".credentials.json"), "{}");
        write(&code_dir.join("telemetry").join("e.json"), "{}");
        write(&code_dir.join("backups").join(".claude.json.backup.1"), "{}");
        let transcript = code_dir.join("projects").join("p").join("s.jsonl");
        write(&transcript, "chat");
        write(&code_dir.join("settings.json"), "{}");

        let env = Env {
            desktop_dirs: vec![d.clone()],
            desktop_extra: vec![],
            code_dir: Some(code_dir.clone()),
            code_json: Some(code_json.clone()),
            code_json_dir: Some(home.clone()),
            code_extra: vec![],
            keychain: false,
        };

        let s = scan_env(&env);
        assert!(s.groups.iter().any(|g| g.id == Group::Login && !g.code));
        assert!(s.groups.iter().any(|g| g.id == Group::Cache && g.bytes >= 10));
        assert!(s.groups.iter().any(|g| g.code));
        assert_eq!(s.kept_sessions, 1);

        // Только Desktop: Claude Code не тронут.
        let r = wipe_env(&env, true, false, false);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        for gone in ["Network", "Local Storage", "IndexedDB", "Cache", "ant-did", "sentry", "logs", "ant-device-key-generation.json", "host-creds-abc.json", "Partitions"] {
            assert!(!d.join(gone).exists(), "{gone} должен быть удалён");
        }
        let cfg = read_json(&d.join("config.json")).unwrap();
        assert_eq!(cfg.keys().cloned().collect::<Vec<_>>(), vec!["bootFrameLayout", "locale"]);
        let prefs = std::fs::read_to_string(d.join("claude_desktop_config.json")).unwrap();
        assert!(!prefs.contains(ACC), "записи под аккаунт убраны");
        assert!(!prefs.contains("MYPC"), "имя устройства убрано");
        assert!(prefs.contains("mcpServers") && prefs.contains("sidebarMode") && prefs.contains("\"keep\""));
        assert!(sess.exists() && cowork.exists(), "сессии остаются");
        assert!(d.join("window-state.json").exists() && d.join("claude-code").exists());
        assert!(d.join("Local State").exists(), "ключ шифрования остаётся");
        assert!(code_dir.join(".credentials.json").exists(), "Claude Code не тронут");

        // Вместе с Claude Code.
        let r = wipe_env(&env, true, true, false);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        let cj = read_json(&code_json).unwrap();
        for k in ["userID", "machineID", "oauthAccount", "cachedGrowthBookFeatures"] {
            assert!(!cj.contains_key(k), "{k} убран");
        }
        assert!(cj.contains_key("projects") && cj.contains_key("numStartups") && cj.contains_key("hasCompletedOnboarding"));
        assert!(!home.join(".claude.json.backup").exists());
        assert!(!code_dir.join(".credentials.json").exists());
        assert!(!code_dir.join("telemetry").exists());
        assert!(!code_dir.join("backups").join(".claude.json.backup.1").exists());
        assert!(transcript.exists() && code_dir.join("settings.json").exists(), "чаты Code остаются");

        // Повторная очистка — ничего не ломает и ничего не находит.
        let again = scan_env(&env);
        assert!(again.groups.iter().all(|g| g.id == Group::Chats), "остались только чаты: {:?}", again.groups);
        assert_eq!(again.kept_sessions, 1);

        // Отдельно — локальные чаты: удаляются только по своему флагу.
        let r = wipe_env(&env, false, false, true);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert!(!sess.exists() && !cowork.exists(), "чаты Desktop удалены");
        assert!(!transcript.exists(), "переписка Code удалена");
        assert!(code_dir.join("settings.json").exists(), "настройки Claude Code остаются");
        assert!(d.join("window-state.json").exists() && d.join("claude-code").exists());
        assert!(scan_env(&env).groups.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uuid_detection() {
        assert!(is_uuid(ACC));
        assert!(!is_uuid("local_21bfa578-1aae-41f1-87f2-b57c42b5d8d2"));
        assert!(!is_uuid("skills-plugin"));
    }

    #[test]
    #[ignore = "только чтение: смотрит настоящие данные Claude на этом ПК"]
    fn scan_real_machine() {
        let s = scan();
        eprintln!("{s:#?}");
    }
}
