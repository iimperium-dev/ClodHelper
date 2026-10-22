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

export type AppView = "overview" | "proxies" | "apps" | "sites" | "settings" | "wizard";

// ─── Сайты через прокси (mirror of Rust SiteEntry / commands::sites) ──

export interface SiteEntry {
  id: string;
  name: string;
  /** Домены сайта — вместе с поддоменами. */
  domains: string[];
  /** Индекс прокси в пуле; null — «Авто». */
  proxy: number | null;
  enabled: boolean;
}

export interface SitesBrowser {
  id: string;
  name: string;
  installed: boolean;
  /** "sites" — только AI и сайты; "all" — весь трафик; "ai" — старый мост. */
  mode: "sites" | "all" | "ai";
}

export interface SitesStatus {
  /** Мост понимает сайты (≥1.9). */
  supported: boolean;
  browsers: SitesBrowser[];
}

/** Браузер, в котором расширения перехватывают прокси (сайты в нём не работают). */
export interface BrowserProxyExtensions {
  name: string;
  /** Путь к exe/.app — для «Открыть расширение». */
  path: string;
  extensions: { id: string; name: string }[];
}

/** «Сайты во всех браузерах» — системная автонастройка прокси (mirror system_pac.rs). */
export interface SystemPacStatus {
  /** Есть в этой ОС (пока только Windows). */
  supported: boolean;
  /** Включено пользователем. */
  enabled: boolean;
  /** В системе сейчас стоит наш PAC (enabled && !active — сбросила другая программа). */
  active: boolean;
  /** Чужая настройка прокси в системе. */
  foreign: string | null;
}

export interface SeenHost {
  host: string;
  count: number;
  first_ms: number;
  last_ms: number;
  via: "proxy" | "direct";
  site: string | null;
}

export interface SeenHosts {
  now_ms: number;
  hosts: SeenHost[];
}

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
  /** Убрано пользователем из списка. */
  hidden?: boolean;
}

/** Runtime-статус профиля (mirror of Rust AppProfileStatus). */
export interface AppProfileStatus {
  id: string;
  name: string;
  kind: LaunchKind;
  enabled: boolean;
  builtin: boolean;
  /** Убрано из списка (встроенные прячутся, а не удаляются). */
  hidden: boolean;
  /** Свой прокси из пула (индекс); null — «Авто». */
  proxy: number | null;
  /** Весь трафик через прокси (иначе только AI-сайты). */
  full_proxy: boolean;
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

/** Найденная установленная программа (поиск при добавлении). */
export interface FoundApp {
  name: string;
  path: string;
  /** Иконка (data-URL PNG) сразу с ответом — Windows. */
  icon?: string;
}

/** Живые соединения приложения: через мост / мимо моста. */
export interface AppTraffic {
  id: string;
  running: boolean;
  via_bridge: number;
  direct: number;
  /** Браузер: включённое расширение, перехватившее прокси. */
  proxy_extension: { id: string; name: string } | null;
  /** Telegram: чужой локальный прокси («127.0.0.1:1442»), мимо моста. */
  other_proxy?: string | null;
}

/** Telegram через SOCKS-вход моста. */
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

/** Внешний IP с подписью (страна, сеть). */
export interface IpInfo {
  ip: string;
  country: string;
  org: string;
}

/** «Проверка приватности»: что видят сервисы и есть ли утечки. */
export interface PrivacyReport {
  exit: IpInfo | null;
  home: IpInfo | null;
  dns: IpInfo[];
  ip_hidden: boolean;
  dns_leak: boolean;
  leaking_apps: string[];
  error: string | null;
}

/** «Всегда через мост» у Chromium-браузера (политика macOS / Windows). */
export interface BrowserPolicyStatus {
  configured: boolean;
  managed: boolean;
  conflict: string | null;
  auto_restore: boolean;
  server: string;
  notice: string | null;
}

/** Новая версия Clod Helper на GitHub. */
export interface UpdateInfo {
  version: string;
  notes: string;
  /** Установщик для этой ОС (.dmg или -setup.exe). */
  url: string;
  sums_url: string;
  /** Пункты «Что нового» из описания релиза. */
  highlights?: string[];
}

/** Ход установки обновления (событие "update:progress"). */
export interface UpdateProgress {
  stage: "download" | "verify" | "install" | "restart";
  received: number;
  /** 0 — размер неизвестен. */
  total: number;
}

/** «Защита от утечек» (pf на macOS, брандмауэр Windows). */
export interface LeakGuardStatus {
  enabled: boolean;
  /** "claude", "telegram" или "claude,telegram". */
  services: string;
  auto_restore: boolean;
  /** Живая проверка: прямое соединение с Anthropic отклонено. null — не понять. */
  verified: boolean | null;
}
