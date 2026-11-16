//! «Инструменты» → очистка данных аккаунта Claude Desktop и Claude Code.
//!
//! Задача: сменить аккаунт так, чтобы новый ничем не пересекался с прошлым
//! на этом компьютере. Удаляем всё, что несёт отпечаток аккаунта или
//! устройства: вход (куки, токены), постоянные идентификаторы устройства
//! (ant-did, соли, ключи устройства в TPM, id Claude Code), хранилище сайта
//! claude.ai, кэш под аккаунт, журналы и временные файлы.
//!
//! Не трогаем (если не выбрано удаление локальных чатов):
//!   - чаты и проекты в облаке — они на серверах Anthropic, в самом аккаунте;
//!   - локальные сессии вкладки Code и задачи Cowork (claude-code-sessions,
//!     local-agent-mode-sessions, ~/.claude/projects) — они разложены по
//!     папкам аккаунта и вернутся, если снова войти в прошлый. Перед
//!     удалением их можно выгрузить в архив (`claude_chats_export`);
//!   - настройки MCP и прочие свои настройки (из файлов настроек убираются
//!     только записи, привязанные к аккаунту или устройству);
//!   - саму программу и Claude Code внутри неё.
//!
//! Где лежат данные (Electron userData):
//!   - Windows, Store (новые версии): %LOCALAPPDATA%\Claude-Data\<пакет>
//!   - Windows, Store (старые): %LOCALAPPDATA%\Packages\<пакет>\LocalCache\Roaming\Claude
//!   - Windows, установщик: %APPDATA%\Claude
//!   - macOS: ~/Library/Application Support/Claude
//! Claude Code (им пользуется и вкладка Code): ~/.claude.json и ~/.claude
//! (или CLAUDE_CONFIG_DIR). Вне файлов: ключи устройства Claude Desktop в
//! TPM (Windows) и записи Claude Code в Диспетчере учётных данных / Связке
//! ключей.
//!
//! Ключ к полноте: любой ключ JSON, в имени которого есть UUID (id
//! аккаунта, организации, сессии), — запись «под аккаунт». Их убираем все,
//! а не по списку: новые версии Claude добавляют такие ключи постоянно.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// Журналы, телеметрия, отчёты об ошибках, временные файлы.
    Logs,
    /// Кэш страниц и видеокарты.
    Cache,
    /// Локальные чаты и проекты (вкладка Code, Cowork). Удаляются, только
    /// если это выбрано отдельно: по умолчанию остаются на компьютере.
    Chats,
}

