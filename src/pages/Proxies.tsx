import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Check,
  ChevronDown,
  Copy,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { Button } from "../components/Button";
import { CountryFlag } from "../components/CountryFlag";
import {
  appsList,
  checkProxy,
  proxyBackupsGet,
  proxyBackupsSet,
  proxiesGet,
  proxiesSet,
  proxyAssignmentsGet,
  proxyLabelsGet,
  proxyLabelSet,
} from "../lib/api";
import { ProxyInputField, useProxyInput } from "../components/ProxyInput";
import { ProxyBackups } from "../components/ProxyBackups";
import { BridgeFileNotice } from "../components/BridgeFileNotice";
import { useStatus } from "../lib/status";
import type { AppProfileStatus, ProxyCheckResult, ServiceGroup } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { cn } from "../lib/cn";

/** Сервисы для назначений (mirror tasks::SERVICE_GROUPS в Rust). */
const SERVICES: { id: ServiceGroup; name: string; sub: string }[] = [
  { id: "anthropic", name: "Claude", sub: "Anthropic · claude.ai" },
  { id: "openai", name: "ChatGPT", sub: "OpenAI · Codex" },
  { id: "google", name: "Gemini", sub: "Google AI" },
  { id: "other_ai", name: "Остальные AI", sub: "Copilot · Cursor · Grok…" },
  // Telegram ходит через SOCKS-вход моста (macOS и Windows).
  { id: "telegram", name: "Telegram", sub: "Через мост · «Авто» = основной прокси" },
];


const MAX_POOL = 5;
/** Кэш гео-проверок в localStorage: флаг и страна видны всегда, не только
    после ручного пинга (переживает перезапуски приложения).
    v2: ключи — host:port (логин-пароль в localStorage не пишем). */
const GEO_CACHE_KEY = "clod-proxy-geo-v2";

/** host:port из URL пула (без схемы и логина-пароля — эстетика и ключ кэша). */
export function hostPort(url: string): string {
  const rest = url.includes("://") ? url.split("://")[1] : url;
  return rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest;
}

/** Хост прокси без порта — так его показываем и копируем. */
export function hostOnly(url: string): string {
  const hp = hostPort(url);
  const colon = hp.lastIndexOf(":");
  return colon > 0 ? hp.slice(0, colon) : hp;
}

export function loadGeoCache(): Record<string, ProxyCheckResult> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(GEO_CACHE_KEY) ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v)
      ? (v as Record<string, ProxyCheckResult>)
      : {};
  } catch {
    return {};
  }
}

