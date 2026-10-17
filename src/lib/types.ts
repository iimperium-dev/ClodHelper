export type InstallMode = "code" | "desktop" | "both";

export interface ProxyConfig {
  url: string;
  host: string;
  port: number;
  username: string;
  password: string;
  label: string;
}

export interface NodeInfo {
  installed: boolean;
  version?: string;
  npm_prefix?: string;
  prefix_in_program_files: boolean;
}

export interface PythonInfo {
  installed: boolean;
  version?: string;
  command?: string;
}

export interface CursorInfo {
  installed: boolean;
  exe_path?: string;
  shortcut_exists: boolean;
}

export interface CursorSetupResult {
  ok: boolean;
  found: boolean;
  message: string;
}

export interface ProxyCheckResult {
  reachable: boolean;
  latency_ms?: number;
  status_code?: number;
  error?: string;
  ip?: string;
  country_code?: string;
  country_name?: string;
  isp?: string;
}

/** Новый мастер: 3 шага (прокси → установка → готово). */
export type WizardStep = "proxy" | "install" | "done";

/** Результат wizard_install: честная проверка + состояние env. */
export interface WizardInstallResult {
  test: ShimTestResult;
  env_on: boolean;
}

// ─── Dashboard / Shim ────────────────────────────────────────────

export interface ShimStatus {
  running: boolean;
  pid?: number;
  uptime_sec?: number;
  listen?: string;
  /** URL с замаскированным паролем: http://user:****@host:port */
  upstream_masked?: string;
  upstream_host?: string;
  upstream_port?: number;
}

export interface ShimTestResult {
  ok: boolean;
  latency_ms?: number;
  error?: string;
  external_ip?: string;
}

// ─── App-level navigation ────────────────────────────────────────

export type AppView = "overview" | "proxies" | "apps" | "settings" | "wizard";

// ─── Persistent settings (mirror of Rust Settings) ───────────────

export interface AppSettings {
  proxy_url?: string;
  autostart_dashboard: boolean;
  app_profiles?: AppProfile[];
  killswitch_enabled?: boolean;
  /** "all" | "smart" (None = "all"). */
  route_mode?: string;
}

// ─── App profiles (проксируемые приложения) ──────────────────────

export type LaunchKind = "exe_flag" | "msix" | "custom";

export interface AppProfile {
  id: string;
  name: string;
  kind: LaunchKind;
  exe_path?: string;
  app_id?: string;
  process_names: string[];
  enabled: boolean;
  builtin: boolean;
}

/** Runtime-статус профиля (mirror of Rust AppProfileStatus). */
export interface AppProfileStatus {
  id: string;
  name: string;
  kind: LaunchKind;
  enabled: boolean;
  builtin: boolean;
  installed: boolean;
  location?: string;
  desktop_shortcut: boolean;
  process_names: string[];
}

export type ShortcutTarget = "desktop" | "start_menu";

export interface ClaudeProxyStatus {
  policy: {
    configured: boolean;
    managed: boolean;
    conflict: string | null;
    proxy_url: string | null;
    pac_configured: boolean;
    notice: string | null;
    /** macOS: служба возвращает настройку, если система её сотрёт. */
    auto_restore?: boolean;
  };
  bridge_ready: boolean;
  bridge_error: string | null;
  network_enforced: boolean;
}

// ─── Killswitch ──────────────────────────────────────────────────

export interface KillswitchStatus {
  active: boolean;
  rule_count: number;
  blocked_processes: string[];
}

// ─── Global proxy env (главный выключатель) ──────────────────────

export interface GlobalProxyEnv {
  present: boolean;
  /** Все значения указывают на наш мост (127.0.0.1:8889). */
  points_to_bridge: boolean;
  /** [name, value] пары. */
  values: [string, string][];
}

// ─── Автозапуск моста (Планировщик задач) ────────────────────────

export interface BridgeTaskStatus {
  registered: boolean;
  /** Ready / Running / Disabled. */
  state?: string;
}

// ─── Мост: /healthz и маршрутизация ──────────────────────────────

export interface BridgeUpstreamHealth {
  url: string;
  healthy: boolean;
  ok: number;
  fail: number;
  /** Отказов подряд (мост 1.5.0+). */
  fail_streak?: number;
}

/** Автозапуск приложения при входе в систему. */
export interface AutostartState {
  enabled: boolean;
  /** macOS: объект входа ждёт разрешения в Системных настройках. */
  needs_approval: boolean;
}

/** Живые соединения приложения: через мост / мимо моста (macOS). */
export interface AppTraffic {
  id: string;
  running: boolean;
  via_bridge: number;
  direct: number;
}

/** Telegram через SOCKS-вход моста (macOS). */
export interface TelegramStatus {
  supported: boolean;
  app_path: string | null;
  running: boolean;
  /** Мост слушает SOCKS 127.0.0.1:8891 (мост ≥1.6). */
  socks_ready: boolean;
  /** Соединений Telegram через мост сейчас. */
  via_bridge: number;
  /** Соединений Telegram наружу мимо моста. */
  direct: number;
}

/** Версии моста: установленного и встроенного в приложение. */
export interface BridgeVersions {
  installed: string | null;
  bundled: string | null;
  /** Встроенный новее установленного — пора «Обновить мост». */
  outdated: boolean;
}

/** Ошибка из кольцевого буфера моста: секунды-от-старта + текст. */
export interface BridgeErrorEntry {
  t: number;
  msg: string;
}

export interface BridgeHealth {
  status: string;
  version: string;
  uptime_sec: number;
  listen: string;
  /** "smart" — через upstream только AI-домены; "all" — весь трафик. */
  mode: string;
  active: number;
  total: number;
  via_upstream: number;
  via_direct: number;
  errors: number;
  upstreams: BridgeUpstreamHealth[];
  /** Последние ошибки (мост 1.1.0+; у старых может отсутствовать). */
  last_errors?: BridgeErrorEntry[];
}

// ─── Прокси-пул ──────────────────────────────────────────────────

/** TCP-пинг одного прокси из пула. ms=null — не подключились. */
export interface ProxyPing {
  url: string;
  host: string;
  port: number;
  ms: number | null;
}

/** Группы сервисов для назначений (mirror tasks::SERVICE_GROUPS). */
export type ServiceGroup = "anthropic" | "openai" | "google" | "other_ai" | "telegram";