/// Локальные чаты Claude Desktop (в его папке данных).
pub(crate) const DESKTOP_CHATS: &[&str] = &[
    "claude-code-sessions",
    "local-agent-mode-sessions",
    "scratch-workspaces",
    "side-session-notes",
    "git-worktrees.json",
];
/// Переписка и связанное у Claude Code (~/.claude).
const CODE_CHATS: &[&str] = &[
    "projects",
    "file-history",
    "todos",
    "sessions",
    "session-env",
    "shell-snapshots",
    "plans",
    "uploads",
    "history.jsonl",
];

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
    /// Claude Code, запущенный отдельно (терминал, VS Code): закроем,
    /// если очищаем Claude Code или чаты, — иначе он запишет данные обратно.
    pub cli_running: u32,
    /// Локальные сессии вкладки Code и Cowork, которые остаются.
    pub kept_sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Running {
    pub desktop_running: bool,
    pub cli_running: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeResult {
    pub removed_bytes: u64,
    pub removed_items: u32,
    /// Что удалить не удалось (путь — причина).
    pub failed: Vec<String>,
    /// Claude Desktop был запущен и закрыт для очистки.
    pub closed_desktop: bool,
    /// Архив локальных чатов, сохранённый перед их удалением.
    pub archive: Option<crate::claude_chats_export::Archive>,
}

#[derive(Debug)]
enum Action {
    /// Удалить файл или папку целиком.
    Remove(PathBuf),
    /// config.json Claude Desktop: оставить только нейтральные ключи.
    DesktopConfig(PathBuf),
    /// claude_desktop_config.json: убрать записи аккаунта и устройства.
    DesktopPrefs(PathBuf),
    /// Любой JSON: убрать записи с UUID в имени ключа.
    Scrub(PathBuf),
    /// «Local State» Chromium: убрать дату установки, ключ шифрования оставить.
    LocalState(PathBuf),
    /// ~/.claude.json: убрать аккаунт, идентификаторы и кэш сервера.
    CodeConfig(PathBuf),
    /// macOS: запись Claude Code в Связке ключей.
    #[allow(dead_code)]
    Keychain(&'static str),
    /// Windows: ключ устройства Claude Desktop в TPM.
    #[allow(dead_code)]
    TpmKey(String),
    /// Windows: запись Claude Code в Диспетчере учётных данных.
    #[allow(dead_code)]
    Credential(String),
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
    /// Где лежат .claude.json.backup* и другие конфиги (домашняя папка).
    pub code_json_dir: Option<PathBuf>,
    /// Кэш/журналы Claude Code вне ~/.claude.
    pub code_extra: Vec<PathBuf>,
    /// Другие конфиги Claude Code (~/.claude.json при CLAUDE_CONFIG_DIR).
    pub code_json_extra: Vec<PathBuf>,
    /// Временная папка: Claude Code пишет туда вывод задач и рабочие папки.
    pub temp_dir: Option<PathBuf>,
    pub keychain: bool,
    /// Смотреть хранилища системы (TPM, Диспетчер учётных данных).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub os_stores: bool,
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
    // Поколение аппаратного ключа устройства (сами ключи — в TPM, их
    // удаляем отдельно: Action::TpmKey).
    (Group::Device, "ant-device-key-generation.json"),
    // Соль id камер и микрофонов, которые видит страница (постоянный отпечаток).
    // «Local State» целиком не удаляем: там ключ шифрования секретов
    // расширений, наружу он не уходит, а без него сломаются их настройки.
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
    // Плагины организации: папки <аккаунт>/<организация>, скачаются заново.
    (Group::Account, "plugin-list"),
    // Ключ «<аккаунт>:<организация>» и сброс памяти Cowork под него.
    (Group::Account, "cowork-memory-resets.json"),
    // Список блокировки расширений организации (адрес с id организации).
    (Group::Account, "extensions-blocklist.json"),
    (Group::Account, "remote-file-download-parts.json"),
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
    (Group::Cache, "shared_proto_db"),
    (Group::Cache, "VideoDecodeStats"),
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
    // Тема оформления — своя настройка, не про аккаунт.
    "userThemeMode",
];

/// claude_desktop_config.json → preferences: устройство и его привязки.
const DESKTOP_PREFS_DROP: &[&str] = &[
    // Имя устройства для удалённого доступа — сервер видит его у аккаунта.
    "remoteToolsDeviceName",
    // Связка с расширением Claude для Chrome: id устройства и браузера.
    "chromeExtension",
];

/// ~/.claude.json: ключи аккаунта, идентификаторы установки и счётчики,
/// по которым видно «тот же компьютер» (в нижнем регистре).
const CODE_JSON_DROP: &[&str] = &[
    "oauthaccount",
    "userid",
    "machineid",
    "anonymousid",
    "remotecontrolmachineid",
    "summonsidkey",
    "hostbootstrapseed",
    "firststarttime",
    "firststartversion",
    "claudecodefirsttokendate",
    "primaryapikey",
    "customapikeyresponses",
    "knownaccounts",
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
    "grovechoiceseenat",
    "hasshownopus46notice",
    "programusecasechoices",
    "promocreditcheck",
    "orgsurveycheck",
    "managedsettingsnoticeseenorg",
    "lastseenorgdefaultupdatedat",
    "trainingconsentsettledat",
    "mcplegacykeyowners",
    "penguinmodeorgenabled",
    "startupprefetchedat",
    "bridgeoauthdeadexpiresat",
    "githubrepopaths",
    "numstartups",
    "tipshistory",
    "skillusage",
    "toolusage",
    "pluginusage",
    "pluginusagelspgraceappliedids",
];

/// Части имён ключей, которые всегда про аккаунт или устройство.
const CODE_JSON_DROP_PARTS: &[&str] = &[
    "cache",
    "eligib",
    "subscription",
    "overage",
    "extrausage",
    "oauth",
    "account",
    "organization",
    "statsig",
    "growthbook",
    "trusteddevice",
    "deviceid",
    "byprincipal",
    "byorg",
];

/// …и всё, что сервер прислал под этот аккаунт (флаги, лимиты, модели).
fn code_key_is_account(key: &str) -> bool {
    let k = key.to_lowercase();
    CODE_JSON_DROP.contains(&k.as_str()) || CODE_JSON_DROP_PARTS.iter().any(|w| k.contains(w))
}

/// Файлы Claude Code в ~/.claude, привязанные к аккаунту или устройству.
const CODE_DIR_REMOVE: &[(Group, &str)] = &[
    (Group::Login, ".credentials.json"),
    // Ключ устройства и id строки устройства на сервере (у каждого аккаунта свой).
    (Group::Login, ".device-keys.json"),
    (Group::Login, ".session_ingress_token"),
    (Group::Login, "hfi-auth.json"),
    (Group::Device, "statsig"),
    (Group::Account, "remote-settings.json"),
    (Group::Account, "remote-settings-consent.json"),
    (Group::Account, "policy-limits.json"),
    (Group::Account, "mcp-needs-auth-cache.json"),
    (Group::Account, "bridge-pointer.json"),
    // Каталог моделей: имена файлов — id организации и хэш аккаунта.
    (Group::Account, "cache/model-catalog"),
    (Group::Logs, "telemetry"),
    (Group::Logs, "debug"),
    (Group::Logs, "stats-cache.json"),
];

/// Подписи и копии файлов настроек сервера: policy-limits.signature*.json и т.п.
fn code_dir_extra(name: &str) -> Option<Group> {
    let n = name.to_lowercase();
    if (n.starts_with("policy-limits") || n.starts_with("remote-settings")) && n.ends_with(".json") {
        return Some(Group::Account);
    }
    None
}

fn targets(env: &Env) -> Vec<Target> {
    let mut v = Vec::new();
    let mut push = |group: Group, code: bool, action: Action| v.push(Target { group, code, action });
    for d in &env.desktop_dirs {
        for (group, rel) in DESKTOP_REMOVE {
            push(*group, false, Action::Remove(d.join(rel)));
        }
        for entry in read_dir(d) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("host-creds-") {
                push(Group::Login, false, Action::Remove(entry.path()));
            }
        }
        push(Group::Login, false, Action::DesktopConfig(d.join("config.json")));
        push(Group::Account, false, Action::DesktopPrefs(d.join("claude_desktop_config.json")));
        // Пресеты сессий разложены по аккаунтам: byAccount.<id аккаунта>.
        push(Group::Account, false, Action::Scrub(d.join("ccd-session-presets.json")));
        push(Group::Device, false, Action::LocalState(d.join("Local State")));
    }
    for (group, p) in &env.desktop_extra {
        push(*group, false, Action::Remove(p.clone()));
    }
    #[cfg(target_os = "windows")]
    if env.os_stores {
        let (tpm, creds) = os_store_items();
        for name in tpm {
            push(Group::Device, false, Action::TpmKey(name));
        }
        for name in creds {
            push(Group::Login, true, Action::Credential(name));
        }
    }

    // Claude Code: основной конфиг и все его копии и варианты.
    if let Some(json) = &env.code_json {
        push(Group::Login, true, Action::CodeConfig(json.clone()));
    }
    for json in &env.code_json_extra {
        push(Group::Login, true, Action::CodeConfig(json.clone()));
    }
    let backups = |dir: &Path| -> Vec<PathBuf> {
        read_dir(dir)
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with(".claude.json.backup") || n.starts_with(".claude.json.tmp") || n.starts_with(".claude.json.corrupted")
            })
            .map(|e| e.path())
            .collect()
    };
    if let Some(home) = &env.code_json_dir {
        for p in backups(home) {
            push(Group::Device, true, Action::Remove(p));
        }
        // Варианты конфига: .claude-custom-oauth.json, .claude-staging-oauth.json…
        // Только они: чужие файлы с похожим именем не трогаем.
        for e in read_dir(home) {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with(".claude-") && n.ends_with("-oauth.json") && e.path().is_file() {
                push(Group::Login, true, Action::CodeConfig(e.path()));
            }
        }
    }
    if let Some(dir) = &env.code_dir {
        for (group, rel) in CODE_DIR_REMOVE {
            push(*group, true, Action::Remove(dir.join(rel)));
        }
        for e in read_dir(dir) {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(group) = code_dir_extra(&n) {
                if !CODE_DIR_REMOVE.iter().any(|(_, r)| *r == n) {
                    push(group, true, Action::Remove(e.path()));
                }
            }
        }
        for p in backups(&dir.join("backups")) {
            push(Group::Device, true, Action::Remove(p));
        }
        // Старое имя конфига: если он есть, Claude Code читает его вместо ~/.claude.json.
        push(Group::Login, true, Action::CodeConfig(dir.join(".config.json")));
        // Конфиг внутри папки (CLAUDE_CONFIG_DIR=~/.claude у другой установки).
        if env.code_json.as_deref() != Some(dir.join(".claude.json").as_path()) {
            push(Group::Login, true, Action::CodeConfig(dir.join(".claude.json")));
        }
    }
    for p in &env.code_extra {
        push(Group::Logs, true, Action::Remove(p.clone()));
    }
    // Временные файлы Claude Code: вывод задач, рабочие папки сессий.
    if let Some(tmp) = &env.temp_dir {
        for e in read_dir(tmp) {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n == "claude" || (n.starts_with("claude-") && n.ends_with("-cwd")) {
                push(Group::Logs, true, Action::Remove(e.path()));
            }
        }
    }
    if env.keychain {
        push(Group::Login, true, Action::Keychain("Claude Code-credentials"));
        push(Group::Login, true, Action::Keychain("Claude Code-device-keys"));
    }
    // Локальные чаты — отдельная цель (группа Chats), по умолчанию не трогаем.
    for d in &env.desktop_dirs {
        for rel in DESKTOP_CHATS {
            push(Group::Chats, false, Action::Remove(d.join(rel)));
        }
    }
    if let Some(dir) = &env.code_dir {
        for rel in CODE_CHATS {
            push(Group::Chats, false, Action::Remove(dir.join(rel)));
        }
    }
    v
}

