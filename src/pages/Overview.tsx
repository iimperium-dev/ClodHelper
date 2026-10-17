import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Loader2,
  Network,
  Pause,
  Play,
  RotateCw,
  Route,
} from "lucide-react";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { StatusOrb } from "../components/StatusOrb";
import { NumberTicker } from "../components/NumberTicker";
import { Sparkline } from "../components/Sparkline";
import { InfoTip } from "../components/InfoTip";
import {
  bridgeSetRouteMode,
  bridgeTaskRegister,
  bridgeUpdate,
  clearGlobalProxyEnv,
  setGlobalProxyEnv,
  shimRestart,
  shimStart,
  shimStop,
  shimTest,
} from "../lib/api";
import { formatUptime, useStatus } from "../lib/status";
import type { ShimTestResult } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

interface Props {
  /** Переход в раздел «Прокси» (клик по строке пула). */
  onOpenProxies: () => void;
}

/** host:port из masked-URL пула ("http://user:****@host:port"). */
function chipLabel(masked: string): string {
  const rest = masked.includes("://") ? masked.split("://")[1] : masked;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

/** "N сек/мин/ч назад" для списка ошибок (t — секунды-от-старта моста). */
function agoLabel(uptimeSec: number, t: number): string {
  const d = Math.max(0, uptimeSec - t);
  if (d < 60) return `${d} сек назад`;
  if (d < 3600) return `${Math.floor(d / 60)} мин назад`;
  return `${Math.floor(d / 3600)} ч назад`;
}

/*
  Обзор — hero-зона статуса (НЕ карточка: открытая композиция с орбом),
  живые метрики с тикерами и sparkline, и группа управления одной
  поверхностью с hairline-разделителями (вместо трёх отдельных карточек).
  Описания «для чайника» живут в «?»-подсказках, не в инлайн-тексте.
*/
export function Overview({ onOpenProxies }: Props) {
  const {
    status,
    health,
    healthErr,
    task,
    env,
    exeLegacy,
    versions,
    quality,
    orb,
    history,
    poolPing,
    refresh,
    setEnv,
  } = useStatus();

  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [envBusy, setEnvBusy] = useState(false);
  const [routeBusy, setRouteBusy] = useState(false);
  const [taskBusy, setTaskBusy] = useState(false);
  const [updatingBridge, setUpdatingBridge] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [errorsOpen, setErrorsOpen] = useState(false);

  const loading = status === null;
  const running = status?.running ?? false;
  // Прокси, у которых за последнюю минуту часть подключений не прошла.
  const shaky = quality.filter((q) => q.state !== "ok");
  const shakyFails = shaky.reduce((a, q) => a + q.recentFail, 0);
  const shakyTotal = shaky.reduce((a, q) => a + q.recentOk + q.recentFail, 0);
  const bridgeOutdated = running && (versions?.outdated ?? false);

  const envOn = (env?.present ?? false) && (env?.points_to_bridge ?? false);
  const envForeign = (env?.present ?? false) && !(env?.points_to_bridge ?? false);
  const smartOn = health?.mode === "smart";
  const oldBridge =
    !health && (exeLegacy || (healthErr !== null && healthErr.includes("разобрать")));

  // ── Действия моста ──
  // Все команды могут реально упасть (порт занят, exe удалён, PowerShell
  // отказал) — без catch клик выглядел бы как «нажал и ничего»: спиннер
  // мигнул, ошибки нет. Показываем её строкой под hero.
  const [actionError, setActionError] = useState<string | null>(null);

  const bridgeAction = async (
    kind: "start" | "stop" | "restart",
    fn: () => Promise<unknown>,
  ) => {
    setBusy(kind);
    setActionError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleTest = async () => {
    setBusy("test");
    setTestResult(null);
    setActionError(null);
    try {
      setTestResult(await shimTest());
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleEnvToggle = async () => {
    setEnvBusy(true);
    setActionError(null);
    try {
      setEnv(envOn ? await clearGlobalProxyEnv() : await setGlobalProxyEnv());
    } catch (e) {
      setActionError(String(e));
    } finally {
      setEnvBusy(false);
    }
  };

  const handleRouteToggle = async () => {
    setRouteBusy(true);
    setActionError(null);
    try {
      await bridgeSetRouteMode(smartOn ? "all" : "smart");
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setRouteBusy(false);
    }
  };

  const handleTaskRegister = async () => {
    setTaskBusy(true);
    setActionError(null);
    try {
      await bridgeTaskRegister();
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setTaskBusy(false);
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

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[620px] flex-col px-8 py-7"
    >
      {/* ── Hero: статус ── */}
      <motion.section variants={fadeInUp}>
        <div className="flex items-center gap-4">
          <StatusOrb state={orb} size={16} />
          <div className="min-w-0 flex-1">
            <h1 className="text-[24px] font-bold leading-tight tracking-[-0.02em] text-vb-fg">
              {loading
                ? "Проверяю мост…"
                : running
                  ? "Мост активен"
                  : "Мост остановлен"}
            </h1>
            <p className="tnum mt-0.5 text-[13px] text-vb-silver-dim">
              {running ? (
                <>
                  {formatUptime(health?.uptime_sec ?? status?.uptime_sec)}
                  {" · "}
                  <span className="font-mono">{status?.listen ?? "127.0.0.1:8889"}</span>
                  {status?.pid !== undefined && (
                    <span className="font-mono text-vb-silver-faint"> · PID {status.pid}</span>
                  )}
                </>
              ) : loading ? (
                " "
              ) : (
                "Трафик AI-инструментов не проксируется"
              )}
            </p>
          </div>

          {/* Действия */}
          <div className="flex shrink-0 items-center gap-2">
            {loading ? null : running ? (
              <>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleTest}
                  disabled={busy !== null}
                >
                  {busy === "test" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Activity className="h-3.5 w-3.5" />
                  )}
                  Тест
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => bridgeAction("restart", shimRestart)}
                  disabled={busy !== null}
                  title="Перезапустить мост"
                >
                  <RotateCw
                    className={cn("h-3.5 w-3.5", busy === "restart" && "animate-spin")}
                  />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => bridgeAction("stop", shimStop)}
                  disabled={busy !== null}
                  title="Остановить мост"
                >
                  <Pause className="h-3.5 w-3.5" />
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
                {busy === "start" ? "Запуск…" : "Запустить"}
              </Button>
            )}
          </div>
        </div>

        {/* Ошибка последнего действия (запуск/стоп/тумблеры) */}
        <AnimatePresence>
          {actionError && (
            <motion.p
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
              className="mt-2 overflow-hidden text-[12px] text-vb-loss"
            >
              {actionError}
            </motion.p>
          )}
        </AnimatePresence>

        {/* Результат теста */}
        <AnimatePresence>
          {testResult && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
              className="overflow-hidden"
            >
              <div
                className={cn(
                  "mt-4 rounded-lg border p-3 text-[13px]",
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
                  {testResult.external_ip && (
                    <span className="font-mono text-[12px] text-vb-silver-dim">
                      · {testResult.external_ip}
                    </span>
                  )}
                  {testResult.latency_ms !== undefined && (
                    <span className="tnum ml-auto font-mono text-[11px] text-vb-silver-dim">
                      {testResult.latency_ms} ms
                    </span>
                  )}
                </div>
                {testResult.error && (
                  <div className="mt-1.5 text-[12px] text-vb-loss/90">{testResult.error}</div>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.section>

      {/* ── Живые метрики + sparkline ── */}
      <motion.section variants={fadeInUp} className="mt-7">
        <div className="mb-2.5 flex items-center gap-2">
          <h2 className="text-[11px] font-medium uppercase tracking-[0.08em] text-vb-silver-faint">
            Соединения моста
          </h2>
          <InfoTip>
            Счётчики с момента запуска моста. <b>Через прокси</b> — соединения
            к AI-сервисам, ушедшие через платный прокси. <b>Напрямую</b> —
            обычный трафик в обход прокси (умная маршрутизация). <b>Сейчас</b> —
            открытые соединения в эту секунду. <b>Сбои</b> — соединения,
            которые не удалось установить: кликните на цифру «Сбои», чтобы
            посмотреть подробности. Единичные сбои — это нормально, мост
            повторяет сам. При перезапуске моста счётчики обнуляются.
          </InfoTip>
        </div>
        <div className="flex items-end justify-between gap-6">
          <div className="flex gap-7">
            <div>
              <div className="text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
                Через прокси
              </div>
              <div className="mt-1 font-mono text-[20px] font-semibold leading-none text-vb-fg">
                <NumberTicker value={health?.via_upstream ?? 0} />
              </div>
            </div>
            <div>
              <div className="text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
                Напрямую
              </div>
              <div className="mt-1 font-mono text-[20px] font-semibold leading-none text-vb-fg">
                <NumberTicker value={health?.via_direct ?? 0} />
              </div>
            </div>
            <div>
              <div className="text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
                Сейчас
              </div>
              <div className="mt-1 font-mono text-[20px] font-semibold leading-none text-vb-emerald">
                <NumberTicker value={health?.active ?? 0} />
              </div>
            </div>
            {/* «Сбои» видны ВСЕГДА и всегда кликабельны (0 — нейтральный
                серый): иначе пользователь не понимает, куда делись ошибки. */}
            <button
              type="button"
              onClick={() => setErrorsOpen((v) => !v)}
              className="text-left transition-opacity hover:opacity-80 active:scale-[0.98]"
              title="Показать последние сбои"
            >
              <div
                className={cn(
                  "flex items-center gap-1 text-[11px] font-medium uppercase tracking-[0.06em]",
                  (health?.errors ?? 0) > 0 ? "text-vb-warn/80" : "text-vb-silver-faint",
                )}
              >
                Сбои
                <ChevronDown
                  className={cn(
                    "h-3 w-3 transition-transform duration-200",
                    errorsOpen && "rotate-180",
                  )}
                />
              </div>
              <div
                className={cn(
                  "mt-1 font-mono text-[20px] font-semibold leading-none",
                  (health?.errors ?? 0) > 0 ? "text-vb-warn" : "text-vb-silver-dim",
                )}
              >
                <NumberTicker value={health?.errors ?? 0} />
              </div>
            </button>
          </div>
          <Sparkline data={history} width={170} height={34} className="mb-0.5 shrink-0" />
        </div>

        {/* Последние сбои: чайник должен ВИДЕТЬ, что именно ломалось */}
        <AnimatePresence>
          {errorsOpen && health && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
              className="overflow-hidden"
            >
              <div className="mt-3 rounded-lg border border-vb-border bg-vb-surface p-3">
                <div className="mb-1.5 text-[11px] text-vb-silver-faint">
                  Последние сбои · мост повторяет неудачные соединения сам
                </div>
                {(health.last_errors ?? []).length === 0 ? (
                  <p className="text-[12px] text-vb-silver-dim">
                    {health.errors === 0
                      ? "Сбоев с момента запуска моста нет — всё чисто."
                      : "Подробности сбоев появятся после обновления моста до 1.1.0 (раздел «Прокси»)."}
                  </p>
                ) : (
                  <div className="space-y-1">
                    {[...(health.last_errors ?? [])]
                      .reverse()
                      .slice(0, 6)
                      .map((e, i) => (
                        <div key={i} className="flex gap-2 font-mono text-[11px]">
                          <span className="tnum w-[86px] shrink-0 text-vb-silver-faint">
                            {agoLabel(health.uptime_sec, e.t)}
                          </span>
                          <span className="truncate text-vb-silver">{e.msg}</span>
                        </div>
                      ))}
                  </div>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.section>

      {/* ── Управление: одна поверхность, hairline-строки ── */}
      <motion.section variants={fadeInUp} className="surface-card mt-7 divide-y divide-vb-border/70">
        {/* Системное проксирование */}
        <div className="p-4">
          <div className="flex items-center justify-between gap-4">
            <div className="flex min-w-0 items-center gap-3">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-vb-surface-2 text-vb-silver-dim">
                <Network className="h-4 w-4" strokeWidth={1.9} />
              </div>
              <div className="flex min-w-0 items-center gap-2">
                <div className="text-[14px] font-semibold text-vb-fg">
                  Системное проксирование
                </div>
                <InfoTip>
                  Прописывает переменные HTTP_PROXY/HTTPS_PROXY, и вся система
                  (VS Code, терминал, Claude Code, git, node) автоматически идёт
                  через мост — без ярлыков и запусков «через хелпер». Действует
                  на процессы, запущенные после включения: уже открытые
                  терминалы нужно перезапустить.
                </InfoTip>
              </div>
            </div>
            {env === null ? (
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
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
                Прокси-переменные в системе указывают НЕ на мост:
              </p>
              <div className="mt-1.5 space-y-0.5 font-mono text-[11px] text-vb-silver-dim">
                {env?.values.map(([k, v]) => (
                  <div key={k} className="truncate">
                    {k}=<span className="text-vb-fg">{v}</span>
                  </div>
                ))}
              </div>
              <p className="mt-1.5 text-vb-silver-dim">
                Включение перепишет их на 127.0.0.1:8889.
              </p>
            </div>
          )}
        </div>

        {/* Умная маршрутизация */}
        <div className="p-4">
          <div className="flex items-center justify-between gap-4">
            <div className="flex min-w-0 items-center gap-3">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-vb-surface-2 text-vb-silver-dim">
                <Route className="h-4 w-4" strokeWidth={1.9} />
              </div>
              <div className="flex min-w-0 items-center gap-2">
                <div className="text-[14px] font-semibold text-vb-fg">
                  Умная маршрутизация
                </div>
                <InfoTip>
                  Через платный прокси идут только AI-домены (Anthropic, OpenAI,
                  Copilot…), остальной трафик — напрямую: быстрее и не тратит
                  трафик прокси. Выключено — весь трафик через прокси.
                </InfoTip>
              </div>
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
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
            ) : (
              <span className="shrink-0 text-[12px] text-vb-silver-faint">
                мост не запущен
              </span>
            )}
          </div>
          {oldBridge && running && (
            <p className="mt-2.5 pl-11 text-[12px] leading-snug text-vb-warn">
              Мост старой версии (без /healthz) — «Обновить мост» заменит его на новый
              с сохранением настроек.
            </p>
          )}
          {updateError && (
            <p className="mt-2 pl-11 text-[12px] text-vb-loss">{updateError}</p>
          )}
        </div>

        {/* Прокси-пул: чипы host:port (без голых URL с маской) → раздел «Прокси» */}
        <div>
          <button
            type="button"
            onClick={onOpenProxies}
            className="group flex w-full items-center gap-3 p-4 text-left transition-colors hover:bg-vb-surface-2/40"
          >
            <div className="shrink-0 text-[11px] font-medium uppercase tracking-[0.06em] text-vb-silver-faint">
              Прокси
            </div>
            <div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-1.5">
              {(poolPing.length > 0
                ? poolPing.map((p, i) => ({
                    label: `${p.host}:${p.port}`,
                    // Живое качество из /healthz важнее 5-минутного пинга.
                    state: quality[i]?.state ?? (p.ms !== null ? "ok" : "down"),
                    ms: p.ms,
                    q: quality[i],
                  }))
                : (health?.upstreams ?? []).map((u, i) => ({
                    label: chipLabel(u.url),
                    state: quality[i]?.state ?? (u.healthy ? "ok" : "down"),
                    ms: null as number | null,
                    q: quality[i],
                  }))
              ).map((c, ci) => (
                <span
                  key={`${ci}:${c.label}`}
                  title={
                    c.q
                      ? `За минуту: ${c.q.recentOk} успешных, ${c.q.recentFail} неудачных подключений`
                      : undefined
                  }
                  className="flex items-center gap-1.5 rounded-md border border-vb-border bg-vb-surface-2 px-2 py-0.5 font-mono text-[11px] text-vb-silver"
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 rounded-full",
                      c.state === "ok"
                        ? "bg-vb-emerald"
                        : c.state === "unstable"
                          ? "bg-vb-warn"
                          : "bg-vb-loss",
                    )}
                  />
                  {c.label}
                  {c.ms !== null && (
                    <span className="tnum text-vb-silver-faint">{c.ms} мс</span>
                  )}
                </span>
              ))}
              {poolPing.length === 0 && !health && (
                <span className="text-[12px] text-vb-silver-faint">настроить</span>
              )}
            </div>
            <ChevronRight className="h-4 w-4 shrink-0 text-vb-silver-faint transition-transform duration-150 group-hover:translate-x-0.5 group-hover:text-vb-silver" />
          </button>
          {/* Нестабильный прокси: объясняем, что происходит и что это не опасно */}
          {running && shaky.length > 0 && (
            <div className="px-4 pb-4">
              <div className="rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] p-3">
                <p className="text-[12px] leading-snug text-vb-warn">
                  {shaky.some((q) => q.state === "down")
                    ? "Прокси не отвечает."
                    : `Прокси нестабилен: за минуту не прошло ${shakyFails} из ${shakyTotal} подключений.`}{" "}
                  Мост повторяет попытки к тому же прокси — IP не меняется, трафик в
                  обход прокси не идёт. Если так постоянно — попросите у продавца другой
                  сервер.
                </p>
              </div>
            </div>
          )}
          {/* Встроенный в приложение мост новее установленного */}
          {bridgeOutdated && (
            <div className="px-4 pb-4">
              <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-border bg-vb-surface-2/60 p-3">
                <p className="text-[12px] leading-snug text-vb-silver">
                  Доступно обновление моста {versions?.installed} → {versions?.bundled}:
                  повторы при сбоях прокси, пароль прокси скрыт из списка процессов.
                </p>
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
                  Обновить
                </Button>
              </div>
              {updateError && (
                <p className="mt-2 text-[12px] text-vb-loss">{updateError}</p>
              )}
            </div>
          )}
          {/* Автозапуск: показываем только проблему (прогрессивное раскрытие) */}
          {task !== null && !task.registered && (
            <div className="px-4 pb-4">
              <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] p-3">
                <p className="text-[12px] leading-snug text-vb-warn">
                  Автозапуск моста не настроен — после сбоя он не восстановится.
                </p>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={handleTaskRegister}
                  disabled={taskBusy}
                >
                  {taskBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  Исправить
                </Button>
              </div>
            </div>
          )}
        </div>
      </motion.section>

      {/* Версия моста — тихий факт внизу */}
      {health && (
        <motion.p
          variants={fadeInUp}
          className="mt-4 text-right font-mono text-[11px] text-vb-silver-faint"
        >
          clod-bridge {health.version} · планировщик{" "}
          {task?.registered ? "активен" : "—"}
        </motion.p>
      )}
    </motion.div>
  );
}
