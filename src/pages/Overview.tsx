import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Loader2,
  Pause,
  Play,
  RotateCw,
} from "lucide-react";
import { Button } from "../components/Button";
import { StatusOrb } from "../components/StatusOrb";
import { NumberTicker } from "../components/NumberTicker";
import { Sparkline } from "../components/Sparkline";
import { UpdateBanner } from "../components/UpdateBanner";
import {
  bridgeTaskRegister,
  bridgeUpdate,
  shimRestart,
  shimStart,
  shimStop,
  shimTest,
  privacyCheck,
  appsList,
  proxyAssignmentsGet,
  proxyLabelsGet,
  sitesGet,
} from "../lib/api";
import { formatUptime, isUsable, useStatus, type ProxyState } from "../lib/status";
import type { AppProfileStatus, PrivacyReport, ShimTestResult, SiteEntry } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

interface Props {
  /** Переход в раздел «Прокси» (клик по строке пула). */
  onOpenProxies: () => void;
}

/** Названия сервисов назначений — как в «Прокси». */
const SERVICE_NAMES: Record<string, string> = {
  anthropic: "Claude",
  openai: "ChatGPT",
  google: "Gemini",
  other_ai: "остальные AI",
  telegram: "Telegram",
};

/** Кто закреплён за прокси пула: сервисы, сайты, приложения. */
interface Pins {
  assign: Record<string, number>;
  sites: SiteEntry[];
  apps: AppProfileStatus[];
  labels: Record<string, string>;
}

function pinnedTo(pins: Pins | null, i: number): string[] {
  if (!pins) return [];
  return [
    ...Object.entries(pins.assign)
      .filter(([, v]) => v === i)
      .map(([k]) => SERVICE_NAMES[k] ?? k),
    ...pins.sites.filter((s) => s.enabled && s.proxy === i).map((s) => s.name),
    ...pins.apps.filter((a) => !a.hidden && a.proxy === i).map((a) => a.name),
  ];
}

/** Что значит отказ прокси для пользователя — одной строкой. */
function impactOf(p: ProxyState, i: number, exit: number | null, allMode: boolean, pins: Pins | null, proxies: ProxyState[]): string {
  // Резерв: сейчас ничего не ломает, но подстраховки у своего прокси нет.
  if (p.backupOf !== undefined) {
    const owner = proxies[p.backupOf];
    return owner ? `Резерв для ${owner.host}: если тот упадёт, этот не подхватит` : "Резервный прокси";
  }
  const users = pinnedTo(pins, i);
  // Подменён живым резервом — всё работает, но с другого IP.
  if (p.coveredBy !== undefined) {
    const sub = proxies[p.coveredBy];
    return `Подменяет резерв ${sub?.host ?? ""}${users.length ? ` — ${users.slice(0, 4).join(", ")} работают через него` : ""}`;
  }
  const parts: string[] = [];
  if (exit === i) {
    parts.push(allMode ? "Через него идёт весь трафик — сейчас он стоит" : "Через него сейчас выход в интернет");
  } else if (i === 0 && !allMode) {
    const alt = exit !== null ? proxies[exit] : undefined;
    parts.push(alt ? `Основной: «Авто» сейчас идёт через ${alt.host}` : "Основной, а запасных живых нет");
  }
  if (users.length) parts.push(`${p.state === "down" ? "Не работают" : "Сбоят"}: ${users.slice(0, 4).join(", ")}${users.length > 4 ? "…" : ""}`);
  if (!parts.length) parts.push("Сейчас через него ничего не идёт");
  return parts.join(" · ");
}

/** "N сек/мин/ч назад" для списка ошибок (t — секунды-от-старта моста). */
function agoLabel(uptimeSec: number, t: number): string {
  const d = Math.max(0, uptimeSec - t);
  if (d < 60) return `${d} сек назад`;
  if (d < 3600) return `${Math.floor(d / 60)} мин назад`;
  return `${Math.floor(d / 3600)} ч назад`;
}

/** Техническую строку моста — в короткую человеческую причину. */
function humanError(msg: string): string {
  if (/unreachable|No route|os error 51|os error 65/i.test(msg)) return "Не было интернета на Mac";
  if (/логин\/пароль|407/.test(msg)) return "Прокси не принял логин или пароль";
  if (/молчит на CONNECT|нет ответа на TCP|не отвечает/.test(msg)) return "Прокси не отвечал";
  if (/ответил 5\d\d/.test(msg)) return "Прокси не смог открыть сайт";
  if (/прямое соединение/.test(msg)) return "Сайт не открылся напрямую";
  if (/лимит/.test(msg)) return "Слишком много соединений сразу";
  return msg.length > 60 ? `${msg.slice(0, 57)}…` : msg;
}