fn read_dir(d: &Path) -> impl Iterator<Item = std::fs::DirEntry> {
    std::fs::read_dir(d).into_iter().flatten().flatten()
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
    let mut e = Env { os_stores: true, temp_dir: Some(std::env::temp_dir()), ..Env::default() };
    let home = dirs::home_dir();

    #[cfg(target_os = "windows")]
    {
        let local = dirs::data_local_dir();
        let roaming = dirs::data_dir();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(l) = &local {
            // Store, новые версии: данные вынесены из пакета.
            for entry in read_dir(&l.join("Claude-Data")) {
                candidates.push(entry.path());
            }
            // Store, старые версии: виртуализированный %APPDATA% пакета.
            for entry in read_dir(&l.join("Packages")) {
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
    // Конфиг в домашней папке при этом тоже чистим: он остаётся от прежних запусков.
    match std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(dir) => {
            e.code_json = Some(dir.join(".claude.json"));
            e.code_json_dir = Some(dir.clone());
            e.code_dir = Some(dir);
            if let Some(h) = &home {
                e.code_json_extra.push(h.join(".claude.json"));
            }
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

/// Размер файла/папки без перехода по ссылкам (ссылка — 0). None — пути нет.
fn size_of(p: &Path) -> Option<(u64, u32)> {
    let meta = std::fs::symlink_metadata(p).ok()?;
    if meta.file_type().is_symlink() {
        return Some((0, 1));
    }
    if meta.is_file() {
        return Some((meta.len(), 1));
    }
    let mut bytes = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in read_dir(&d) {
            // Метаданные из самого перечисления папки: без лишнего обращения
            // к диску на каждый файл (во временной папке их сотни тысяч).
            let Ok(m) = entry.metadata() else { continue };
            if m.is_dir() && !m.file_type().is_symlink() {
                stack.push(entry.path());
            } else {
                bytes += m.len();
            }
        }
    }
    Some((bytes, 1))
}

/// JSON-объект из файла. Ok(None) — файла нет; Err — есть, но не читается
/// (занят, повреждён). Такой файл нельзя молча пропускать: в нём могут
/// остаться токены.
fn read_json_checked(p: &Path) -> Result<Option<Map<String, Value>>, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match std::fs::read(p) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                return match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
                    Ok(Value::Object(m)) => Ok(Some(m)),
                    Ok(_) => Err("не объект JSON".into()),
                    Err(e) => Err(format!("файл повреждён: {e}")),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => last = e.to_string(),
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    Err(last)
}

fn read_json(p: &Path) -> Option<Map<String, Value>> {
    read_json_checked(p).ok().flatten()
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() }
        })
}

/// Все UUID внутри строки (в нижнем регистре).
fn uuids_in(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 36 <= b.len() {
        if s.is_char_boundary(i) && s.is_char_boundary(i + 36) && is_uuid(&s[i..i + 36]) {
            out.push(s[i..i + 36].to_lowercase());
            i += 36;
        } else {
            i += 1;
        }
    }
    out
}

fn has_uuid(s: &str) -> bool {
    !uuids_in(s).is_empty()
}

/// id аккаунтов и организаций, которые видны на этом компьютере: папки,
/// разложенные как <аккаунт>/<организация>, ключи настроек и сохранённые
/// значения — в том числе прошлых аккаунтов из резервных копий.
fn account_ids(env: &Env) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut add = |s: &str| {
        if is_uuid(s) {
            ids.insert(s.to_lowercase());
        }
    };
    let key_ids = |m: &Map<String, Value>, add: &mut dyn FnMut(&str)| {
        fn walk(v: &Value, add: &mut dyn FnMut(&str), depth: u32) {
            if depth > 6 {
                return;
            }
            if let Value::Object(m) = v {
                for (k, val) in m {
                    for id in uuids_in(k) {
                        add(&id);
                    }
                    walk(val, add, depth + 1);
                }
            }
        }
        walk(&Value::Object(m.clone()), add, 0);
    };
    for d in &env.desktop_dirs {
        for sub in ["claude-code-sessions", "local-agent-mode-sessions", "scratch-workspaces", "plugin-list"] {
            for acc in read_dir(&d.join(sub)) {
                add(&acc.file_name().to_string_lossy());
                for org in read_dir(&acc.path()) {
                    add(&org.file_name().to_string_lossy());
                }
            }
        }
        for f in ["config.json", "claude_desktop_config.json", "ant-device-registry.json", "ccd-session-presets.json"] {
            if let Some(m) = read_json(&d.join(f)) {
                key_ids(&m, &mut add);
                if f == "config.json" {
                    if let Some(Value::String(s)) = m.get("lastKnownAccountUuid") {
                        add(s);
                    }
                    if let Some(Value::String(s)) = m.get("bootSideSessions").and_then(|b| b.get("accountUuid")) {
                        add(s);
                    }
                    if let Some(ssh) = m.get("sshTransportFlagLastAnswer") {
                        for k in ["accountUuid", "orgUuid"] {
                            if let Some(Value::String(s)) = ssh.get(k) {
                                add(s);
                            }
                        }
                    }
                }
            }
        }
        if let Some(Value::String(s)) = read_json(&d.join("cowork-enabled-cli-ops.json")).and_then(|m| m.get("ownerAccountId").cloned()) {
            add(&s);
        }
    }
    // ~/.claude.json и его копии: там же и прошлые аккаунты.
    let mut code_files: Vec<PathBuf> = env.code_json.iter().cloned().collect();
    if let Some(home) = &env.code_json_dir {
        code_files.extend(
            read_dir(home)
                .filter(|e| e.file_name().to_string_lossy().starts_with(".claude.json."))
                .map(|e| e.path()),
        );
    }
    if let Some(dir) = &env.code_dir {
        code_files.extend(read_dir(&dir.join("backups")).map(|e| e.path()));
    }
    for f in code_files {
        let Some(m) = read_json(&f) else { continue };
        if let Some(acc) = m.get("oauthAccount") {
            for k in ["accountUuid", "organizationUuid"] {
                if let Some(Value::String(s)) = acc.get(k) {
                    add(s);
                }
            }
        }
        for k in ["groveChoiceSeenAt", "hasShownOpus46Notice"] {
            if let Some(Value::Object(sub)) = m.get(k) {
                for id in sub.keys() {
                    add(id);
                }
            }
        }
    }
    ids
}

fn mentions(s: &str, ids: &BTreeSet<String>) -> bool {
    let l = s.to_lowercase();
    ids.iter().any(|id| l.contains(id.as_str()))
}