export function saveGeoCache(cache: Record<string, ProxyCheckResult>) {
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

export function ProxySelect({ value, pool, labels, disabled, onChange }: SelectProps) {
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
  const { proxies, refresh, refreshPoolPing, bridgeFile } = useStatus();

  const [pool, setPool] = useState<string[] | null>(null);
  const [labels, setLabels] = useState<Record<string, string>>({});
  const [assignments, setAssignments] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [adding, setAdding] = useState(false);
  const addInput = useProxyInput();
  // Резервы прокси (URL прокси → резервы) и раскрытые строки пула.
  const [backups, setBackups] = useState<Record<string, string[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  // Закреплённые за прокси приложения — для предупреждения о резервах.
  const [apps, setApps] = useState<AppProfileStatus[]>([]);

  const [geo, setGeo] = useState<Record<string, ProxyCheckResult>>(loadGeoCache);
  const [geoBusy, setGeoBusy] = useState<Record<string, boolean>>({});

  const [editingTag, setEditingTag] = useState<number | null>(null);
  const [tagInput, setTagInput] = useState("");

  const [confirmDel, setConfirmDel] = useState<number | null>(null);
  const confirmTimer = useRef<number | null>(null);

  const load = useCallback(async () => {
    setPool(await proxiesGet().catch(() => []));
    setLabels(await proxyLabelsGet().catch(() => ({})));
    setAssignments(await proxyAssignmentsGet().catch(() => ({})));
    setBackups(await proxyBackupsGet().catch(() => ({})));
    setApps(await appsList().catch(() => []));
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
        // Заодно — настоящая проверка всего пула: состояние (точка, пинг,
        // «не работает») обновляется вместе с гео, а не через 5 минут.
        const [r] = await Promise.all([checkProxy(url), refreshPoolPing()]);
        if (!r) return; // нет ответа — не роняем страницу, просто без флага
        // Прокси не ответил — страна от этого не поменялась: флаг из кэша
        // оставляем, а «не работает» покажет состояние прокси.
        if (!r.reachable && !r.country_code) return;
        setGeo((old) => {
          const next = { ...old, [key]: r };
          // Провал (гео-API недоступен, прокси моргнул) навсегда не кэшируем —
          // иначе флаг «не появится никогда». В кэш — только удачные ответы,
          // с прунингом по текущему пулу (записи удалённых прокси не копятся).
          if (r.reachable || r.country_code) {
            const keep = new Set([...(pool ?? []), ...Object.values(backups).flat()].map(hostPort));
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
    [pool, backups],
  );

  // Гео нового резерва (проверенного перед добавлением) — сразу в кэш.
  const rememberGeo = (url: string, r: ProxyCheckResult) =>
    setGeo((old) => {
      const next = { ...old, [hostPort(url)]: r };
      saveGeoCache(next);
      return next;
    });

  useEffect(() => {
    if (!pool) return;
    for (const url of [...pool, ...Object.values(backups).flat()]) {
      const key = hostPort(url);
      if (!geo[key] && !geoBusy[key]) void geoCheck(url);
    }
    // geo/geoBusy намеренно не в deps: цикл сам себя не перезапускает
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pool, backups, geoCheck]);

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
      // Резервы едут за своим прокси, удалённый уносит свои — перечитываем.
      setBackups(await proxyBackupsGet().catch(() => ({})));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const handleAdd = async () => {
    const parsed = addInput.parsed;
    if (!parsed || !pool) return;
    if (pool.includes(parsed.url)) {
      setError("Этот прокси уже в пуле");
      return;
    }
    // Ввод чистим только при успехе — иначе пользователь перенабирает строку.
    if (await applyPool([...pool, parsed.url], "add")) {
      addInput.reset();
      setAdding(false);
    }
  };

  /** Сохранить резервы прокси: мост перезапускается с новым списком. */
  const saveBackups = async (parent: string, list: string[]): Promise<boolean> => {
    setBusy(`bk:${parent}`);
    setError(null);
    try {
      setBackups(await proxyBackupsSet(parent, list));
      await refresh();
      await refreshPoolPing();
      setLabels(await proxyLabelsGet().catch(() => ({})));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(null);
    }
  };

  /** Кто работает через прокси №i: сервисы ИИ и закреплённые приложения. */
  const usersOf = (i: number): string[] => {
    const names = SERVICES.filter((s) => s.id !== "telegram" && assignments[s.id] === i).map((s) => s.name);
    // Claude без назначения в «весь трафик» и Claude Desktop — на основном.
    if (i === 0 && assignments.anthropic === undefined) names.push("Claude");
    for (const a of apps) if (!a.hidden && a.proxy === i) names.push(a.name);
    return [...new Set(names)];
  };

  const toggleExpanded = (url: string) =>
    setExpanded((old) => {
      const next = new Set(old);
      if (next.has(url)) next.delete(url);
      else next.add(url);
      return next;
    });

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


  const stateFor = (url: string) => proxies.find((p) => p.url === url);


  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[520px] flex-col px-6 py-6"
    >
      <motion.header variants={fadeInUp} className="flex items-end justify-between">
        <div>
          <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">
            Прокси
          </h1>
          <p className="text-[12px] text-vb-silver-faint">
            Основной работает всегда, запасные — если он упал. Резервы — по клику на прокси
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
              <ProxyInputField input={addInput} autoFocus onEnter={() => void handleAdd()} />

              <div className="mt-3 flex items-center gap-2">
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleAdd}
                  disabled={!addInput.parsed || busy !== null}
                >
                  {busy === "add" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  {busy === "add" ? "Добавляю…" : "Добавить в пул"}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setAdding(false);
                    addInput.reset();
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

      {/* Антивирус удалил мост — чаще всего всплывает здесь, при добавлении
          прокси (мост перезапускается). Блок объясняет и ведёт по шагам —
          ту же ошибку отдельной красной строкой не дублируем. */}
      <BridgeFileNotice className="mt-4" />

      {error && !(bridgeFile && bridgeFile.kind !== "restored" && /Антивирус/.test(error)) && (
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
              const st = stateFor(url);
              const ms = st?.ms;
              const live = st?.state ?? "unknown";
              const dot =
                live === "down"
                  ? st?.coveredBy !== undefined
                    ? "bg-vb-warn" // упал, но подменён живым резервом
                    : "bg-vb-loss"
                  : live === "unstable" || live === "slow"
                    ? "bg-vb-warn"
                    : live === "ok"
                      ? "bg-vb-emerald"
                      : "bg-vb-silver-faint";
              const gkey = hostPort(url);
              const host = hostOnly(url);
              const shown = pool.filter((u) => hostOnly(u) === host).length > 1 ? gkey : host;
              const g = geo[gkey];
              const gBusy = geoBusy[gkey];
              const tag = labels[url];
              // Назначенные на этот прокси сервисы: предупреждаем при удалении —
              // их закрепление снимется (уйдут в «Авто»).
              const pinnedHere = SERVICES.filter((s) => assignments[s.id] === i).map(
                (s) => s.name,
              );
              const myBackups = backups[url] ?? [];
              const open = expanded.has(url);
              const cover = st?.coveredBy !== undefined ? proxies[st.coveredBy] : undefined;
              return (
                <div key={url} className={cn("transition-colors duration-200", open && "bg-vb-surface-2/25")}>
                <div
                  role="button"
                  tabIndex={0}
                  aria-expanded={open}
                  title={open ? undefined : "Резервные прокси"}
                  // Клик по строке (не по кнопкам и полям) — раскрыть резервы.
                  onClick={(e) => {
                    if ((e.target as HTMLElement).closest("button, input, a")) return;
                    toggleExpanded(url);
                  }}
                  onKeyDown={(e) => {
                    if ((e.key === "Enter" || e.key === " ") && e.target === e.currentTarget) {
                      e.preventDefault();
                      toggleExpanded(url);
                    }
                  }}
                  className="group flex cursor-pointer items-center gap-3 px-4 py-3 outline-none transition-colors duration-150 hover:bg-vb-surface-2/40 focus-visible:bg-vb-surface-2/40"
                >
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
                      <div className="flex min-w-0 items-center gap-1.5">
                        {/* Имя: карандаш — только при наведении на него */}
                        {tag ? (
                          <button
                            type="button"
                            onClick={() => startTagEdit(i, url)}
                            title="Переименовать"
                            className="group/name flex min-w-0 items-center rounded-md text-left"
                          >
                            <span className="truncate text-[13px] font-semibold text-vb-fg">{tag}</span>
                            {/* В покое места не занимает — раскрывается при наведении */}
                            <Pencil className="ml-0 h-3 w-0 shrink-0 text-vb-silver-faint opacity-0 transition-all group-hover/name:ml-1.5 group-hover/name:w-3 group-hover/name:opacity-100 group-focus-visible/name:ml-1.5 group-focus-visible/name:w-3 group-focus-visible/name:opacity-100" />
                          </button>
                        ) : (
                          <>
                            <CopyIp text={shown} copy={host} className="text-[13px] text-vb-fg" />
                            <button
                              type="button"
                              onClick={() => startTagEdit(i, url)}
                              title="Дать имя"
                              aria-label="Дать имя"
                              className="shrink-0 rounded-md p-0.5 text-vb-silver-faint opacity-0 transition-opacity hover:text-vb-silver focus-visible:opacity-100 group-hover:opacity-100"
                            >
                              <Pencil className="h-3 w-3" />
                            </button>
                          </>
                        )}
                        {/* «основной» — у основного; у остальных на его месте при
                            наведении — «сделать основным» тем же бейджем */}
                        {i === 0 ? (
                          <span className="shrink-0 rounded-md border border-vb-emerald/30 bg-vb-emerald/[0.07] px-1.5 py-[3px] text-[10px] font-medium leading-none text-vb-emerald">
                            основной
                          </span>
                        ) : (
                          <button
                            type="button"
                            onClick={() => handleMakePrimary(i)}
                            disabled={busy !== null}
                            className="shrink-0 rounded-md border border-dashed border-vb-border px-1.5 py-[3px] text-[10px] font-medium leading-none text-vb-silver-dim opacity-0 transition-[opacity,color,border-color] hover:border-vb-emerald/40 hover:text-vb-emerald focus-visible:opacity-100 group-hover:opacity-100 disabled:pointer-events-none"
                          >
                            {busy === `primary:${i}` ? "…" : "сделать основным"}
                          </button>
                        )}
                      </div>
                    )}
                    {/* Не работает / сбоит — причина прямо в строке, не молча */}
                    {(live === "down" || live === "unstable") && st?.reason && (
                      <div
                        className={cn(
                          "mt-1 text-[11px] leading-snug",
                          live === "down" && !cover ? "text-vb-loss" : "text-vb-warn",
                        )}
                      >
                        {live === "down" ? "Не работает: " : "Сбоит: "}
                        {st.reason.charAt(0).toLowerCase() + st.reason.slice(1)}
                        {cover && ` — подменяет резерв ${cover.host}`}
                      </div>
                    )}
                    {/* Флаг и страна (кэш гео) · IP — копируется по клику · пинг · резервы */}
                    {(tag || g?.country_code || gBusy || typeof ms === "number" || myBackups.length > 0) && (
                      <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint">
                        {gBusy && !g ? (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        ) : g?.country_code ? (
                          <span className="inline-flex shrink-0 items-center gap-1.5">
                            <CountryFlag code={g.country_code} />
                            {g.country_name ?? g.country_code}
                          </span>
                        ) : null}
                        {tag && (g?.country_code || gBusy) && <span aria-hidden="true">·</span>}
                        {tag && <CopyIp text={shown} copy={host} />}
                        {/* Пинг — рядом с IP; у неработающего его нет: врать нельзя */}
                        {live !== "down" && typeof ms === "number" && (
                          <>
                            {(tag || g?.country_code) && <span aria-hidden="true">·</span>}
                            <span
                              className={cn(
                                "tnum font-mono",
                                live === "slow" || live === "unstable" ? "text-vb-warn" : "text-vb-silver-dim",
                              )}
                            >
                              {ms} мс
                            </span>
                          </>
                        )}
                        {myBackups.length > 0 && (
                          <span
                            className={cn(
                              "ml-1 shrink-0 rounded-md border px-1.5 py-[3px] text-[10px] font-medium leading-none",
                              cover
                                ? "border-vb-warn/35 bg-vb-warn/[0.08] text-vb-warn"
                                : "border-vb-border bg-vb-surface-2/60 text-vb-silver-dim",
                            )}
                          >
                            {cover ? "работает резерв" : `+${myBackups.length} ${myBackups.length === 1 ? "резерв" : myBackups.length < 5 ? "резерва" : "резервов"}`}
                          </span>
                        )}
                      </div>
                    )}
                  </div>

                  <div className="flex shrink-0 items-center gap-1">
                    {confirmDel === i ? (
                      /* Подтверждение удаления — обязательное */
                      <div className="flex items-center gap-1.5 rounded-lg border border-vb-loss/40 bg-vb-loss/[0.06] px-2 py-1">
                        <span className="text-[12px] font-medium text-vb-loss">
                          {pinnedHere.length
                            ? `Удалить? ${pinnedHere.join(", ")} → Авто`
                            : "Удалить?"}
                          {myBackups.length > 0 && ` Резервы (${myBackups.length}) — тоже`}
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
                        {/* Действия — при наведении на строку, в покое строка чистая */}
                        <div className="flex items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
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
                        </div>
                      </>
                    )}
                    {/* Раскрыть резервы */}
                    <ChevronDown
                      aria-hidden
                      className={cn(
                        "ml-0.5 h-3.5 w-3.5 shrink-0 text-vb-silver-faint transition-transform duration-200",
                        open && "rotate-180",
                      )}
                    />
                  </div>
                </div>
                {/* Резервы — разъезжаются вниз под строкой */}
                <AnimatePresence initial={false}>
                  {open && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: "auto", opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
                      className="overflow-hidden"
                    >
                      <ProxyBackups
                        parent={url}
                        parentIndex={i}
                        backups={myBackups}
                        states={proxies}
                        geo={geo}
                        geoBusy={geoBusy}
                        onRecheck={(u) => void geoCheck(u)}
                        onGeo={rememberGeo}
                        users={usersOf(i)}
                        existing={[...pool, ...Object.values(backups).flat()]}
                        busy={busy !== null}
                        onSave={(list) => saveBackups(url, list)}
                      />
                    </motion.div>
                  )}
                </AnimatePresence>
                </div>
              );
            })}
          </div>
        )}
      </motion.section>

    </motion.div>
  );
}

/** IP прокси: при наведении — значок копирования, по клику — в буфер обмена. */
export function CopyIp({ text, copy, className }: { text: string; copy: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(copy);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* буфер обмена недоступен — молча */
    }
  };
  return (
    <button
      type="button"
      onClick={(e) => {
        // Внутри кликабельной строки (выбор прокси в настройках приложения)
        // копирование не должно выбирать строку.
        e.stopPropagation();
        void onCopy();
      }}
      title={copied ? "Скопировано" : "Скопировать IP"}
      className={cn(
        "group/ip inline-flex min-w-0 items-center gap-1 rounded font-mono transition-colors hover:text-vb-silver",
        className,
      )}
    >
      <span className="truncate">{text}</span>
      {copied ? (
        <Check className="h-3 w-3 shrink-0 text-vb-emerald" />
      ) : (
        <Copy className="h-3 w-3 shrink-0 opacity-0 transition-opacity group-hover/ip:opacity-100 group-focus-visible/ip:opacity-100" />
      )}
    </button>
  );
}
