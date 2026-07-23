import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  ArrowUpDown,
  Check,
  ChevronDown,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  RotateCw,
  Trash2,
} from "lucide-react";
import { Button } from "../components/Button";
import { InfoTip } from "../components/InfoTip";
import { CountryFlag } from "../components/CountryFlag";
import {
  bridgeUpdate,
  checkProxy,
  proxiesGet,
  proxiesSet,
  proxyAssignmentsGet,
  proxyAssignmentsSet,
  proxyLabelsGet,
  proxyLabelSet,
} from "../lib/api";
import { ServiceLogo } from "../components/BrandLogos";
import { isParseReject, parseProxyInput, type ParsedProxy } from "../lib/proxyParse";
import { useStatus } from "../lib/status";
import type { ProxyCheckResult, ServiceGroup } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

/** Сервисы для назначений (mirror tasks::SERVICE_GROUPS в Rust). */
const SERVICES: { id: ServiceGroup; name: string; sub: string }[] = [
  { id: "anthropic", name: "Claude", sub: "Anthropic · claude.ai" },
  { id: "openai", name: "ChatGPT", sub: "OpenAI · Codex" },
  { id: "google", name: "Gemini", sub: "Google AI" },
  { id: "other_ai", name: "Остальные AI", sub: "Copilot · Cursor · Grok…" },
];


const MAX_POOL = 5;
/** Кэш гео-проверок в localStorage: флаг и страна видны всегда, не только
    после ручного пинга (переживает перезапуски приложения).
    v2: ключи — host:port (логин-пароль в localStorage не пишем). */
const GEO_CACHE_KEY = "clod-proxy-geo-v2";

/** host:port из URL пула (без схемы и логина-пароля — эстетика и ключ кэша). */
function hostPort(url: string): string {
  const rest = url.includes("://") ? url.split("://")[1] : url;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

function loadGeoCache(): Record<string, ProxyCheckResult> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(GEO_CACHE_KEY) ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v)
      ? (v as Record<string, ProxyCheckResult>)
      : {};
  } catch {
    return {};
  }
}

function saveGeoCache(cache: Record<string, ProxyCheckResult>) {
  try {
    localStorage.setItem(GEO_CACHE_KEY, JSON.stringify(cache));
  } catch {
    /* ignore */
  }
}

/* ── Кастомный пикер прокси (нативный <select> выбивался из стиля) ── */
interface SelectProps {
  value: number; // -1 = Авто
  pool: string[];
  labels: Record<string, string>;
  disabled?: boolean;
  onChange: (v: number) => void;
}

