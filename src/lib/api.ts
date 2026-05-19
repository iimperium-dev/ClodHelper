import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  NodeInfo,
  ProxyCheckResult,
  ProxyConfig,
  PythonInfo,
  ShimStatus,
  ShimTestResult,
} from "./types";

export async function detectNode(): Promise<NodeInfo> {
  return invoke<NodeInfo>("detect_node");
}

export async function detectPython(): Promise<PythonInfo> {
  return invoke<PythonInfo>("detect_python");
}

export async function parseProxy(url: string): Promise<ProxyConfig> {
  return invoke<ProxyConfig>("parse_proxy", { url });
}

export async function checkProxy(url: string): Promise<ProxyCheckResult> {
  return invoke<ProxyCheckResult>("check_proxy", { url });
}

export async function runInstall(params: {
  mode: "code" | "desktop" | "both";
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
