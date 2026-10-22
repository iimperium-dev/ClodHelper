import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Loader2, Plus, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "./Button";
import { CountryFlag } from "./CountryFlag";
import { ProxyInputField, useProxyInput } from "./ProxyInput";
import { checkProxy } from "../lib/api";
import { cn } from "../lib/cn";
import type { ProxyState } from "../lib/status";
import type { ProxyCheckResult } from "../lib/types";
import { CopyIp, hostOnly, hostPort } from "../pages/Proxies";

/** Сколько резервов можно привязать к одному прокси (как в Rust). */
export const MAX_BACKUPS = 5;

interface Props {
  /** Прокси пула, к которому привязаны резервы. */
  parent: string;
  /** Его индекс (в пуле и в списке моста — совпадают). */
  parentIndex: number;
  backups: string[];
  /** Состояния всех прокси моста (пул и резервы). */
  states: ProxyState[];
  geo: Record<string, ProxyCheckResult>;
  geoBusy: Record<string, boolean>;
  onRecheck: (url: string) => void;
  /** Гео нового прокси, проверенного перед добавлением, — в общий кэш. */
  onGeo: (url: string, r: ProxyCheckResult) => void;
  /** Кто работает через этот прокси (сервисы ИИ, закреплённые приложения). */
  users: string[];
  /** Все адреса пула и резервов — дубли не добавляем. */
  existing: string[];
  busy: boolean;
  onSave: (backups: string[]) => Promise<boolean>;
}

interface Warning {
  /** loss — опасно (другая страна, прокси мёртв), warn — будьте аккуратны. */
  tone: "loss" | "warn";
  title: React.ReactNode;
  text: string;
}

const dotOf = (s?: ProxyState) =>
  s?.state === "down"
    ? "bg-vb-loss"
    : s?.state === "unstable" || s?.state === "slow"
      ? "bg-vb-warn"
      : s?.state === "ok"
        ? "bg-vb-emerald"
        : "bg-vb-silver-faint";

/** Флаг и страна одной строкой. */
function Country({ g }: { g?: ProxyCheckResult }) {
  if (!g?.country_code) return null;
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5">
      <CountryFlag code={g.country_code} />
      {g.country_name ?? g.country_code}
    </span>
  );
}

