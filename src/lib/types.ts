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

export interface WizardState {
  step: WizardStep;
  mode: InstallMode | null;
  cursor: boolean;
  proxy: ProxyConfig | null;
  proxy_check: ProxyCheckResult | null;
  node: NodeInfo | null;
  python: PythonInfo | null;
  install_log: string[];
  install_done: boolean;
  install_error: string | null;
}

export type WizardStep =
  | "welcome"
  | "mode"
  | "proxy"
  | "deps"
  | "install"
  | "done";

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

export type AppView = "dashboard" | "settings" | "wizard";

// ─── Persistent settings (mirror of Rust Settings) ───────────────

export interface AppSettings {
  proxy_url?: string;
  autostart_dashboard: boolean;
  app_profiles?: AppProfile[];
  killswitch_enabled?: boolean;
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

// ─── Killswitch ──────────────────────────────────────────────────

export interface KillswitchStatus {
  active: boolean;
  rule_count: number;
  blocked_processes: string[];
}

// ─── Global proxy env ────────────────────────────────────────────

export interface GlobalProxyEnv {
  present: boolean;
  /** [name, value] пары. */
  values: [string, string][];
}
