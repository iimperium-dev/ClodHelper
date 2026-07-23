import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import {
  Activity,
  CalendarClock,
  Copy,
  Globe,
  Loader2,
  Network,
  Pause,
  Play,
  Plus,
  RotateCw,
  Route,
  Settings as SettingsIcon,
  Wifi,
} from "lucide-react";
import { Button } from "../components/Button";
import { AppRow } from "../components/AppRow";
import { KillswitchCard } from "../components/KillswitchCard";
import { Toggle } from "../components/Toggle";
import {
  appsCreateShortcut,
  appsLaunch,
  appsList,
  appsRemove,
  appsSet,
  bridgeHealth,
  bridgeSetRouteMode,
  bridgeTaskRegister,
  bridgeTaskStatus,
  bridgeUpdate,
  clearGlobalProxyEnv,
  globalProxyEnv,
  killswitchDisable,
  killswitchEnable,
  killswitchStatus,
  setGlobalProxyEnv,
  shimRestart,
  shimStart,
  shimStatus,
  shimStop,
  shimTest,
} from "../lib/api";
import type {
  AppProfileStatus,
  BridgeHealth,
  BridgeTaskStatus,
  GlobalProxyEnv,
  KillswitchStatus,
  ShimStatus,
  ShimTestResult,
  ShortcutTarget,
} from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

const POLL_INTERVAL_MS = 5_000;