function ProxySelect({ value, pool, labels, disabled, onChange }: SelectProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  const currentLabel =
    value < 0 || !pool[value]
      ? "Авто"
      : labels[pool[value]] || hostPort(pool[value]);

  const items = [
    { v: -1, title: "Авто", sub: "основной, при падении — резерв" },
    ...pool.map((url, i) => ({
      v: i,
      title: labels[url] || hostPort(url),
      sub: `строго этот · IP не меняется${labels[url] ? ` · ${hostPort(url)}` : ""}`,
    })),
  ];

  return (
    <div className="relative shrink-0" ref={ref}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex min-w-[150px] items-center justify-between gap-2 rounded-lg border border-vb-border bg-vb-surface-2 px-2.5 py-1.5 text-left text-[12px] text-vb-silver",
          "transition-colors duration-150 hover:border-vb-border-strong active:scale-[0.98]",
          "disabled:cursor-not-allowed disabled:opacity-40",
          open && "border-vb-emerald/50",
        )}
      >
        <span className="truncate font-medium">{currentLabel}</span>
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 opacity-60 transition-transform duration-200",
            open && "rotate-180",
          )}
        />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, scale: 0.97, y: -4 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.98, y: -2 }}
            transition={{ duration: 0.14, ease: [0.22, 1, 0.36, 1] }}
            style={{ transformOrigin: "top right" }}
            className="z-dropdown absolute right-0 top-full mt-1 w-56 overflow-hidden rounded-lg border border-vb-border bg-vb-surface shadow-[0_12px_32px_rgba(0,0,0,0.55)]"
          >
            {items.map((it) => (
              <button
                key={it.v}
                type="button"
                onClick={() => {
                  setOpen(false);
                  if (it.v !== value) onChange(it.v);
                }}
                className={cn(
                  "flex w-full items-center gap-2.5 px-3 py-2 text-left transition-colors hover:bg-vb-surface-2",
                  it.v === value && "bg-vb-surface-2/60",
                )}
              >
                <div className="min-w-0 flex-1">
                  <div
                    className={cn(
                      "truncate text-[13px] font-medium",
                      it.v === value ? "text-vb-fg" : "text-vb-silver",
                    )}
                  >
                    {it.title}
                  </div>
                  {it.sub && (
                    <div className="truncate font-mono text-[10px] text-vb-silver-faint">
                      {it.sub}
                    </div>
                  )}
                </div>
                {it.v === value && (
                  <Check className="h-3.5 w-3.5 shrink-0 text-vb-emerald" />
                )}
              </button>
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/*
  Прокси: пул до 5 адресов с тэгами, живым пингом и постоянным гео (флаг +
  страна из кэша), умное добавление любого формата с эскалацией, назначения
  «сервис → прокси» через кастомный пикер, удаление с подтверждением.
*/
export function Proxies() {
  const { health, poolPing, refresh, refreshPoolPing } = useStatus();

  const [pool, setPool] = useState<string[] | null>(null);
  const [labels, setLabels] = useState<Record<string, string>>({});
  const [assignments, setAssignments] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [adding, setAdding] = useState(false);
  const [addInput, setAddInput] = useState("");
  const [swapped, setSwapped] = useState(false);

  const [geo, setGeo] = useState<Record<string, ProxyCheckResult>>(loadGeoCache);
  const [geoBusy, setGeoBusy] = useState<Record<string, boolean>>({});

  const [editingTag, setEditingTag] = useState<number | null>(null);
  const [tagInput, setTagInput] = useState("");

  const [confirmDel, setConfirmDel] = useState<number | null>(null);
  const confirmTimer = useRef<number | null>(null);

  const [updatingBridge, setUpdatingBridge] = useState(false);

  const load = useCallback(async () => {
    setPool(await proxiesGet().catch(() => []));
    setLabels(await proxyLabelsGet().catch(() => ({})));
    setAssignments(await proxyAssignmentsGet().catch(() => ({})));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Гео автоматом для записей без кэша — флаг виден всегда, не только
  // после ручной проверки.
  const geoCheck = useCallback(
    async (url: string) => {
      const key = hostPort(url);
      setGeoBusy((b) => ({ ...b, [key]: true }));
      try {
        const r = await checkProxy(url);
        setGeo((old) => {
          const next = { ...old, [key]: r };
          // Провал (гео-API недоступен, прокси моргнул) навсегда не кэшируем —
          // иначе флаг «не появится никогда». В кэш — только удачные ответы,
          // с прунингом по текущему пулу (записи удалённых прокси не копятся).
          if (r.reachable || r.country_code) {
            const keep = new Set((pool ?? []).map(hostPort));
            keep.add(key);
            saveGeoCache(
              Object.fromEntries(Object.entries(next).filter(([k]) => keep.has(k))),
            );
          }
          return next;
        });
      } finally {
        setGeoBusy((b) => ({ ...b, [key]: false }));
      }
    },
    [pool],
  );

  useEffect(() => {
    if (!pool) return;
    for (const url of pool) {
      const key = hostPort(url);
      if (!geo[key] && !geoBusy[key]) void geoCheck(url);
    }
    // geo/geoBusy намеренно не в deps: цикл сам себя не перезапускает
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pool, geoCheck]);

  // ── Умный парсинг ввода ──
  const parsedResult = useMemo(() => parseProxyInput(addInput), [addInput]);
  const parseError = isParseReject(parsedResult) ? parsedResult.error : null;
  const parsedBase: ParsedProxy | null = isParseReject(parsedResult) ? null : parsedResult;
  const parsed: ParsedProxy | null =
    parsedBase && swapped && parsedBase.alt ? parsedBase.alt : parsedBase;
  const ambiguous = !!parsedBase?.alt;
  /** Жёлтая панель: есть alt ИЛИ парсер не уверен (низкая уверенность). */
  const needsReview = ambiguous || parsed?.confidence === "low";

  const applyPool = async (next: string[], busyKey: string): Promise<boolean> => {
    setBusy(busyKey);
    setConfirmDel(null);
    setError(null);
    try {
      setPool(await proxiesSet(next));
      await refresh();
      await refreshPoolPing();
      // Бэкенд перемапил индексы назначений по URL — перечитываем их состояние.
      setAssignments(await proxyAssignmentsGet().catch(() => ({})));
      setLabels(await proxyLabelsGet().catch(() => ({})));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const handleAdd = async () => {
    if (!parsed || !pool) return;
    if (pool.includes(parsed.url)) {
      setError("Этот прокси уже в пуле");
      return;
    }
    // Ввод чистим только при успехе — иначе пользователь перенабирает строку.
    if (await applyPool([...pool, parsed.url], "add")) {
      setAddInput("");
      setSwapped(false);
      setAdding(false);
    }
  };

  const askDelete = (i: number) => {
    setConfirmDel(i);
    if (confirmTimer.current) window.clearTimeout(confirmTimer.current);
    confirmTimer.current = window.setTimeout(() => setConfirmDel(null), 4000);
  };

  const handleRemove = async (i: number) => {
    if (!pool || pool.length <= 1) return;
    setConfirmDel(null);
    await applyPool(pool.filter((_, k) => k !== i), `del:${i}`);
  };

  const handleMakePrimary = async (i: number) => {
    if (!pool || i === 0) return;
    const next = [pool[i], ...pool.filter((_, k) => k !== i)];
    await applyPool(next, `primary:${i}`);
  };

  const handleAssign = async (group: ServiceGroup, value: number) => {
    const next = { ...assignments };
    if (value < 0) delete next[group];
    else next[group] = value;
    setAssignments(next);
    setBusy(`assign:${group}`);
    setError(null);
    try {
      await proxyAssignmentsSet(next);
      await refresh();
    } catch (e) {
      setError(String(e));
      setAssignments(await proxyAssignmentsGet().catch(() => ({})));
    } finally {
      setBusy(null);
    }
  };

  const startTagEdit = (i: number, url: string) => {
    setEditingTag(i);
    setTagInput(labels[url] ?? "");
  };

  const saveTag = async (url: string) => {
    setEditingTag(null);
    try {
      setLabels(await proxyLabelSet(url, tagInput));
    } catch {
      /* ignore */
    }
  };

  const handleBridgeUpdate = async () => {
    setUpdatingBridge(true);
    setError(null);
    try {
      await bridgeUpdate();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setUpdatingBridge(false);
    }
  };

  const pingFor = (url: string) => poolPing.find((p) => p.url === url)?.ms;
  /** Назначения честно строгие (закрепление сильнее режима, без failover)
      только с моста 1.2.0 — старее гейтим, чтобы UI не врал про «IP не меняется». */
  const bridgeSupportsAssignments =
    health !== null &&
    health.version.localeCompare("1.2.0", undefined, { numeric: true }) >= 0;

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[620px] flex-col px-8 py-7"
    >
      <motion.header variants={fadeInUp} className="flex items-end justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-[24px] font-bold leading-tight tracking-[-0.02em] text-vb-fg">
            Прокси
            <InfoTip>
              Пул до {MAX_POOL} прокси-серверов. <b>Основной</b> используется по
              умолчанию; если он не отвечает — мост автоматически переключается
              на запасные. Ниже можно закрепить сервисы за конкретными прокси.
            </InfoTip>
          </h1>
          <p className="mt-0.5 text-[13px] text-vb-silver-dim">
            Основной, запасные и назначения по сервисам
          </p>
        </div>
        {pool !== null && pool.length < MAX_POOL && (
          <Button variant="secondary" size="sm" onClick={() => setAdding((v) => !v)}>
            <Plus className="h-3.5 w-3.5" />
            Добавить
          </Button>
        )}
      </motion.header>

      {/* ── Добавление: умный парсер любого формата ── */}
      <AnimatePresence>
        {adding && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="surface-card mt-4 p-4">
              <input
                type="text"
                value={addInput}
                onChange={(e) => {
                  setAddInput(e.target.value);
                  setSwapped(false);
                }}
                placeholder="Вставьте прокси в любом формате: ip:порт:логин:пароль…"
                className="w-full rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 font-mono text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                autoFocus
              />

              {addInput.trim() &&
                (parsed ? (
                  <div
                    className={cn(
                      "mt-2.5 rounded-lg border p-3",
                      needsReview
                        ? "border-vb-warn/35 bg-vb-warn/[0.05]"
                        : "border-vb-emerald/25 bg-vb-emerald/[0.04]",
                    )}
                  >
                    {needsReview && (
                      <p className="mb-2 text-[12px] text-vb-warn">
                        {ambiguous
                          ? "Формат неоднозначный — проверьте, где хост, а где логин:"
                          : "Не до конца уверен в разборе — проверьте поля:"}
                      </p>
                    )}
                    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px]">
                      <span className="text-vb-silver-dim">
                        Хост <span className="font-mono text-vb-fg">{parsed.host}</span>
                      </span>
                      <span className="text-vb-silver-dim">
                        Порт <span className="tnum font-mono text-vb-fg">{parsed.port}</span>
                      </span>
                      {parsed.username !== undefined && (
                        <>
                          <span className="text-vb-silver-dim">
                            Логин{" "}
                            <span className="font-mono text-vb-fg">{parsed.username}</span>
                          </span>
                          <span className="text-vb-silver-dim">
                            Пароль{" "}
                            {parsed.password ? (
                              <span className="font-mono text-vb-fg">····</span>
                            ) : (
                              <span className="text-vb-warn">пусто</span>
                            )}
                          </span>
                        </>
                      )}
                    </div>
                    {ambiguous && (
                      <button
                        type="button"
                        onClick={() => setSwapped((v) => !v)}
                        className="mt-2 flex items-center gap-1.5 rounded-lg border border-vb-border px-2.5 py-1 text-[12px] font-medium text-vb-silver transition-colors hover:border-vb-border-strong hover:bg-vb-surface-2 active:scale-[0.97]"
                      >
                        <ArrowUpDown className="h-3 w-3" />
                        Поменять местами
                      </button>
                    )}
                  </div>
                ) : parseError ? (
                  <p className="mt-2 text-[12px] text-vb-warn">{parseError}</p>
                ) : (
                  <p className="mt-2 text-[12px] text-vb-loss">
                    Не получилось разобрать. Примеры:{" "}
                    <span className="font-mono">1.2.3.4:8000:login:pass</span>,{" "}
                    <span className="font-mono">login:pass@1.2.3.4:8000</span>
                  </p>
                ))}

              <div className="mt-3 flex items-center gap-2">
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleAdd}
                  disabled={!parsed || busy !== null}
                >
                  {busy === "add" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  {busy === "add" ? "Добавляю…" : "Добавить в пул"}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setAdding(false);
                    setAddInput("");
                  }}
                  disabled={busy === "add"}
                >
                  Отмена
                </Button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {error && (
        <motion.p variants={fadeInUp} className="mt-3 text-[12px] text-vb-loss">
          {error}
        </motion.p>
      )}

      {/* ── Пул ── */}
      <motion.section variants={fadeInUp} className="mt-5">
        {pool === null ? (
          <div className="surface-card flex items-center justify-center gap-2 p-8 text-[13px] text-vb-silver-dim">
            <Loader2 className="h-4 w-4 animate-spin" />
            Загружаю…
          </div>
        ) : pool.length === 0 ? (
          <div className="surface-card p-8 text-center text-[13px] text-vb-silver-dim">
            Прокси ещё не добавлены.
          </div>
        ) : (
          <div className="surface-card divide-y divide-vb-border/70">
            {pool.map((url, i) => {
              const ms = pingFor(url);
              const dot =
                ms === undefined
                  ? "bg-vb-silver-faint"
                  : ms === null
                    ? "bg-vb-loss"
                    : ms > 1500
                      ? "bg-vb-warn"
                      : "bg-vb-emerald";
              const gkey = hostPort(url);
              const g = geo[gkey];
              const gBusy = geoBusy[gkey];
              const tag = labels[url];
              // Назначенные на этот прокси сервисы: предупреждаем при удалении —
              // их закрепление снимется (уйдут в «Авто»).
              const pinnedHere = SERVICES.filter((s) => assignments[s.id] === i).map(
                (s) => s.name,
              );
              return (
                <div key={url} className="flex items-center gap-3 px-4 py-3">
                  <span className={cn("h-2 w-2 shrink-0 rounded-full", dot)} />
                  <div className="min-w-0 flex-1">
                    {editingTag === i ? (
                      <input
                        type="text"
                        value={tagInput}
                        onChange={(e) => setTagInput(e.target.value)}
                        onBlur={() => void saveTag(url)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void saveTag(url);
                          if (e.key === "Escape") setEditingTag(null);
                        }}
                        placeholder="Имя прокси (например, Немецкий)"
                        className="w-full max-w-[220px] rounded-md border border-vb-emerald/50 bg-vb-surface-2 px-2 py-0.5 text-[13px] text-vb-fg outline-none"
                        autoFocus
                      />
                    ) : (
                      <div className="flex items-center gap-2">
                        <span
                          className={cn(
                            "truncate text-[13px] text-vb-fg",
                            tag ? "font-semibold" : "font-mono",
                          )}
                        >
                          {tag || hostPort(url)}
                        </span>
                        {i === 0 && (
                          <span className="shrink-0 rounded-md border border-vb-emerald/30 bg-vb-emerald/[0.07] px-1.5 py-0.5 text-[10px] font-medium text-vb-emerald">
                            основной
                          </span>
                        )}
                        <button
                          type="button"
                          onClick={() => startTagEdit(i, url)}
                          className="shrink-0 rounded-md p-1 text-vb-silver-faint opacity-60 transition-all hover:bg-vb-surface-2 hover:text-vb-silver hover:opacity-100"
                          title={tag ? "Переименовать" : "Дать имя"}
                        >
                          <Pencil className="h-3 w-3" />
                        </button>
                      </div>
                    )}
                    <div className="tnum mt-px flex items-center gap-2 text-[11px] text-vb-silver-faint">
                      {tag && <span className="font-mono">{hostPort(url)}</span>}
                      <span>
                        {ms === undefined ? "пинг…" : ms === null ? (
                          <span className="text-vb-loss">не отвечает</span>
                        ) : (
                          `${ms} мс`
                        )}
                      </span>
                      {/* Флаг + страна — видны ВСЕГДА (кэш гео) */}
                      {gBusy && !g ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : g?.country_code ? (
                        <span className="inline-flex items-center gap-1">
                          <CountryFlag code={g.country_code} />
                          {g.country_name ?? g.country_code}
                        </span>
                      ) : null}
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center gap-1">
                    {confirmDel === i ? (
                      /* Подтверждение удаления — обязательное */
                      <div className="flex items-center gap-1.5 rounded-lg border border-vb-loss/40 bg-vb-loss/[0.06] px-2 py-1">
                        <span className="text-[12px] font-medium text-vb-loss">
                          {pinnedHere.length
                            ? `Удалить? ${pinnedHere.join(", ")} → Авто`
                            : "Удалить?"}
                        </span>
                        <button
                          type="button"
                          onClick={() => void handleRemove(i)}
                          disabled={busy !== null}
                          className="rounded-md bg-vb-loss px-2 py-0.5 text-[11px] font-semibold text-white transition-colors hover:bg-vb-loss/85 active:scale-[0.96] disabled:opacity-40"
                        >
                          Да
                        </button>
                        <button
                          type="button"
                          onClick={() => setConfirmDel(null)}
                          className="rounded-md px-1.5 py-0.5 text-[11px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
                        >
                          Отмена
                        </button>
                      </div>
                    ) : (
                      <>
                        {i !== 0 && (
                          <button
                            type="button"
                            onClick={() => handleMakePrimary(i)}
                            disabled={busy !== null}
                            className="rounded-lg px-2.5 py-1.5 text-[12px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.97] disabled:opacity-35"
                          >
                            {busy === `primary:${i}` ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              "Сделать основным"
                            )}
                          </button>
                        )}
                        <button
                          type="button"
                          onClick={() => void geoCheck(url)}
                          disabled={busy !== null || gBusy}
                          className="rounded-lg p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.97] disabled:opacity-35"
                          title="Перепроверить гео и доступность"
                        >
                          {gBusy ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <RefreshCw className="h-3.5 w-3.5" />
                          )}
                        </button>
                        {pool.length > 1 && (
                          <button
                            type="button"
                            onClick={() => askDelete(i)}
                            disabled={busy !== null}
                            className="rounded-lg p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-loss/10 hover:text-vb-loss active:scale-[0.97] disabled:opacity-35"
                            title="Убрать из пула"
                          >
                            {busy === `del:${i}` ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Trash2 className="h-3.5 w-3.5" />
                            )}
                          </button>
                        )}
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        <p className="mt-2 px-1 text-[11px] text-vb-silver-faint">
          Изменение пула перезапускает мост (~2 секунды). Пинг обновляется
          каждые 5 минут, гео — кнопкой.
        </p>
      </motion.section>

      {/* ── Назначения по сервисам ── */}
      <motion.section variants={fadeInUp} className="mt-6">
        <div className="mb-2 flex items-center gap-2 px-1">
          <h2 className="text-[15px] font-semibold text-vb-fg">Назначения</h2>
          <InfoTip>
            Каждый AI-сервис можно закрепить за конкретным прокси из пула.
            Закрепление <b>строгое</b>: сервис ходит только через выбранный
            прокси — если тот лёг, сервис ждёт его восстановления, а НЕ
            прыгает на другой IP (AI-сервисы, особенно Claude, флагают
            аккаунты за смену IP). «Авто» — основной прокси, при его падении
            разрешён переход на резервные.
          </InfoTip>
        </div>

        {!bridgeSupportsAssignments && health !== null && (
          <div className="mb-3 flex items-center justify-between gap-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] p-3">
            <p className="text-[12px] leading-snug text-vb-warn">
              Для строгих назначений мосту нужна версия 1.2.0+ (сейчас {health.version}).
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
              Обновить мост
            </Button>
          </div>
        )}

        <div className="surface-card divide-y divide-vb-border/70">
          {SERVICES.map(({ id, name, sub }) => {
            const value = assignments[id] ?? -1;
            const rowBusy = busy === `assign:${id}`;
            return (
              <div key={id} className="flex items-center gap-3 px-4 py-3">
                <ServiceLogo id={id} />
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] font-semibold text-vb-fg">{name}</div>
                  <div className="mt-px text-[11px] text-vb-silver-faint">{sub}</div>
                </div>
                {rowBusy && (
                  <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-dim" />
                )}
                <ProxySelect
                  value={value}
                  pool={pool ?? []}
                  labels={labels}
                  // Старый мост назначения игнорирует — активный контрол врал бы,
                  // что закрепление работает (см. плашку «Обновить мост» выше).
                  disabled={
                    busy !== null ||
                    pool === null ||
                    pool.length === 0 ||
                    !bridgeSupportsAssignments
                  }
                  onChange={(v) => void handleAssign(id, v)}
                />
              </div>
            );
          })}
        </div>
      </motion.section>
    </motion.div>
  );
}