/** Сгруппировать одинаковые причины: свежие — первыми, с количеством. */
function errorGroups(
  errors: { t: number; msg: string }[],
  uptimeSec: number,
): { text: string; count: number; ago: string; fresh: boolean }[] {
  const map = new Map<string, { text: string; count: number; t: number }>();
  for (const e of errors) {
    const text = humanError(e.msg);
    const g = map.get(text);
    if (g) {
      g.count += 1;
      g.t = Math.max(g.t, e.t);
    } else {
      map.set(text, { text, count: 1, t: e.t });
    }
  }
  return [...map.values()]
    .sort((a, b) => b.t - a.t)
    .map((g) => ({
      text: g.text,
      count: g.count,
      ago: agoLabel(uptimeSec, g.t),
      // Старые перебои не должны вечно гореть оранжевым.
      fresh: uptimeSec - g.t < 30 * 60,
    }));
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
    exeLegacy,
    versions,
    proxies,
    orb,
    history,
    refresh,
    refreshPoolPing,
  } = useStatus();

  const [busy, setBusy] = useState<"start" | "stop" | "restart" | "test" | null>(null);
  const [testResult, setTestResult] = useState<ShimTestResult | null>(null);
  const [report, setReport] = useState<PrivacyReport | null>(null);
  const [taskBusy, setTaskBusy] = useState(false);
  const [updatingBridge, setUpdatingBridge] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [errorsOpen, setErrorsOpen] = useState(false);

  const loading = status === null;
  const running = status?.running ?? false;
  // Прокси с проблемами: каждый — отдельной строкой с последствиями.
  const issues = proxies.map((p, i) => ({ p, i })).filter(({ p }) => p.state === "down" || p.state === "unstable");
  const issueKey = issues.map(({ i, p }) => `${i}:${p.state}`).join(",");
  // Подпись под заголовком: что с прокси — одной фразой.
  const lostCount = issues.filter(({ p }) => p.backupOf === undefined && p.coveredBy === undefined).length;
  const issuesNote = !issues.length
    ? ""
    : lostCount > 1
      ? ` · ${lostCount} прокси с проблемами`
      : lostCount === 1
        ? ` · прокси ${issues.find(({ p }) => p.backupOf === undefined && p.coveredBy === undefined)!.p.state === "down" ? "не работает" : "сбоит"}`
        : issues.some(({ p }) => p.coveredBy !== undefined)
          ? " · работает резервный прокси"
          : " · резервный прокси не отвечает";
  // Кто за каким прокси закреплён — нужно только при проблемах.
  const [pins, setPins] = useState<Pins | null>(null);
  useEffect(() => {
    if (!issueKey) return;
    void Promise.all([
      proxyAssignmentsGet().catch(() => ({})),
      sitesGet().catch(() => [] as SiteEntry[]),
      appsList().catch(() => [] as AppProfileStatus[]),
      proxyLabelsGet().catch(() => ({})),
    ]).then(([assign, sites, apps, labels]) => setPins({ assign, sites, apps, labels }));
  }, [issueKey]);
  const bridgeOutdated = running && (versions?.outdated ?? false);

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

  /** «Проверить»: связь через мост + проверка приватности (IP, DNS, утечки). */
  const handleTest = async () => {
    setBusy("test");
    setTestResult(null);
    setReport(null);
    setActionError(null);
    try {
      // Заодно — свежий пинг: рядом с выходным IP должна стоять актуальная цифра.
      const [t, r] = await Promise.all([
        shimTest(),
        privacyCheck().catch(() => null),
        refreshPoolPing(),
      ]);
      setTestResult(t);
      setReport(r);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
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

  // Через какой прокси реально идёт трафик моста. «Весь трафик» — строго
  // через один (закреплённый за Claude, иначе основной), без запасных.
  // Умный режим («Авто») — первый работающий по порядку пула (мост так и
  // переключается). IP — ровно одно место на экране; после «Проверить» —
  // реальный выходной IP.
  const allMode = health?.mode === "all";
  // Сам прокси пула (по индексу моста) или резерв, который его подменяет.
  const viaCover = (i: number) => proxies[i]?.coveredBy ?? i;
  const poolIdx = proxies.map((p, i) => (p.backupOf === undefined ? i : -1)).filter((i) => i >= 0);
  const exitIndex: number | null = !poolIdx.length
    ? null
    : allMode && typeof health?.strict_upstream_index === "number"
      ? viaCover(health.strict_upstream_index)
      : allMode
        ? viaCover(0)
        : viaCover(poolIdx.find((i) => isUsable(proxies[i].state)) ?? 0);
  const exitProxy = exitIndex !== null ? proxies[exitIndex] : undefined;
  const exitIp =
    report?.exit?.ip ?? (testResult?.ok && testResult.external_ip ? testResult.external_ip : exitProxy?.host ?? null);
  // «мс» — всегда TCP-пинг прокси, как в «Прокси». Время запроса из
  // «Проверить» (TLS до сайта через прокси + ответ сервиса, секунды) сюда
  // не годится: рядом с IP его читали как пинг. Выход не совпал ни с одним
  // прокси пула (у провайдера выходной IP другой) — пинг не показываем.
  const exitMs = proxies.find((p) => p.host === exitIp)?.ms ?? undefined;
  const exitState = testResult && !testResult.ok ? "down" : (exitProxy?.state ?? "unknown");
  const spare = allMode ? 0 : poolIdx.filter((i) => i !== exitIndex && isUsable(proxies[i].state)).length;

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[460px] flex-col px-6 py-6"
    >
      {/* ── Статус ── */}
      <motion.section variants={fadeInUp} className="flex items-center gap-3">
        <StatusOrb state={orb} size={12} />
        <div className="min-w-0 flex-1">
          <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">
            {loading ? "Проверяю…" : running ? "Мост активен" : "Мост остановлен"}
          </h1>
          <p className="tnum text-[12px] text-vb-silver-faint">
            {running
              ? `работает ${formatUptime(health?.uptime_sec ?? status?.uptime_sec)}${issuesNote}`
              : loading
                ? " "
                : "Трафик не проксируется"}
          </p>
        </div>
        {!loading &&
          (running ? (
            <div className="flex shrink-0 items-center">
              <button
                type="button"
                onClick={() => bridgeAction("restart", shimRestart)}
                disabled={busy !== null}
                title="Перезапустить мост"
                className="rounded-lg p-2 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94] disabled:opacity-40"
              >
                <RotateCw className={cn("h-4 w-4", busy === "restart" && "animate-spin")} />
              </button>
              <button
                type="button"
                onClick={() => bridgeAction("stop", shimStop)}
                disabled={busy !== null}
                title="Остановить мост"
                className="rounded-lg p-2 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94] disabled:opacity-40"
              >
                <Pause className="h-4 w-4" />
              </button>
            </div>
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
          ))}
      </motion.section>

      <AnimatePresence>
        {actionError && (
          <motion.p
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            className="mt-2 overflow-hidden text-[12px] text-vb-loss"
          >
            {actionError}
          </motion.p>
        )}
      </AnimatePresence>

      {/* ── Выход в интернет: единственное место с IP ── */}
      {running && (
        <motion.section variants={fadeInUp} className="surface-card mt-5">
          <div className="flex items-center gap-3 p-4">
            <button
              type="button"
              onClick={onOpenProxies}
              className="group flex min-w-0 flex-1 items-center gap-3 text-left"
            >
              <span
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  exitState === "down"
                    ? "bg-vb-loss"
                    : exitState === "unstable" || exitState === "slow"
                      ? "bg-vb-warn"
                      : exitState === "unknown"
                        ? "bg-vb-silver-faint"
                        : "bg-vb-emerald",
                )}
              />
              <span className="min-w-0">
                <span className="block text-[11px] text-vb-silver-faint">
                  Выход через мост{allMode ? " · весь трафик" : spare > 0 ? ` · +${spare} запасн.` : ""}
                </span>
                <span className="flex items-baseline gap-2">
                  <span className="truncate font-mono text-[14px] text-vb-fg">
                    {exitIp ?? "прокси не настроен"}
                  </span>
                  {exitMs !== undefined && exitMs !== null && (
                    <span className="tnum shrink-0 font-mono text-[11px] text-vb-silver-faint">
                      {exitMs} мс
                    </span>
                  )}
                </span>
              </span>
              <ChevronRight className="ml-auto h-4 w-4 shrink-0 text-vb-silver-faint transition-transform group-hover:translate-x-0.5" />
            </button>
            <Button variant="secondary" size="sm" onClick={handleTest} disabled={busy !== null}>
              {busy === "test" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
              Проверить
            </Button>
          </div>
          {/* Проверка приватности — вердикт по пунктам */}
          <AnimatePresence>
            {report && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                className="overflow-hidden border-t border-vb-border/70"
              >
                <PrivacyRows report={report} />
              </motion.div>
            )}
          </AnimatePresence>
          {!report && testResult?.error && (
            <p className="border-t border-vb-border/70 px-4 py-2.5 text-[12px] leading-snug text-vb-loss">
              {testResult.error}
            </p>
          )}
          {/* Прокси с проблемами — каждый отдельно: что с ним и кого задевает */}
          {issues.length > 0 && (
            <ul className="divide-y divide-vb-border/50 border-t border-vb-border/70">
              {issues.map(({ p, i }) => {
                const name = pins?.labels[p.url];
                return (
                  <li key={p.url || i} className="flex items-start gap-2.5 px-4 py-2.5">
                    <AlertCircle
                      className={cn(
                        "mt-0.5 h-3.5 w-3.5 shrink-0",
                        p.state === "down" && p.coveredBy === undefined && p.backupOf === undefined
                          ? "text-vb-loss"
                          : "text-vb-warn",
                      )}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block text-[12px] text-vb-fg">
                        {name ? `${name} · ` : ""}
                        {p.backupOf !== undefined && "резерв "}
                        <span className="font-mono">{p.host}</span>{" "}
                        {p.state === "down" ? "не работает" : "нестабилен"}
                      </span>
                      {p.reason && <span className="block text-[11px] leading-snug text-vb-silver-faint">{p.reason}</span>}
                      <span className="block text-[11px] leading-snug text-vb-silver">
                        {impactOf(p, i, exitIndex, allMode, pins, proxies)}
                      </span>
                    </span>
                    <button
                      type="button"
                      onClick={onOpenProxies}
                      className="shrink-0 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
                    >
                      Прокси →
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </motion.section>
      )}

      {/* ── Трафик: доля через прокси / напрямую + человеческий статус сбоев ── */}
      {running && health && (
        <motion.section variants={fadeInUp} className="mt-5">
          {(() => {
            const total = health.via_upstream + health.via_direct;
            const share = total > 0 ? (health.via_upstream / total) * 100 : 0;
            const groups = errorGroups(health.last_errors ?? [], health.uptime_sec);
            const lastAgo = groups[0]?.ago;
            return (
              <>
                <div className="flex items-center gap-3">
                  <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-vb-surface-2">
                    <motion.div
                      className="absolute inset-y-0 left-0 rounded-full bg-vb-emerald"
                      initial={false}
                      animate={{ width: `${share}%` }}
                      transition={{ type: "spring", bounce: 0, duration: 0.6 }}
                    />
                  </div>
                  <Sparkline data={history} width={72} height={20} className="shrink-0 opacity-80" />
                </div>
                <div className="tnum mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 whitespace-nowrap text-[12px] text-vb-silver-faint">
                  <span>
                    <span className="text-vb-silver">
                      <NumberTicker value={health.via_upstream} />
                    </span>{" "}
                    через прокси
                  </span>
                  <span>
                    <span className="text-vb-silver">
                      <NumberTicker value={health.via_direct} />
                    </span>{" "}
                    напрямую
                  </span>
                  <span className="flex items-center gap-1.5">
                    <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-vb-emerald" />
                    <NumberTicker value={health.active} /> сейчас
                  </span>
                  <button
                    type="button"
                    onClick={() => groups.length > 0 && setErrorsOpen((v) => !v)}
                    className={cn(
                      "ml-auto flex items-center gap-0.5 transition-colors",
                      groups.length === 0
                        ? "cursor-default"
                        : groups[0].fresh
                          ? "text-vb-warn/90 hover:text-vb-warn"
                          : "hover:text-vb-silver",
                    )}
                  >
                    {groups.length > 0 ? `перебои · ${lastAgo}` : "без сбоев"}
                    {groups.length > 0 && (
                      <ChevronDown className={cn("h-3 w-3 transition-transform", errorsOpen && "rotate-180")} />
                    )}
                  </button>
                </div>
                <AnimatePresence>
                  {errorsOpen && groups.length > 0 && (
                    <motion.ul
                      initial={{ opacity: 0, height: 0 }}
                      animate={{ opacity: 1, height: "auto" }}
                      exit={{ opacity: 0, height: 0 }}
                      transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
                      className="mt-2 overflow-hidden"
                    >
                      {groups.slice(0, 3).map((g) => (
                        <li key={g.text} className="flex items-center gap-2 py-1 text-[12px]">
                          <span className="h-1 w-1 shrink-0 rounded-full bg-vb-warn/70" />
                          <span className="min-w-0 flex-1 truncate text-vb-silver">{g.text}</span>
                          {g.count > 1 && <span className="tnum text-vb-silver-faint">×{g.count}</span>}
                          <span className="tnum shrink-0 text-vb-silver-faint">{g.ago}</span>
                        </li>
                      ))}
                    </motion.ul>
                  )}
                </AnimatePresence>
              </>
            );
          })()}
        </motion.section>
      )}

      {/* ── Требует внимания (только когда есть что сделать) ── */}
      {(bridgeOutdated || (running && oldBridge) || (task !== null && !task.registered)) && (
        <motion.section variants={fadeInUp} className="mt-3 space-y-2">
          {(bridgeOutdated || (running && oldBridge)) && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-border bg-vb-surface-2/60 px-4 py-2.5">
              <p className="text-[12px] text-vb-silver">
                Новый мост{versions?.bundled ? ` ${versions.bundled}` : ""} — стабильнее при сбоях прокси
              </p>
              <Button variant="secondary" size="sm" onClick={handleBridgeUpdate} disabled={updatingBridge}>
                {updatingBridge && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Обновить
              </Button>
            </div>
          )}
          {task !== null && !task.registered && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] px-4 py-2.5">
              <p className="text-[12px] text-vb-warn">Мост не запустится сам после перезагрузки</p>
              <Button variant="secondary" size="sm" onClick={handleTaskRegister} disabled={taskBusy}>
                {taskBusy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                Исправить
              </Button>
            </div>
          )}
          {updateError && <p className="text-[12px] text-vb-loss">{updateError}</p>}
        </motion.section>
      )}

      <UpdateBanner compact />

      {health && (
        <p className="mt-4 text-center font-mono text-[10px] text-vb-silver-faint/70">
          мост {health.version}
        </p>
      )}
    </motion.div>
  );
}

/** Вердикт проверки приватности: одна строка на пункт, ✓ или ⚠. */
function PrivacyRows({ report }: { report: PrivacyReport }) {
  const rows: { ok: boolean; title: string; sub?: string }[] = [];
  if (report.error && !report.exit) {
    rows.push({ ok: false, title: "Проверка не удалась", sub: report.error });
  } else {
    rows.push({
      ok: report.ip_hidden,
      title: report.ip_hidden ? "Через мост ваш IP скрыт" : "Через мост виден ваш настоящий IP",
      sub: report.exit
        ? `Видят ${report.exit.ip}${report.exit.country ? ` · ${report.exit.country}` : ""}${
            report.home ? ` — настоящий ${report.home.ip}` : ""
          }`
        : undefined,
    });
    const dnsOrg = [...new Set(report.dns.map((d) => d.org).filter(Boolean))].slice(0, 2).join(", ");
    rows.push({
      ok: !report.dns_leak,
      title: report.dns_leak ? "DNS выдаёт вашего провайдера" : "DNS не выдаёт провайдера",
      sub: dnsOrg || undefined,
    });
  }
  rows.push({
    ok: report.leaking_apps.length === 0,
    title:
      report.leaking_apps.length === 0
        ? "Приложения из вашего списка — через мост"
        : `Мимо моста: ${report.leaking_apps.join(", ")}`,
    // Мост — не VPN: остальные программы компьютера ходят напрямую.
    sub:
      report.leaking_apps.length > 0
        ? "Подробности — в «Приложениях»"
        : "Остальные программы работают напрямую, как обычно",
  });
  const allOk = rows.every((r) => r.ok);
  return (
    <div className="px-4 py-3">
      <div className={cn("mb-2 text-[13px] font-medium", allOk ? "text-vb-emerald" : "text-vb-warn")}>
        {allOk ? "Всё чисто" : "Есть что поправить"}
      </div>
      <ul className="space-y-1.5">
        {rows.map((r) => (
          <li key={r.title} className="flex items-start gap-2">
            {r.ok ? (
              <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-vb-emerald" />
            ) : (
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-vb-warn" />
            )}
            <span className="min-w-0">
              <span className="block text-[12px] text-vb-fg">{r.title}</span>
              {r.sub && <span className="block truncate text-[11px] text-vb-silver-faint">{r.sub}</span>}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