function formatUptime(sec?: number): string {
  if (!sec || sec < 0) return "—";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}ч ${m}м`;
  if (m > 0) return `${m}м ${s}с`;
  return `${s}с`;
}

interface Props {
  onOpenSettings: () => void;
}

export function Dashboard({ onOpenSettings }: Props) {
  const reduce = useReducedMotion();
  const [status, setStatus] = useState<ShimStatus | null>(null);
  const [apps, setApps] = useState<AppProfileStatus[]>([]);
  const [ks, setKs] = useState<KillswitchStatus | null>(null);
  const [globalEnv, setGlobalEnvState] = useState<GlobalProxyEnv | null>(null);
  const [envBusy, setEnvBusy] = useState(false);
  const [task, setTask] = useState<BridgeTaskStatus | null>(null);
  const [taskBusy, setTaskBusy] = useState(false);
  const [health, setHealth] = useState<BridgeHealth | null>(null);
  const [healthErr, setHealthErr] = useState<string | null>(null);
  const [routeBusy, setRouteBusy] = useState(false);
  const [updatingBridge, setUpdatingBridge] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [copied, setCopied] = useState(false);
  const intervalRef = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    const [s, a, k, e, t, h] = await Promise.all([
      shimStatus().catch(() => null),
      appsList().catch(() => [] as AppProfileStatus[]),
      killswitchStatus().catch(() => null),
      globalProxyEnv().catch(() => null),
      bridgeTaskStatus().catch(() => null),
      // Ошибку healthz СОХРАНЯЕМ: по её тексту различаем «мост лежит /
      // перезапускается» (не отвечает) и «старый python-мост» (ответ не
      // парсится) — см. bridge_health в commands.rs.
      bridgeHealth()
        .then((x) => ({ ok: x as BridgeHealth | null, err: null as string | null }))
        .catch((err) => ({ ok: null, err: String(err) })),
    ]);
    if (s) setStatus(s);
    setApps(a);
    if (k) setKs(k);
    if (e) setGlobalEnvState(e);
    if (t) setTask(t);
    setHealth(h.ok);
    setHealthErr(h.err);
  }, []);

  useEffect(() => {
    void refresh();
    intervalRef.current = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => {
      if (intervalRef.current) window.clearInterval(intervalRef.current);
    };
  }, [refresh]);

  const running = status?.running ?? false;
  const enabledCount = apps.filter((a) => a.enabled).length;
  const envOn = (globalEnv?.present ?? false) && (globalEnv?.points_to_bridge ?? false);
  const envForeign = (globalEnv?.present ?? false) && !(globalEnv?.points_to_bridge ?? false);

  // ─── Системное проксирование (главный выключатель) ───
  const handleEnvToggle = async () => {
    setEnvBusy(true);
    try {
      setGlobalEnvState(envOn ? await clearGlobalProxyEnv() : await setGlobalProxyEnv());
    } finally {
      setEnvBusy(false);
    }
  };

  const handleTaskRegister = async () => {
    setTaskBusy(true);
    try {
      setTask(await bridgeTaskRegister());
    } catch {
      await refresh();
    } finally {
      setTaskBusy(false);
    }
  };

  // ─── Маршрутизация (smart/all) ───
  const smartOn = health?.mode === "smart";
  // «Старый мост»: healthz ОТВЕТИЛ, но не JSON (python-мост пробросил запрос
  // upstream'у). «Не отвечает» = мост лежит/перезапускается — это НЕ повод
  // предлагать обновление.
  const oldBridge = !health && healthErr !== null && healthErr.includes("разобрать");

  const handleRouteToggle = async () => {
    setRouteBusy(true);
    try {
      await bridgeSetRouteMode(smartOn ? "all" : "smart");
      await refresh();
    } finally {
      setRouteBusy(false);
    }
  };

  const handleBridgeUpdate = async () => {
    setUpdatingBridge(true);
    setUpdateError(null);
    try {
      await bridgeUpdate();
      await refresh();
    } catch (e) {
      setUpdateError(String(e));
    } finally {
      setUpdatingBridge(false);
    }
  };

  // ─── Bridge controls ───
  const bridgeAction = async (
    kind: "start" | "stop" | "restart",
    fn: () => Promise<unknown>,
  ) => {
    setBusy(kind);
    try {
      await fn();
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const handleTest = async () => {
    setBusy("test");
    setTestResult(null);
    try {
      setTestResult(await shimTest());
    } finally {
      setBusy(null);
    }
  };

  const copyUpstream = async () => {
    if (!status?.upstream_masked) return;
    try {
      await navigator.clipboard.writeText(status.upstream_masked);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  };

  // ─── App controls ───
  const toggleApp = async (id: string, enabled: boolean) => {
    // Оптимистично обновляем UI, затем шлём полное состояние на бэкенд.
    const next = apps.map((a) => (a.id === id ? { ...a, enabled } : a));
    setApps(next);
    try {
      const saved = await appsSet(
        next.map((a) => ({
          id: a.id,
          name: a.name,
          kind: a.kind,
          exe_path: a.location && a.kind !== "msix" ? a.location : undefined,
          app_id: a.kind === "msix" ? a.location : undefined,
          process_names: a.process_names,
          enabled: a.enabled,
          builtin: a.builtin,
        })),
      );
      setApps(saved);
    } catch {
      await refresh();
    }
  };

  const removeApp = async (id: string) => {
    try {
      setApps(await appsRemove(id));
    } catch {
      await refresh();
    }
  };

  const launchApp = async (id: string) => {
    await appsLaunch(id);
  };

  const shortcutApp = async (id: string, target: ShortcutTarget) => {
    return appsCreateShortcut(id, target);
  };

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-5 p-7">
        {/* Header */}
        <header className="flex items-center justify-between">
          <div>
            <h1 className="text-[22px] font-bold tracking-tight text-vb-fg">
              Clod Helper
            </h1>
            <p className="mt-0.5 text-[13px] text-vb-silver-dim">
              Системный прокси, умный мост и killswitch
            </p>
          </div>
          <Button variant="ghost" size="sm" onClick={onOpenSettings}>
            <SettingsIcon className="h-4 w-4" />
            Настройки
          </Button>
        </header>

        {/* Bridge status */}
        <section className="glass-card p-5">
          <div className="flex items-center gap-3">
            <span className="relative inline-flex h-2.5 w-2.5">
              <span
                className={cn(
                  "absolute inset-0 rounded-full",
                  running
                    ? "bg-vb-emerald shadow-[0_0_12px_var(--color-vb-emerald-glow)]"
                    : "bg-vb-loss shadow-[0_0_8px_rgba(239,68,68,0.5)]",
                )}
              />
              {running && !reduce && (
                <motion.span
                  className="absolute -inset-1 rounded-full bg-vb-emerald/40"
                  animate={{ scale: [1, 1.7, 1], opacity: [0.5, 0, 0.5] }}
                  transition={{ duration: 2.4, repeat: Infinity, ease: "easeInOut" }}
                />
              )}
            </span>
            <span className="text-[15px] font-semibold text-vb-fg">
              {running ? "Мост активен" : "Мост остановлен"}
            </span>
            {running && status?.pid !== undefined && (
              <span className="tnum ml-auto rounded-md border border-vb-border bg-vb-surface-2 px-2 py-0.5 font-mono text-[11px] text-vb-silver-dim">
                PID {status.pid}
              </span>
            )}
          </div>

          <dl className="mt-5 grid grid-cols-2 gap-x-6 gap-y-4 text-[13px]">
            <div>
              <dt className="flex items-center gap-1.5 text-vb-silver-dim">
                <Activity className="h-3.5 w-3.5" />
                Время работы
              </dt>
              <dd className="tnum mt-1 font-mono text-vb-fg">
                {running ? formatUptime(status?.uptime_sec) : "—"}
              </dd>
            </div>
            <div>
              <dt className="flex items-center gap-1.5 text-vb-silver-dim">
                <Wifi className="h-3.5 w-3.5" />
                Слушает
              </dt>
              <dd className="tnum mt-1 font-mono text-vb-fg">
                {status?.listen ?? "—"}
              </dd>
            </div>
            <div className="col-span-2">
              <dt className="flex items-center gap-1.5 text-vb-silver-dim">
                <CalendarClock className="h-3.5 w-3.5" />
                Автозапуск
              </dt>
              <dd className="mt-1 flex items-center gap-2 text-[13px]">
                {task === null ? (
                  // Статус ещё грузится — не пугаем ложным «устаревший».
                  <span className="text-vb-silver-dim">—</span>
                ) : task.registered ? (
                  <span className="text-vb-fg">
                    Планировщик задач{" "}
                    <span className="text-vb-emerald">✓ автоперезапуск при падении</span>
                  </span>
                ) : (
                  <>
                    <span className="text-vb-warn">
                      устаревший (.vbs в Startup) — упавший мост сам не поднимется
                    </span>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={handleTaskRegister}
                      disabled={taskBusy}
                    >
                      {taskBusy ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : null}
                      Перенастроить
                    </Button>
                  </>
                )}
              </dd>
            </div>
            <div className="col-span-2">
              <dt className="flex items-center gap-1.5 text-vb-silver-dim">
                <Globe className="h-3.5 w-3.5" />
                Upstream-прокси
              </dt>
              <dd className="mt-1 flex items-center gap-2">
                <span className="truncate font-mono text-[13px] text-vb-fg">
                  {status?.upstream_masked ?? "—"}
                </span>
                {status?.upstream_masked && (
                  <button
                    type="button"
                    onClick={copyUpstream}
                    className="text-vb-silver-dim transition-colors hover:text-vb-silver"
                    title="Копировать"
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </button>
                )}
                {copied && <span className="text-[12px] text-vb-emerald">скопировано</span>}
              </dd>
            </div>
          </dl>

          <div className="mt-5 flex flex-wrap gap-2">
            {running ? (
              <>
                <Button variant="primary" size="sm" onClick={handleTest} disabled={busy !== null}>
                  <Activity className="h-3.5 w-3.5" />
                  {busy === "test" ? "Проверка…" : "Тест"}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => bridgeAction("restart", shimRestart)}
                  disabled={busy !== null}
                >
                  <RotateCw className="h-3.5 w-3.5" />
                  Перезапустить
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => bridgeAction("stop", shimStop)}
                  disabled={busy !== null}
                >
                  <Pause className="h-3.5 w-3.5" />
                  Остановить
                </Button>
              </>
            ) : (
              <Button
                variant="primary"
                size="sm"
                onClick={() => bridgeAction("start", shimStart)}
                disabled={busy !== null}
              >
                <Play className="h-3.5 w-3.5" />
                {busy === "start" ? "Запуск…" : "Запустить мост"}
              </Button>
            )}
          </div>

          <AnimatePresence>
            {testResult && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                className="mt-4 overflow-hidden"
              >
                <div
                  className={cn(
                    "rounded-xl border p-3 text-[13px]",
                    testResult.ok
                      ? "border-vb-emerald/30 bg-vb-emerald/[0.04]"
                      : "border-vb-loss/30 bg-vb-loss/[0.04]",
                  )}
                >
                  <div className="flex items-center gap-2">
                    <span
                      className={cn(
                        "h-1.5 w-1.5 rounded-full",
                        testResult.ok ? "bg-vb-emerald" : "bg-vb-loss",
                      )}
                    />
                    <span className="font-medium text-vb-fg">
                      {testResult.ok ? "Соединение работает" : "Не удалось проверить"}
                    </span>
                    {testResult.latency_ms !== undefined && (
                      <span className="tnum ml-auto font-mono text-[11px] text-vb-silver-dim">
                        {testResult.latency_ms} ms
                      </span>
                    )}
                  </div>
                  {testResult.external_ip && (
                    <div className="mt-1.5 font-mono text-[12px] text-vb-silver-dim">
                      IP-выход:{" "}
                      <span className="text-vb-fg">{testResult.external_ip}</span>
                    </div>
                  )}
                  {testResult.error && (
                    <div className="mt-1.5 text-[12px] text-vb-loss/90">{testResult.error}</div>
                  )}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </section>

        {/* Системное проксирование — главный выключатель */}
        <section
          className={cn(
            "glass-card p-5",
            envOn && "border-vb-emerald/25",
          )}
        >
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h2 className="flex items-center gap-2 text-[15px] font-semibold text-vb-fg">
                <Network className="h-4 w-4" />
                Системное проксирование
              </h2>
              <p className="mt-1 text-[12px] text-vb-silver-dim">
                HTTP(S)_PROXY → мост для всей системы. Каждый новый процесс —
                VS Code, терминал, Claude Code, git, node — проксируется сам,
                без запуска через хелпер и без ярлыков.
              </p>
            </div>
            {globalEnv === null ? (
              <Loader2 className="h-5 w-5 shrink-0 animate-spin text-vb-silver-dim" />
            ) : (
              <Toggle
                checked={envOn}
                onChange={handleEnvToggle}
                disabled={envBusy}
                label="Системный прокси"
              />
            )}
          </div>

          {envForeign && (
            <div className="mt-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] p-3 text-[12px]">
              <p className="text-vb-warn">
                В системе уже есть прокси-переменные, указывающие НЕ на мост:
              </p>
              <div className="mt-1.5 space-y-0.5 font-mono text-[11px] text-vb-silver-dim">
                {globalEnv?.values.map(([k, v]) => (
                  <div key={k} className="truncate">
                    {k}=<span className="text-vb-fg">{v}</span>
                  </div>
                ))}
              </div>
              <p className="mt-1.5 text-vb-silver-dim">
                Включение тумблера перепишет их на мост (127.0.0.1:8889).
              </p>
            </div>
          )}

          {envOn && (
            <p className="mt-3 text-[11px] text-vb-silver-faint">
              Действует на процессы, запущенные после включения. Уже открытые
              терминалы нужно перезапустить. NO_PROXY-исключения сохранены.
            </p>
          )}
        </section>

        {/* Маршрутизация: smart (только AI через upstream) / all (всё) */}
        <section className="glass-card p-5">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h2 className="flex items-center gap-2 text-[15px] font-semibold text-vb-fg">
                <Route className="h-4 w-4" />
                Умная маршрутизация
              </h2>
              <p className="mt-1 text-[12px] text-vb-silver-dim">
                Через платный прокси — только AI-домены (Anthropic, OpenAI,
                Copilot…). Остальной трафик идёт напрямую, не тратя upstream.
              </p>
            </div>
            {health ? (
              <Toggle
                checked={smartOn}
                onChange={handleRouteToggle}
                disabled={routeBusy}
                label="Умная маршрутизация"
              />
            ) : running && oldBridge ? (
              <Button
                variant="secondary"
                size="sm"
                onClick={handleBridgeUpdate}
                disabled={updatingBridge}
              >
                {updatingBridge ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RotateCw className="h-3.5 w-3.5" />
                )}
                Обновить мост
              </Button>
            ) : running ? (
              // Мост запускается/перезапускается — healthz ещё молчит.
              <Loader2 className="h-5 w-5 shrink-0 animate-spin text-vb-silver-dim" />
            ) : (
              <span className="shrink-0 text-[12px] text-vb-silver-dim">мост не запущен</span>
            )}
          </div>

          {oldBridge && running && (
            <p className="mt-3 text-[12px] text-vb-warn">
              Мост старой версии (без /healthz) — маршрутизация недоступна.
              «Обновить мост» заменит его на новый (Rust) с сохранением настроек.
            </p>
          )}
          {updateError && <p className="mt-2 text-[12px] text-vb-loss">{updateError}</p>}

          {health && (
            <div className="mt-4 border-t border-vb-border pt-3">
              <div className="flex flex-wrap items-center gap-x-5 gap-y-1.5 text-[12px] text-vb-silver-dim">
                <span>
                  через прокси:{" "}
                  <span className="tnum font-mono text-vb-fg">{health.via_upstream}</span>
                </span>
                <span>
                  напрямую:{" "}
                  <span className="tnum font-mono text-vb-fg">{health.via_direct}</span>
                </span>
                <span>
                  активных:{" "}
                  <span className="tnum font-mono text-vb-fg">{health.active}</span>
                </span>
                {health.errors > 0 && (
                  <span className="text-vb-warn">
                    ошибок: <span className="tnum font-mono">{health.errors}</span>
                  </span>
                )}
                <span className="ml-auto font-mono text-[11px] text-vb-silver-faint">
                  clod-bridge {health.version}
                </span>
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {health.upstreams.map((u) => (
                  <span
                    key={u.url}
                    className={cn(
                      "flex items-center gap-1.5 rounded-md border px-2 py-0.5 font-mono text-[11px]",
                      u.healthy
                        ? "border-vb-emerald/25 text-vb-silver"
                        : "border-vb-loss/30 text-vb-loss",
                    )}
                  >
                    <span
                      className={cn(
                        "h-1.5 w-1.5 rounded-full",
                        u.healthy ? "bg-vb-emerald" : "bg-vb-loss",
                      )}
                    />
                    {u.url}
                  </span>
                ))}
              </div>
            </div>
          )}
        </section>

        {/* Killswitch */}
        <KillswitchCard
          status={ks}
          bridgeRunning={running}
          enabledAppCount={enabledCount}
          onEnable={async () => {
            setKs(await killswitchEnable());
          }}
          onDisable={async () => {
            setKs(await killswitchDisable());
          }}
        />

        {/* Apps map */}
        <section className="flex flex-col gap-3">
          <div className="flex items-center justify-between px-1">
            <div>
              <h2 className="text-[15px] font-semibold text-vb-fg">Приложения</h2>
              <p className="mt-0.5 text-[12px] text-vb-silver-dim">
                Отметьте, что проксировать через мост
              </p>
            </div>
            <Button variant="ghost" size="sm" onClick={onOpenSettings}>
              <Plus className="h-3.5 w-3.5" />
              Добавить
            </Button>
          </div>

          <motion.div
            variants={staggerContainer}
            initial="hidden"
            animate="show"
            className="flex flex-col gap-2.5"
          >
            <AnimatePresence mode="popLayout">
              {apps.map((app) => (
                <AppRow
                  key={app.id}
                  app={app}
                  onToggle={(en) => toggleApp(app.id, en)}
                  onLaunch={() => launchApp(app.id)}
                  onShortcut={(t) => shortcutApp(app.id, t)}
                  onRemove={app.builtin ? undefined : () => removeApp(app.id)}
                />
              ))}
            </AnimatePresence>
          </motion.div>

          {apps.length === 0 && (
            <motion.div
              variants={fadeInUp}
              initial="hidden"
              animate="show"
              className="surface-card p-6 text-center text-[13px] text-vb-silver-dim"
            >
              <Loader2 className="mx-auto mb-2 h-4 w-4 animate-spin" />
              Загружаю приложения…
            </motion.div>
          )}
        </section>
      </div>
    </div>
  );
}