/*
  Резервы прокси — «веточка» под строкой прокси: если он упадёт, всё, что
  через него шло (закреплённые сервисы, сайты, приложения), подхватит
  первый живой резерв по порядку, а когда прокси оживёт — трафик вернётся
  на него. Добавление — тем же полем, что и в пул, с проверкой до
  сохранения и предупреждением, если через прокси работают сервисы ИИ:
  смена IP (а тем более страны) для них опасна.
*/
export function ProxyBackups({
  parent,
  parentIndex,
  backups,
  states,
  geo,
  geoBusy,
  onRecheck,
  onGeo,
  users,
  existing,
  busy,
  onSave,
}: Props) {
  const [adding, setAdding] = useState(false);
  const input = useProxyInput();
  const [phase, setPhase] = useState<"input" | "checking" | "confirm" | "saving">("input");
  const [warnings, setWarnings] = useState<Warning[]>([]);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState<string | null>(null);

  const parentState = states[parentIndex];
  const parentGeo = geo[hostPort(parent)];

  const close = () => {
    setAdding(false);
    setPhase("input");
    setWarnings([]);
    setPending(null);
    setError(null);
    input.reset();
  };

  const save = async (url: string) => {
    setPhase("saving");
    if (await onSave([...backups, url])) close();
    else setPhase("input");
  };

  /** Проверить прокси до сохранения: живой ли, какая страна, кого заденет. */
  const submit = async () => {
    const url = input.parsed?.url;
    if (!url) return;
    setError(null);
    if (existing.includes(url)) {
      setError("Этот прокси уже есть в списке");
      return;
    }
    setPhase("checking");
    const r = await checkProxy(url).catch(() => null);
    if (r?.country_code) onGeo(url, r);
    const list: Warning[] = [];
    if (!r?.reachable) {
      list.push({
        tone: "loss",
        title: "Прокси не ответил",
        text: `${r?.error ?? "Проверка не прошла"}. Нерабочий резерв не подстрахует — проверьте адрес и данные.`,
      });
    }
    if (users.length) {
      const who = users.slice(0, 4).join(", ") + (users.length > 4 ? "…" : "");
      const from = parentGeo?.country_code?.toLowerCase();
      const to = r?.country_code?.toLowerCase();
      if (from && to && from !== to) {
        list.push({
          tone: "loss",
          title: (
            <span className="inline-flex flex-wrap items-center gap-1.5">
              Другая страна: <Country g={parentGeo} /> → <Country g={r ?? undefined} />
            </span>
          ),
          text: `Через этот прокси работают ${who}. Если он упадёт, они продолжат с IP другой страны — Claude и другие сервисы ИИ считают такие скачки подозрительными и могут заблокировать аккаунт. Берите резерв из той же страны.`,
        });
      } else {
        list.push({
          tone: "warn",
          title: from && to ? (
            <span className="inline-flex flex-wrap items-center gap-1.5">
              Та же страна: <Country g={parentGeo} />
            </span>
          ) : (
            "Страну не удалось сравнить"
          ),
          text: `Через этот прокси работают ${who}. При переключении на резерв у них сменится IP — сервисы ИИ к этому чувствительны. ${
            from && to
              ? "Страна та же — так безопаснее всего; прокси других стран для них не добавляйте."
              : "Убедитесь, что резерв из той же страны, что и основной."
          }`,
        });
      }
    }
    if (!list.length) {
      await save(url);
      return;
    }
    setWarnings(list);
    setPending(url);
    setPhase("confirm");
  };

  const danger = warnings.some((w) => w.tone === "loss");

  return (
    <div className="relative pb-3 pl-[34px] pr-4">
      {/* Ствол «веточки»: от точки прокси вниз, к резервам и плюсику */}
      <span aria-hidden className="absolute bottom-[19px] left-[19px] top-0 w-px bg-vb-border-strong/80" />

      {backups.map((url, k) => {
        const j = states.findIndex((s) => s.url === url);
        const st = j >= 0 ? states[j] : undefined;
        const serving = j >= 0 && parentState?.coveredBy === j;
        const g = geo[hostPort(url)];
        const host = hostOnly(url);
        const live = st?.state;
        return (
          <div key={url} className="group/b relative flex items-center gap-2.5 rounded-lg px-2.5 py-2 transition-colors hover:bg-vb-surface-2/60">
            {/* Отвод от ствола */}
            <span aria-hidden className="absolute -left-[15px] top-1/2 h-px w-[15px] bg-vb-border-strong/80" />
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotOf(st))} />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-1.5">
                <span className="tnum shrink-0 text-[10px] font-medium text-vb-silver-faint">№{k + 1}</span>
                <CopyIp text={host} copy={host} className="text-[12px] text-vb-fg" />
                {serving && (
                  <span className="shrink-0 rounded-md border border-vb-warn/35 bg-vb-warn/[0.08] px-1.5 py-[3px] text-[10px] font-medium leading-none text-vb-warn">
                    работает вместо основного
                  </span>
                )}
              </div>
              <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint">
                {geoBusy[hostPort(url)] && !g ? <Loader2 className="h-3 w-3 animate-spin" /> : <Country g={g} />}
                {live === "down" ? (
                  <span className="truncate text-vb-loss">
                    {g?.country_code && "· "}не работает{st?.reason ? `: ${st.reason.charAt(0).toLowerCase()}${st.reason.slice(1)}` : ""}
                  </span>
                ) : (
                  typeof st?.ms === "number" && (
                    <span className={cn("tnum font-mono", live === "slow" || live === "unstable" ? "text-vb-warn" : "")}>
                      {g?.country_code && "· "}
                      {st.ms} мс
                    </span>
                  )
                )}
              </div>
            </div>
            {confirmDel === url ? (
              <div className="flex shrink-0 items-center gap-1.5 rounded-lg border border-vb-loss/40 bg-vb-loss/[0.06] px-2 py-1">
                <span className="text-[11px] font-medium text-vb-loss">Убрать резерв?</span>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    setConfirmDel(null);
                    void onSave(backups.filter((b) => b !== url));
                  }}
                  className="rounded-md bg-vb-loss px-2 py-0.5 text-[11px] font-semibold text-white transition-colors hover:bg-vb-loss/85 disabled:opacity-40"
                >
                  Да
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmDel(null)}
                  className="rounded-md px-1.5 py-0.5 text-[11px] font-medium text-vb-silver-dim hover:bg-vb-surface-2 hover:text-vb-silver"
                >
                  Отмена
                </button>
              </div>
            ) : (
              <div className="flex shrink-0 items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover/b:opacity-100">
                <button
                  type="button"
                  onClick={() => onRecheck(url)}
                  disabled={busy || geoBusy[hostPort(url)]}
                  title="Перепроверить гео и доступность"
                  className="rounded-lg p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver disabled:opacity-35"
                >
                  {geoBusy[hostPort(url)] ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmDel(url)}
                  disabled={busy}
                  title="Убрать резерв"
                  className="rounded-lg p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-loss/10 hover:text-vb-loss disabled:opacity-35"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            )}
          </div>
        );
      })}

      <AnimatePresence initial={false} mode="wait">
        {adding ? (
          <motion.div
            key="form"
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="relative overflow-hidden"
          >
            <span aria-hidden className="absolute -left-[15px] top-[19px] h-px w-[15px] bg-vb-border-strong/80" />
            <div className="mt-1 rounded-xl border border-vb-border bg-vb-surface-2/40 p-3">
              <div className="mb-2 text-[11px] font-medium text-vb-silver">Резервный прокси</div>
              {phase === "confirm" ? (
                <div className="space-y-2">
                  {warnings.map((w, k) => (
                    <div
                      key={k}
                      className={cn(
                        "rounded-lg border p-2.5",
                        w.tone === "loss" ? "border-vb-loss/40 bg-vb-loss/[0.06]" : "border-vb-warn/35 bg-vb-warn/[0.06]",
                      )}
                    >
                      <div
                        className={cn(
                          "flex items-center gap-1.5 text-[12px] font-medium",
                          w.tone === "loss" ? "text-vb-loss" : "text-vb-warn",
                        )}
                      >
                        <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                        {w.title}
                      </div>
                      <p className="mt-1 text-[11px] leading-snug text-vb-silver">{w.text}</p>
                    </div>
                  ))}
                </div>
              ) : (
                <ProxyInputField input={input} autoFocus compact onEnter={() => void submit()} />
              )}
              {error && <p className="mt-2 text-[12px] text-vb-loss">{error}</p>}
              <div className="mt-3 flex items-center gap-2">
                {phase === "confirm" ? (
                  <Button
                    variant={danger ? "danger" : "primary"}
                    size="sm"
                    onClick={() => pending && void save(pending)}
                  >
                    {danger ? "Всё равно добавить" : "Добавить резерв"}
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() => void submit()}
                    disabled={!input.parsed || phase !== "input" || busy}
                  >
                    {phase !== "input" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                    {phase === "checking" ? "Проверяю…" : phase === "saving" ? "Добавляю…" : "Добавить"}
                  </Button>
                )}
                <Button variant="ghost" size="sm" onClick={close} disabled={phase === "saving"}>
                  Отмена
                </Button>
              </div>
            </div>
          </motion.div>
        ) : (
          backups.length < MAX_BACKUPS && (
            <motion.button
              key="plus"
              type="button"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={() => setAdding(true)}
              disabled={busy}
              aria-label="Добавить резервный прокси"
              className="group/add relative mt-0.5 flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-[12px] text-vb-silver-faint transition-colors hover:text-vb-emerald disabled:opacity-40"
            >
              <span aria-hidden className="absolute -left-[15px] top-1/2 h-px w-[15px] bg-vb-border-strong/80" />
              <span className="flex h-5 w-5 items-center justify-center rounded-full border border-dashed border-vb-border-strong transition-colors group-hover/add:border-vb-emerald/60 group-hover/add:bg-vb-emerald/10">
                <Plus className="h-3 w-3" />
              </span>
              {/* Подсказка раскрывается при наведении на плюсик */}
              <span className="max-w-0 overflow-hidden whitespace-nowrap opacity-0 transition-all duration-200 group-hover/add:max-w-[220px] group-hover/add:opacity-100 group-focus-visible/add:max-w-[220px] group-focus-visible/add:opacity-100">
                {backups.length ? "Добавить ещё резервный прокси" : "Добавить резервный прокси"}
              </span>
            </motion.button>
          )
        )}
      </AnimatePresence>
    </div>
  );
}
