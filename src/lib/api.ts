import { invoke } from "@tauri-apps/api/core";
import type {
  AppProfile,
  AppProfileStatus,
  AppSettings,
  BridgeHealth,
  BridgeTaskStatus,
  CursorInfo,
  CursorSetupResult,
  GlobalProxyEnv,
  KillswitchStatus,
  NodeInfo,
  ProxyCheckResult,
  ProxyConfig,
  PythonInfo,
  ShimStatus,
  ShimTestResult,
  ShortcutTarget,
} from "./types";

export async function detectNode(): Promise<NodeInfo> {
  return invoke<NodeInfo>("detect_node");
}

export async function detectPython(): Promise<PythonInfo> {
  return invoke<PythonInfo>("detect_python");
}

// ─── Cursor IDE ──────────────────────────────────────────────────

export async function detectCursor(): Promise<CursorInfo> {
  return invoke<CursorInfo>("detect_cursor");
}

export async function setupCursor(): Promise<CursorSetupResult> {
  return invoke<CursorSetupResult>("setup_cursor");
}

export async function launchCursor(): Promise<void> {
  return invoke<void>("launch_cursor");
}

export async function parseProxy(url: string): Promise<ProxyConfig> {
  return invoke<ProxyConfig>("parse_proxy", { url });
}

export async function checkProxy(url: string): Promise<ProxyCheckResult> {
  return invoke<ProxyCheckResult>("check_proxy", { url });
}

export async function runInstall(params: {
  mode: "code" | "desktop" | "both" | null;
  cursor: boolean;
  proxyUrl: string;
}): Promise<void> {
  return invoke<void>("run_install", params);
}

// ─── Shim control ────────────────────────────────────────────────

export async function shimStatus(): Promise<ShimStatus> {
  return invoke<ShimStatus>("shim_status");
}

export async function shimStart(): Promise<void> {
  return invoke<void>("shim_start");
}

export async function shimStop(): Promise<number> {
  return invoke<number>("shim_stop");
}

export async function shimRestart(): Promise<void> {
  return invoke<void>("shim_restart");
}

export async function shimTest(): Promise<ShimTestResult> {
  return invoke<ShimTestResult>("shim_test");
}

export async function shimScriptPath(): Promise<string | null> {
  return invoke<string | null>("shim_script_path");
}

// ─── Settings ────────────────────────────────────────────────────

export async function settingsGet(): Promise<AppSettings> {
  return invoke<AppSettings>("settings_get");
}

export async function settingsSetProxy(url: string): Promise<AppSettings> {
  return invoke<AppSettings>("settings_set_proxy", { url });
}

// ─── App profiles ────────────────────────────────────────────────

export async function appsList(): Promise<AppProfileStatus[]> {
  return invoke<AppProfileStatus[]>("apps_list");
}

export async function appsSet(
  profiles: AppProfile[],
): Promise<AppProfileStatus[]> {
  return invoke<AppProfileStatus[]>("apps_set", { profiles });
}

export async function appsAddCustom(
  name: string,
  exePath: string,
): Promise<AppProfileStatus[]> {
  return invoke<AppProfileStatus[]>("apps_add_custom", { name, exePath });
}

export async function appsRemove(id: string): Promise<AppProfileStatus[]> {
  return invoke<AppProfileStatus[]>("apps_remove", { id });
}

export async function appsLaunch(id: string): Promise<void> {
  return invoke<void>("apps_launch", { id });
}

export async function appsCreateShortcut(
  id: string,
  target: ShortcutTarget,
): Promise<string> {
  return invoke<string>("apps_create_shortcut", { id, target });
}

// ─── Killswitch ──────────────────────────────────────────────────

export async function killswitchStatus(): Promise<KillswitchStatus> {
  return invoke<KillswitchStatus>("killswitch_status");
}

export async function killswitchEnable(): Promise<KillswitchStatus> {
  return invoke<KillswitchStatus>("killswitch_enable");
}

export async function killswitchDisable(): Promise<KillswitchStatus> {
  return invoke<KillswitchStatus>("killswitch_disable");
}

// ─── Global proxy env (главный выключатель) ──────────────────────

export async function globalProxyEnv(): Promise<GlobalProxyEnv> {
  return invoke<GlobalProxyEnv>("global_proxy_env");
}

/** Включить системное проксирование: HTTP(S)_PROXY → мост (User scope). */
export async function setGlobalProxyEnv(): Promise<GlobalProxyEnv> {
  return invoke<GlobalProxyEnv>("set_global_proxy_env");
}

export async function clearGlobalProxyEnv(): Promise<GlobalProxyEnv> {
  return invoke<GlobalProxyEnv>("clear_global_proxy_env");
}

// ─── Автозапуск моста (Планировщик задач) ────────────────────────

export async function bridgeTaskStatus(): Promise<BridgeTaskStatus> {
  return invoke<BridgeTaskStatus>("bridge_task_status");
}

export async function bridgeTaskRegister(): Promise<BridgeTaskStatus> {
  return invoke<BridgeTaskStatus>("bridge_task_register");
}

// ─── Мост: /healthz и маршрутизация ──────────────────────────────

/** Живые счётчики моста. Кидает, если мост не отвечает или старый (python). */
export async function bridgeHealth(): Promise<BridgeHealth> {
  return invoke<BridgeHealth>("bridge_health");
}

export async function bridgeSetRouteMode(mode: "smart" | "all"): Promise<void> {
  return invoke<void>("bridge_set_route_mode", { mode });
}

/** Обновить установленный мост из ресурсов приложения (stop → copy → start). */
export async function bridgeUpdate(): Promise<string> {
  return invoke<string>("bridge_update");
}