/// Убрать (рекурсивно) записи под аккаунт: ключи с UUID в имени (id
/// аккаунта, организации, сессии) и строки со знакомым id аккаунта.
fn scrub_ids(v: &mut Value, ids: &BTreeSet<String>) -> usize {
    let mut removed = 0;
    match v {
        Value::Object(m) => {
            let before = m.len();
            m.retain(|k, val| !has_uuid(k) && !mentions(k, ids) && !val.as_str().is_some_and(|s| mentions(s, ids)));
            removed += before - m.len();
            for val in m.values_mut() {
                removed += scrub_ids(val, ids);
            }
        }
        Value::Array(a) => {
            let before = a.len();
            a.retain(|x| !x.as_str().is_some_and(|s| mentions(s, ids)));
            removed += before - a.len();
            for val in a {
                removed += scrub_ids(val, ids);
            }
        }
        _ => {}
    }
    removed
}

/// Новое содержимое JSON-файла и сколько записей уходит.
enum Edit {
    /// Нечего менять (или файла нет).
    None,
    Write(Map<String, Value>, usize),
    /// Файл есть, но не читается.
    Broken(String),
}

fn json_edit(action: &Action, ids: &BTreeSet<String>) -> Edit {
    let path = match action {
        Action::DesktopConfig(p)
        | Action::DesktopPrefs(p)
        | Action::Scrub(p)
        | Action::LocalState(p)
        | Action::CodeConfig(p) => p,
        _ => return Edit::None,
    };
    let mut map = match read_json_checked(path) {
        Ok(Some(m)) => m,
        Ok(None) => return Edit::None,
        Err(e) => return Edit::Broken(e),
    };
    let before = map.len();
    let mut n = 0;
    match action {
        Action::DesktopConfig(_) => map.retain(|k, _| DESKTOP_CONFIG_KEEP.contains(&k.as_str())),
        Action::CodeConfig(_) => {
            map.retain(|k, _| !code_key_is_account(k));
            // У проектов — id последних сессий и их счётчики.
            if let Some(Value::Object(projects)) = map.get_mut("projects") {
                for p in projects.values_mut() {
                    if let Value::Object(p) = p {
                        let b = p.len();
                        p.retain(|k, _| !k.starts_with("last") && k != "hasUnseenTeamArtifacts");
                        n += b - p.len();
                    }
                }
            }
        }
        Action::DesktopPrefs(_) => {
            if let Some(Value::Object(prefs)) = map.get_mut("preferences") {
                for k in DESKTOP_PREFS_DROP {
                    n += usize::from(prefs.remove(*k).is_some());
                }
            }
        }
        Action::LocalState(_) => {
            // Дата установки. Ключ шифрования (os_crypt) остаётся.
            n += usize::from(map.remove("uninstall_metrics").is_some());
        }
        _ => {}
    }
    n += before - map.len();
    let mut v = Value::Object(map);
    if !matches!(action, Action::LocalState(_)) {
        n += scrub_ids(&mut v, ids);
    }
    let Value::Object(m) = v else { return Edit::None };
    if n > 0 { Edit::Write(m, n) } else { Edit::None }
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
    let all = targets(env);
    let sizes = sizes_parallel(&all);
    for (t, size) in all.iter().zip(sizes) {
        match &t.action {
            Action::Remove(_) => {
                if let Some((b, n)) = size {
                    add(t.group, t.code, b, n);
                }
            }
            Action::Keychain(_) => {}
            Action::TpmKey(_) | Action::Credential(_) => add(t.group, t.code, 0, 1),
            other => match json_edit(other, &ids) {
                Edit::Write(_, n) => add(t.group, t.code, 0, n as u32),
                Edit::Broken(_) => add(t.group, t.code, 0, 1),
                Edit::None => {}
            },
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

/// Размеры папок — в несколько потоков: большие папки считаются параллельно.
fn sizes_parallel(all: &[Target]) -> Vec<Option<(u64, u32)>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<(u64, u32)>>> = Mutex::new(vec![None; all.len()]);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(t) = all.get(i) else { break };
                if let Action::Remove(p) = &t.action {
                    let size = size_of(p);
                    out.lock().unwrap_or_else(|e| e.into_inner())[i] = size;
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// Сессии: файлы local_*.json на уровне <аккаунт>/<организация>.
fn count_sessions(root: &Path) -> u32 {
    let mut n = 0;
    for acc in read_dir(root) {
        for org in read_dir(&acc.path()) {
            n += read_dir(&org.path())
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

/// Windows: ссылка или точка соединения, указывающая на папку.
fn link_to_dir(meta: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const DIRECTORY: u32 = 0x10;
        const REPARSE_POINT: u32 = 0x400;
        let a = meta.file_attributes();
        a & DIRECTORY != 0 && a & REPARSE_POINT != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = meta;
        false
    }
}

/// Удалить с повторами: только что закрытая программа ещё держит файлы.
fn remove(p: &Path) -> Result<(u64, u32), String> {
    if std::fs::symlink_metadata(p).is_err() {
        return Ok((0, 0));
    }
    let (bytes, n) = size_of(p).unwrap_or((0, 1));
    let mut last = String::new();
    for attempt in 0..8 {
        let meta = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(_) => return Ok((bytes, n)),
        };
        let r = if meta.is_dir() && !meta.file_type().is_symlink() {
            std::fs::remove_dir_all(p)
        } else if link_to_dir(&meta) {
            // Ссылка на папку (точка соединения): убираем саму ссылку, не содержимое.
            std::fs::remove_dir(p)
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
    let mut last = String::new();
    for attempt in 0..5 {
        match std::fs::rename(&tmp, p) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        if attempt < 4 {
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last)
}

pub fn wipe_env(env: &Env, include_desktop: bool, include_code: bool, include_chats: bool) -> WipeResult {
    let ids = account_ids(env);
    let mut res =
        WipeResult { removed_bytes: 0, removed_items: 0, failed: Vec::new(), closed_desktop: false, archive: None };
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
            Action::TpmKey(name) => match tpm_delete(name) {
                Ok(()) => res.removed_items += 1,
                Err(e) => res.failed.push(format!("Ключ устройства {name} — {e}")),
            },
            Action::Credential(name) => match credential_delete(name) {
                Ok(()) => res.removed_items += 1,
                Err(e) => res.failed.push(format!("Учётные данные {name} — {e}")),
            },
            other => match json_edit(other, &ids) {
                Edit::Write(map, n) => {
                    let path = match other {
                        Action::DesktopConfig(p)
                        | Action::DesktopPrefs(p)
                        | Action::Scrub(p)
                        | Action::LocalState(p)
                        | Action::CodeConfig(p) => p,
                        _ => unreachable!(),
                    };
                    match write_json(path, map) {
                        Ok(()) => res.removed_items += n as u32,
                        Err(e) => res.failed.push(format!("{} — {e}", path.display())),
                    }
                }
                Edit::Broken(e) => match other {
                    // config.json Claude Desktop пересоздаёт сам — испорченный удаляем целиком.
                    Action::DesktopConfig(p) => match remove(p) {
                        Ok((b, n)) => {
                            res.removed_bytes += b;
                            res.removed_items += n;
                        }
                        Err(e2) => res.failed.push(format!("{} — {e}; {e2}", p.display())),
                    },
                    Action::DesktopPrefs(p)
                    | Action::Scrub(p)
                    | Action::LocalState(p)
                    | Action::CodeConfig(p) => res.failed.push(format!("{} — {e}", p.display())),
                    _ => {}
                },
                Edit::None => {}
            },
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

// ─── Хранилища Windows: TPM и Диспетчер учётных данных ──────────────

/// Список ключей TPM и учётных данных — не чаще раза в 30 секунд:
/// открывать TPM на каждый скан дорого.
#[cfg(target_os = "windows")]
static OS_STORES: Mutex<Option<(Instant, Vec<String>, Vec<String>)>> = Mutex::new(None);

#[cfg(target_os = "windows")]
fn os_store_items() -> (Vec<String>, Vec<String>) {
    let mut cache = OS_STORES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, tpm, creds)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(30) {
            return (tpm.clone(), creds.clone());
        }
    }
    let tpm = tpm_keys();
    let creds = credential_targets();
    *cache = Some((Instant::now(), tpm.clone(), creds.clone()));
    (tpm, creds)
}

fn forget_os_stores() {
    #[cfg(target_os = "windows")]
    {
        *OS_STORES.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// Ключи устройства Claude Desktop в TPM: «Claude-HwKey-device-registry-<аккаунт>[:gN]».
/// Свой у каждого аккаунта, файлами не удаляются.
#[cfg(target_os = "windows")]
fn tpm_keys() -> Vec<String> {
    use windows_sys::Win32::Security::Cryptography::{
        NCryptEnumKeys, NCryptFreeBuffer, NCryptFreeObject, NCryptKeyName, NCryptOpenStorageProvider,
        MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG,
    };
    let mut out = Vec::new();
    unsafe {
        let mut prov: NCRYPT_PROV_HANDLE = 0;
        if NCryptOpenStorageProvider(&mut prov, MS_PLATFORM_CRYPTO_PROVIDER, 0) != 0 {
            return out;
        }
        let mut state: *mut core::ffi::c_void = std::ptr::null_mut();
        for _ in 0..512 {
            let mut name: *mut NCryptKeyName = std::ptr::null_mut();
            if NCryptEnumKeys(prov, std::ptr::null(), &mut name, &mut state, NCRYPT_SILENT_FLAG) != 0 || name.is_null() {
                break;
            }
            let n = from_wide((*name).pszName);
            NCryptFreeBuffer(name as *mut _);
            if n.starts_with("Claude-HwKey-") {
                out.push(n);
            }
        }
        if !state.is_null() {
            NCryptFreeBuffer(state);
        }
        NCryptFreeObject(prov);
    }
    out
}

#[cfg(target_os = "windows")]
fn tpm_delete(name: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Cryptography::{
        NCryptDeleteKey, NCryptFreeObject, NCryptOpenKey, NCryptOpenStorageProvider, MS_PLATFORM_CRYPTO_PROVIDER,
        NCRYPT_KEY_HANDLE, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG,
    };
    unsafe {
        let mut prov: NCRYPT_PROV_HANDLE = 0;
        let r = NCryptOpenStorageProvider(&mut prov, MS_PLATFORM_CRYPTO_PROVIDER, 0);
        if r != 0 {
            return Err(format!("TPM недоступен (0x{:08X})", r as u32));
        }
        let mut key: NCRYPT_KEY_HANDLE = 0;
        let w = wide(name);
        let r = NCryptOpenKey(prov, &mut key, w.as_ptr(), 0, NCRYPT_SILENT_FLAG);
        let res = if r != 0 {
            Err(format!("не открылся (0x{:08X})", r as u32))
        } else {
            // NCryptDeleteKey освобождает ключ и при успехе, и при ошибке.
            let r = NCryptDeleteKey(key, 0);
            if r != 0 { Err(format!("не удалился (0x{:08X})", r as u32)) } else { Ok(()) }
        };
        NCryptFreeObject(prov);
        res
    }
}

/// Записи Claude Code в Диспетчере учётных данных («Claude Code-credentials…»,
/// «Claude Code-device-keys»). Обычно их нет: токен лежит в .credentials.json.
#[cfg(target_os = "windows")]
fn credential_targets() -> Vec<String> {
    use windows_sys::Win32::Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW};
    let mut out = Vec::new();
    unsafe {
        let filter = wide("Claude Code-*");
        let mut count = 0u32;
        let mut creds: *mut *mut CREDENTIALW = std::ptr::null_mut();
        if CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut creds) == 0 {
            return out;
        }
        for i in 0..count as usize {
            let c = *creds.add(i);
            if !c.is_null() {
                out.push(from_wide((*c).TargetName));
            }
        }
        CredFree(creds as *const _);
    }
    out
}

#[cfg(target_os = "windows")]
fn credential_delete(target: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let w = wide(target);
    if unsafe { CredDeleteW(w.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(1168) {
            return Ok(()); // уже нет
        }
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn tpm_delete(_name: &str) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "windows"))]
fn credential_delete(_target: &str) -> Result<(), String> {
    Ok(())
}

// ─── Процессы ───────────────────────────────────────────────────────

/// Запущенные процессы Claude.
#[derive(Debug, Default)]
struct Procs {
    /// Окно Claude Desktop и его служебные процессы.
    desktop: Vec<u32>,
    /// Запущенное самим Desktop: Claude Code вкладки Code (из его userData),
    /// мост к расширению Chrome.
    helpers: Vec<u32>,
    /// Claude Code, запущенный отдельно: терминал, редактор.
    cli: Vec<u32>,
}

/// Все процессы с путём, владельцем и командной строкой.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn process_list() -> sysinfo::System {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_user(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    sys
}

/// Только процессы этого пользователя: чужие сеансы (удалённый рабочий
/// стол, другой вход в Windows) не трогаем и не считаем.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn mine(sys: &sysinfo::System) -> impl Fn(&sysinfo::Process) -> bool + '_ {
    let me = sysinfo::get_current_pid().ok().and_then(|pid| sys.process(pid)).and_then(|p| p.user_id().cloned());
    move |p| me.as_ref().map_or(true, |me| p.user_id() == Some(me))
}

/// Claude Code из npm: node/bun, запущенный с cli.js пакета @anthropic-ai/claude-code.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn is_npm_cli(p: &sysinfo::Process) -> bool {
    let name = p.name().to_string_lossy().to_lowercase();
    ["node", "node.exe", "bun", "bun.exe"].contains(&name.as_str())
        && p.cmd().iter().any(|a| a.to_string_lossy().to_lowercase().replace('\\', "/").contains("@anthropic-ai/claude-code"))
}

#[cfg(target_os = "windows")]
fn processes(env: &Env) -> Procs {
    let desktop_exe = crate::claude_desktop::detect().map(|i| i.exe_path.to_lowercase());
    let data: Vec<String> = env.desktop_dirs.iter().map(|d| d.to_string_lossy().to_lowercase()).collect();
    let sys = process_list();
    let mine = mine(&sys);
    let mut out = Procs::default();
    for (pid, p) in sys.processes() {
        if !mine(p) {
            continue;
        }
        let name = p.name().to_string_lossy().to_lowercase();
        let exe = p.exe().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let from_data = !exe.is_empty() && data.iter().any(|d| exe.starts_with(d.as_str()));
        if name == "claude.exe" {
            // Store: …\WindowsApps\AnthropicPBC.Claude_…\app\claude.exe;
            // установщик: %LOCALAPPDATA%\AnthropicClaude\app-x.y.z\claude.exe.
            let is_desktop = !from_data
                && (desktop_exe.as_deref().is_some_and(|d| d == exe)
                    || exe.contains("\\windowsapps\\anthropicpbc.claude_")
                    || exe.contains("\\anthropicclaude\\"));
            if is_desktop {
                out.desktop.push(pid.as_u32());
            } else if from_data {
                out.helpers.push(pid.as_u32());
            } else {
                out.cli.push(pid.as_u32());
            }
        } else if from_data
            || (name == "chrome-native-host.exe" && (exe.contains("anthropic") || exe.contains("claude")))
        {
            out.helpers.push(pid.as_u32());
        } else if is_npm_cli(p) {
            out.cli.push(pid.as_u32());
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn processes(_env: &Env) -> Procs {
    let sys = process_list();
    let mine = mine(&sys);
    let mut out = Procs::default();
    for (pid, p) in sys.processes() {
        if !mine(p) {
            continue;
        }
        let exe = p.exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        if exe.contains("/Claude.app/Contents/") {
            out.desktop.push(pid.as_u32());
        } else if exe.contains("/Application Support/Claude/") {
            out.helpers.push(pid.as_u32());
        } else if p.name().to_string_lossy() == "claude" || is_npm_cli(p) {
            out.cli.push(pid.as_u32());
        }
    }
    out
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn processes(_env: &Env) -> Procs {
    Procs::default()
}

fn kill(pids: &[u32]) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    if pids.is_empty() {
        return;
    }
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    for pid in pids {
        if let Some(p) = sys.process(Pid::from_u32(*pid)) {
            p.kill();
        }
    }
}

/// Ждать, пока `left` не вернёт пусто.
fn wait_until(secs: u64, left: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while left() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    !left()
}

/// Попросить Claude Desktop закрыться, как перед обновлением системы.
#[cfg(target_os = "windows")]
fn ask_desktop_to_quit(pids: &[u32]) {
    let _ = crate::win::end_session(pids);
}
#[cfg(target_os = "macos")]
fn ask_desktop_to_quit(_pids: &[u32]) {
    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"Claude\" to quit"])
        .status();
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn ask_desktop_to_quit(_pids: &[u32]) {}

/// Закрыть всё, что пишет в данные Claude: Desktop (сначала вежливо —
/// он сохраняет состояние и выходит; не вышел — завершаем), его Claude Code,
/// а с `with_cli` — и отдельно запущенный Claude Code. Иначе работающая
/// программа запишет аккаунт и id обратно сразу после очистки.
/// Ok(true) — Claude Desktop был запущен.
fn close_claude(env: &Env, with_cli: bool) -> Result<bool, String> {
    let first = processes(env);
    let was_running = !first.desktop.is_empty();
    if was_running {
        ask_desktop_to_quit(&first.desktop);
        if !wait_until(8, || !processes(env).desktop.is_empty()) {
            kill(&processes(env).desktop);
            if !wait_until(5, || !processes(env).desktop.is_empty()) {
                return Err(
                    "Claude Desktop не закрылся — закройте его вручную (в трее тоже) и нажмите «Очистить» снова".into()
                );
            }
        }
    }
    let rest = |p: &Procs| {
        let mut v = p.helpers.clone();
        if with_cli {
            v.extend(&p.cli);
        }
        v
    };
    let left = rest(&processes(env));
    if !left.is_empty() {
        kill(&left);
        if !wait_until(6, || !rest(&processes(env)).is_empty()) {
            return Err("Claude Code не закрылся — закройте его в терминале или редакторе и нажмите «Очистить» снова".into());
        }
    }
    // Процесс вышел, но система ещё отпускает его файлы.
    std::thread::sleep(Duration::from_millis(400));
    Ok(was_running)
}

// ─── Для команд ─────────────────────────────────────────────────────

pub fn scan() -> Scan {
    let env = env();
    let mut s = scan_env(&env);
    let r = running_env(&env);
    s.desktop_running = r.desktop_running;
    s.cli_running = r.cli_running;
    s
}

fn running_env(env: &Env) -> Running {
    let p = processes(env);
    Running { desktop_running: !p.desktop.is_empty(), cli_running: p.cli.len() as u32 }
}

/// Только запущенные процессы — для частого опроса (скан папок дорогой).
pub fn running() -> Running {
    running_env(&env())
}

pub fn wipe(
    include_desktop: bool,
    include_code: bool,
    include_chats: bool,
    export_chats: bool,
) -> Result<WipeResult, String> {
    let env = env();
    if !include_desktop && !include_code && !include_chats {
        return Err("Не выбрано, что очищать".into());
    }
    if include_desktop && env.desktop_dirs.is_empty() && !include_code && !include_chats {
        return Err("Данные Claude Desktop на этом компьютере не найдены".into());
    }
    let export = include_chats && export_chats && crate::claude_chats_export::has_chats(&env);
    // До закрытия Claude: архив уже сохраняется отдельной кнопкой — ждём его.
    if export && crate::claude_chats_export::progress().active {
        return Err("Сейчас сохраняется архив чатов — дождитесь его и нажмите «Очистить» снова".into());
    }
    // Desktop закрываем всегда: его вкладка Code пишет в данные Claude Code.
    let closed = close_claude(&env, include_code || include_chats)?;
    tracing::info!(
        "очистка данных Claude (Desktop: {include_desktop}, Code: {include_code}, чаты: {include_chats}, архив: {export})"
    );
    // Сначала архив: не сохранился или вошёл не целиком — ничего не удаляем.
    let archive = if export {
        let a = crate::claude_chats_export::export_env(&env)
            .map_err(|e| format!("Архив чатов не сохранился, ничего не удалено: {e}"))?;
        if a.skipped > 0 {
            return Err(format!(
                "В архив не вошло файлов: {} (их держит другая программа) — ничего не удалено. \
                 Архив сохранён в «Загрузки», список — в README.md внутри. Закройте программы, \
                 которые могут держать файлы Claude, и очистите ещё раз.",
                a.skipped
            ));
        }
        Some(a)
    } else {
        None
    };
    let mut res = wipe_env(&env, include_desktop, include_code, include_chats);
    res.closed_desktop = closed;
    res.archive = archive;
    forget_os_stores();
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
    const OLD_ACC: &str = "a23591e5-0000-4000-8000-000000000001";
    const OLD_ORG: &str = "881d1f4b-0000-4000-8000-000000000002";

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn test_env(root: &Path) -> (Env, PathBuf, PathBuf, PathBuf) {
        let d = root.join("Claude-Data").join("anthropicpbc.claude_x");
        let home = root.join("home");
        let tmp = root.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let env = Env {
            desktop_dirs: vec![d.clone()],
            desktop_extra: vec![],
            code_dir: Some(home.join(".claude")),
            code_json: Some(home.join(".claude.json")),
            code_json_dir: Some(home.clone()),
            code_extra: vec![],
            code_json_extra: vec![],
            temp_dir: Some(tmp.clone()),
            keychain: false,
            os_stores: false,
        };
        (env, d, home, tmp)
    }

    #[test]
    fn wipes_account_traces_and_keeps_sessions() {
        let root = std::env::temp_dir().join(format!("clod-wipe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (env, d, home, tmp) = test_env(&root);

        // Claude Desktop.
        write(&d.join("Network").join("Cookies"), "cookies");
        write(&d.join("Local Storage").join("leveldb").join("000001.log"), "ls");
        write(&d.join("IndexedDB").join("x").join("y"), "idb");
        write(&d.join("Cache").join("Cache_Data").join("f"), "0123456789");
        write(&d.join("ant-did"), "device");
        write(&d.join("ant-device-key-generation.json"), "{}");
        write(&d.join("host-creds-abc.json"), "{}");
        write(&d.join("Partitions").join("browser").join("Network").join("Cookies"), "c");
        write(&d.join("Local State"), r#"{"os_crypt":{"encrypted_key":"k"},"uninstall_metrics":{"installation_date2":"1"}}"#);
        write(&d.join("sentry").join("scope_v3.json"), "{}");
        write(&d.join("logs").join("main.log"), "user@example.com");
        write(&d.join("plugin-list").join(ACC).join(ORG).join("p.json"), "{}");
        write(&d.join("cowork-memory-resets.json"), &format!(r#"{{"{ACC}:{ORG}":1}}"#));
        write(&d.join("extensions-blocklist.json"), "{}");
        write(&d.join("ccd-session-presets.json"), &format!(r#"{{"byAccount":{{"{ACC}":{{"x":1}}}},"shared":1}}"#));
        write(
            &d.join("config.json"),
            &format!(
                r#"{{"locale":"ru-RU","oauth:tokenCacheV2":"secret","lastKnownAccountUuid":"{ACC}","coworkTrustedDeviceToken:{ACC}":"t","dxt:allowlistEnabled:{ORG}":true,"bootFrameLayout":{{"sidebarWidth":300}}}}"#
            ),
        );
        write(
            &d.join("claude_desktop_config.json"),
            &format!(
                r#"{{"mcpServers":{{"fs":{{"command":"npx"}}}},"preferences":{{"sidebarMode":"default","remoteToolsDeviceName":"MYPC","chromeExtension":{{"pairedDeviceId":"dev1"}},"bypassPermissionsGateByAccount":{{"{ACC}":true}},"coworkModelAutoFallbackByAccount":{{"{OLD_ACC}":true}},"epitaxyPrefs":{{"x.{ACC}":true,"dframe-group-scopes":{{"{ACC}/{ORG}":1}},"modelSwitchCacheWarningSkip.{OLD_ACC}":1,"keep":1}}}}}}"#
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
                r#"{{"numStartups":3,"userID":"u","machineID":"m","remoteControlMachineId":"r","summonSidKey":"s","oauthAccount":{{"accountUuid":"{ACC}","emailAddress":"a@b"}},"cachedGrowthBookFeatures":{{}},"hasShownOpus46Notice":{{"{OLD_ORG}":true}},"groveChoiceSeenAt":{{"{ACC}":1}},"projects":{{"C:/p":{{"allowedTools":[],"lastSessionId":"x"}},"C:/scratch-workspaces/{ACC}/{ORG}/w":{{}}}},"mcpServers":{{"m":{{"command":"x"}}}},"hasCompletedOnboarding":true}}"#
            ),
        );
        write(&home.join(".claude.json.backup"), "old");
        write(
            &home.join(".claude.json.backup.1778174702734"),
            &format!(r#"{{"oauthAccount":{{"accountUuid":"{OLD_ACC}","organizationUuid":"{OLD_ORG}"}}}}"#),
        );
        write(&home.join(".claude-custom-oauth.json"), r#"{"userID":"u2","theme":"dark"}"#);
        write(&code_dir.join(".credentials.json"), "{}");
        write(&code_dir.join(".device-keys.json"), "{}");
        write(&code_dir.join("telemetry").join("e.json"), "{}");
        write(&code_dir.join("backups").join(".claude.json.backup.1"), "{}");
        write(&code_dir.join("cache").join("model-catalog").join(format!("{ORG}-abc-ccd.json")), "{}");
        write(&code_dir.join("cache").join("changelog.md"), "x");
        write(&code_dir.join("policy-limits.signature.json"), "{}");
        write(&code_dir.join("stats-cache.json"), "{}");
        let transcript = code_dir.join("projects").join("p").join("s.jsonl");
        write(&transcript, "chat");
        write(&code_dir.join("history.jsonl"), "prompt");
        write(&code_dir.join("settings.json"), "{}");
        write(&tmp.join("claude").join("h").join("task.output"), "x");
        write(&tmp.join("claude-1a2b-cwd"), "x");
        write(&tmp.join("other.txt"), "x");

        let s = scan_env(&env);
        assert!(s.groups.iter().any(|g| g.id == Group::Login && !g.code));
        assert!(s.groups.iter().any(|g| g.id == Group::Cache && g.bytes >= 10));
        assert!(s.groups.iter().any(|g| g.code));
        assert_eq!(s.kept_sessions, 1);

        // Только Desktop: Claude Code не тронут.
        let r = wipe_env(&env, true, false, false);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        for gone in [
            "Network", "Local Storage", "IndexedDB", "Cache", "ant-did", "sentry", "logs",
            "ant-device-key-generation.json", "host-creds-abc.json", "Partitions", "plugin-list",
            "cowork-memory-resets.json", "extensions-blocklist.json",
        ] {
            assert!(!d.join(gone).exists(), "{gone} должен быть удалён");
        }
        let cfg = read_json(&d.join("config.json")).unwrap();
        assert_eq!(cfg.keys().cloned().collect::<Vec<_>>(), vec!["bootFrameLayout", "locale"]);
        let prefs = std::fs::read_to_string(d.join("claude_desktop_config.json")).unwrap();
        assert!(!prefs.contains(ACC) && !prefs.contains(ORG) && !prefs.contains(OLD_ACC), "записи под аккаунты убраны: {prefs}");
        assert!(!prefs.contains("MYPC") && !prefs.contains("pairedDeviceId"), "устройство убрано");
        assert!(prefs.contains("mcpServers") && prefs.contains("sidebarMode") && prefs.contains("\"keep\""));
        let presets = std::fs::read_to_string(d.join("ccd-session-presets.json")).unwrap();
        assert!(!presets.contains(ACC) && presets.contains("shared"));
        let ls = std::fs::read_to_string(d.join("Local State")).unwrap();
        assert!(ls.contains("os_crypt") && !ls.contains("uninstall_metrics"));
        assert!(sess.exists() && cowork.exists(), "сессии остаются");
        assert!(d.join("window-state.json").exists() && d.join("claude-code").exists());
        assert!(code_dir.join(".credentials.json").exists(), "Claude Code не тронут");

        // Вместе с Claude Code.
        let r = wipe_env(&env, true, true, false);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        let cj = read_json(&code_json).unwrap();
        for k in [
            "userID", "machineID", "remoteControlMachineId", "summonSidKey", "oauthAccount",
            "cachedGrowthBookFeatures", "hasShownOpus46Notice", "groveChoiceSeenAt", "numStartups",
        ] {
            assert!(!cj.contains_key(k), "{k} убран");
        }
        let text = std::fs::read_to_string(&code_json).unwrap();
        assert!(!text.contains(ACC) && !text.contains(OLD_ORG) && !text.contains("lastSessionId"), "{text}");
        assert!(cj.contains_key("projects") && cj.contains_key("mcpServers") && cj.contains_key("hasCompletedOnboarding"));
        assert!(text.contains("C:/p"), "обычные проекты остаются");
        let custom = read_json(&home.join(".claude-custom-oauth.json")).unwrap();
        assert!(!custom.contains_key("userID") && custom.contains_key("theme"));
        assert!(!home.join(".claude.json.backup").exists());
        assert!(!home.join(".claude.json.backup.1778174702734").exists());
        for gone in [".credentials.json", ".device-keys.json", "telemetry", "cache/model-catalog", "policy-limits.signature.json", "stats-cache.json"] {
            assert!(!code_dir.join(gone).exists(), "{gone} должен быть удалён");
        }
        assert!(code_dir.join("cache").join("changelog.md").exists());
        assert!(!code_dir.join("backups").join(".claude.json.backup.1").exists());
        assert!(!tmp.join("claude").exists() && !tmp.join("claude-1a2b-cwd").exists());
        assert!(tmp.join("other.txt").exists(), "чужие временные файлы не трогаем");
        assert!(transcript.exists() && code_dir.join("settings.json").exists(), "чаты Code остаются");

        // Повторная очистка — ничего не ломает и ничего не находит.
        let again = scan_env(&env);
        assert!(again.groups.iter().all(|g| g.id == Group::Chats), "остались только чаты: {:?}", again.groups);
        assert_eq!(again.kept_sessions, 1);

        // Отдельно — локальные чаты: удаляются только по своему флагу.
        let r = wipe_env(&env, false, false, true);
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert!(!sess.exists() && !cowork.exists(), "чаты Desktop удалены");
        assert!(!transcript.exists() && !code_dir.join("history.jsonl").exists(), "переписка Code удалена");
        assert!(code_dir.join("settings.json").exists(), "настройки Claude Code остаются");
        assert!(d.join("window-state.json").exists() && d.join("claude-code").exists());
        assert!(scan_env(&env).groups.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_json_is_reported_not_skipped() {
        let root = std::env::temp_dir().join(format!("clod-wipe-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (env, d, home, _) = test_env(&root);
        write(&d.join("config.json"), r#"{"oauth:tokenCacheV2":"secret", "#);
        write(&d.join("claude_desktop_config.json"), "{");
        write(&home.join(".claude.json"), "not json");

        let s = scan_env(&env);
        assert!(s.groups.iter().any(|g| g.id == Group::Login && !g.code), "битый config.json виден в скане");
        let r = wipe_env(&env, true, true, false);
        assert!(!d.join("config.json").exists(), "битый config.json удалён целиком");
        assert_eq!(r.failed.len(), 2, "{:?}", r.failed);
        assert!(r.failed.iter().any(|f| f.contains(".claude.json")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uuid_detection() {
        assert!(is_uuid(ACC));
        assert!(!is_uuid("local_21bfa578-1aae-41f1-87f2-b57c42b5d8d2"));
        assert!(!is_uuid("skills-plugin"));
        assert_eq!(uuids_in(&format!("x.{ACC}/{ORG}")), vec![ACC.to_string(), ORG.to_string()]);
        assert!(has_uuid(&format!("local_{ACC}")));
        assert!(!has_uuid("C:/Users/Иван/проект"));
    }

    /// Настоящий TPM и Диспетчер учётных данных: создаём тестовые записи
    /// с префиксами Claude, находим их и удаляем. Ключи Claude не трогаем.
    #[test]
    #[cfg(target_os = "windows")]
    #[ignore = "пишет в TPM и Диспетчер учётных данных этого ПК (и сразу удаляет)"]
    fn os_stores_roundtrip() {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        use windows_sys::Win32::Security::Cryptography::{
            NCryptCreatePersistedKey, NCryptFinalizeKey, NCryptFreeObject, NCryptOpenStorageProvider,
            BCRYPT_ECDSA_P256_ALGORITHM, MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_KEY_HANDLE, NCRYPT_PROV_HANDLE,
        };
        let key = "Claude-HwKey-clod-helper-selftest";
        unsafe {
            let mut prov: NCRYPT_PROV_HANDLE = 0;
            assert_eq!(NCryptOpenStorageProvider(&mut prov, MS_PLATFORM_CRYPTO_PROVIDER, 0), 0);
            let mut h: NCRYPT_KEY_HANDLE = 0;
            let w = wide(key);
            let r = NCryptCreatePersistedKey(prov, &mut h, BCRYPT_ECDSA_P256_ALGORITHM, w.as_ptr(), 0, 0);
            assert!(r == 0 || r as u32 == 0x8009_000F, "create {r:x}");
            if r == 0 {
                assert_eq!(NCryptFinalizeKey(h, 0), 0);
            }
            NCryptFreeObject(h);
            NCryptFreeObject(prov);
        }
        assert!(tpm_keys().iter().any(|k| k == key), "ключ найден");
        let d = tpm_delete(key);
        assert!(d.is_ok(), "{d:?}");
        assert!(!tpm_keys().iter().any(|k| k == key), "ключ удалён");

        let target = "Claude Code-clod-helper-selftest";
        unsafe {
            let t = wide(target);
            let mut blob = *b"x";
            let mut c: CREDENTIALW = std::mem::zeroed();
            c.Type = CRED_TYPE_GENERIC;
            c.TargetName = t.as_ptr() as *mut u16;
            c.CredentialBlobSize = 1;
            c.CredentialBlob = blob.as_mut_ptr();
            c.Persist = CRED_PERSIST_LOCAL_MACHINE;
            assert_ne!(CredWriteW(&c, 0), 0);
        }
        assert!(credential_targets().iter().any(|k| k == target), "запись найдена");
        credential_delete(target).unwrap();
        assert!(!credential_targets().iter().any(|k| k == target), "запись удалена");
    }

    /// Архив настоящих чатов во временную папку: время, размер, что не прочиталось.
    #[test]
    #[ignore = "читает настоящие чаты Claude на этом ПК; архив кладёт во временную папку и удаляет"]
    fn export_real_machine() {
        let out = std::env::temp_dir().join(format!("clod-export-real-{}", std::process::id()));
        let started = Instant::now();
        let _serial = crate::claude_chats_export::tests::serial();
        let a = crate::claude_chats_export::export_to(&env(), &out).unwrap();
        eprintln!("{:?} за {:?}", a, started.elapsed());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    #[ignore = "только чтение: смотрит настоящие данные Claude на этом ПК"]
    fn scan_real_machine() {
        let s = scan();
        eprintln!("{s:#?}");
    }
}
