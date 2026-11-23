import { useEffect, useState, type ReactNode } from "react";
import { AnimatePresence, animate, motion, useMotionValue, useReducedMotion, useTransform } from "framer-motion";
import { ChevronDown, Gauge, Loader2, Pencil, RefreshCw, Star, Trash2 } from "lucide-react";
import { CountryFlag } from "./CountryFlag";
import { CopyIp } from "../pages/Proxies";
import type { ProxyLive } from "../lib/status";
import { cn } from "../lib/cn";
import { causeLabel, causeOf, isAuthReason } from "../lib/proxyCause";
import type { ProxyState } from "../lib/status";

/* ───────────────────────── Модель строки ───────────────────────── */

export type SpeedEntry = { mbps: number; at: number } | { error: string; at: number };

/** Всё, что нужно нарисовать один прокси пула. Считает экран «Прокси». */
export interface PoolRow {
  url: string;
  i: number;
  primary: boolean;
  /** Имя (тэг) прокси; нет - показываем IP. */
  tag?: string;
  /** IP (с портом, если в пуле тот же IP на разных портах). */
  shown: string;
  host: string;
  countryCode?: string;
  countryName?: string;
  geoBusy: boolean;
  ms?: number | null;
  live: ProxyLive;
  reason?: string;
  /** Не работает, но его подменяет резерв (хост резерва). */
  coverHost?: string;
  speed?: SpeedEntry;
  speedBusy: boolean;
  backups: number;
  open: boolean;
  /** Ждёт подтверждения удаления: текст вопроса. */
  confirm?: string;
  /** Куплен в магазине Clod Helper: срок («до 15 декабря») и истёк ли он. */
  shop?: { until: string | null; expired: boolean };
  /** Предупреждение о поломке скрыто (на «Обзоре» кнопкой «Скрыть»). */
  muted?: boolean;
}

export interface PoolActions {
  busy: string | null;
  toggle: (url: string) => void;
  speed: (url: string) => void;
  recheck: (url: string) => void;
  askDelete: (i: number) => void;
  remove: (i: number) => void;
  cancelDelete: () => void;
  makePrimary: (i: number) => void;
  /** «Продлить» прокси из магазина: кабинет покупателя в браузере. */
  renew: (url: string) => void;
  editing: number | null;
  tagInput: string;
  setTagInput: (v: string) => void;
  startEdit: (i: number, url: string) => void;
  saveTag: (url: string) => void;
  cancelEdit: () => void;
  /** Вставить новый логин и пароль для прокси. */
  replace: (url: string) => void;
  /** Панель резервов под прокси (раскрытие - по `open`). */
  backupsPanel: (row: PoolRow) => ReactNode;
  /** Снова предупреждать о поломке прокси. */
  unmute: (url: string) => void;
}

/* ───────────────────────── Общие кусочки ───────────────────────── */

const dotClass = (r: PoolRow) =>
  r.live === "down"
    ? r.coverHost
      ? "bg-vb-warn"
      : "bg-vb-loss"
    : r.live === "unstable" || r.live === "slow"
      ? "bg-vb-warn"
      : r.live === "ok"
        ? "bg-vb-emerald"
        : "bg-vb-silver-faint";

/** Точка состояния: зелёная - работает, жёлтая - есть вопросы, красная -
    не работает. При смене цвета - короткая «волна», чтобы смена была заметна. */
function StatusDot({ row, size = 8 }: { row: PoolRow; size?: number }) {
  const cls = dotClass(row);
  return (
    <span className="relative inline-flex shrink-0" style={{ width: size, height: size }}>
      <motion.span
        key={cls}
        initial={{ scale: 1, opacity: 0.55 }}
        animate={{ scale: 2.6, opacity: 0 }}
        transition={{ duration: 0.9, ease: "easeOut" }}
        className={cn("absolute inset-0 rounded-full", cls)}
      />
      <span className={cn("relative h-full w-full rounded-full transition-colors duration-500", cls)} />
    </span>
  );
}

const statusTitle = (r: PoolRow) =>
  r.live === "down" ? (r.coverHost ? "Не работает - подменяет резерв" : "Не работает") : r.live === "unstable" ? "Сбоит" : r.live === "slow" ? "Медленно отвечает" : r.live === "ok" ? "Работает" : "Проверяю";

/** Причина «не работает / сбоит» - одной тихой строкой под именем; полный
    текст - в подсказке и в раскрытой строке. Цвет говорит точка слева. */
function Problem({ row, a }: { row: PoolRow; a: PoolActions }) {
  // Срок прокси из магазина прошёл, и он не отвечает - дело в сроке, а не в
  // логине и пароле.
  if (row.shop?.expired && row.live === "down" && !row.coverHost) {
    return (
      <div className="mt-1 flex min-w-0 items-center gap-2 text-[11px] leading-snug">
        <span className="truncate text-vb-loss">Срок закончился</span>
        <RenewBtn row={row} a={a} />
      </div>
    );
  }
  // Свёрнуто причина видна справа, вместо цифр; раскрыто - здесь, одной
  // понятной фразой с советом.
  if (!row.open || !(row.live === "down" || row.live === "unstable") || !row.reason) return null;
  const auth = isAuthReason(row.reason);
  const text =
    causeOf({ state: row.live, reason: row.reason } as ProxyState, { canAddSpare: false }).text +
    (row.coverHost ? ` Пока работает резерв ${row.coverHost}.` : "");
  return (
    <>
      <div className="mt-2 max-w-[46ch] text-[11px] leading-snug text-vb-silver-dim">{text}</div>
      {row.open && auth && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            a.replace(row.url);
          }}
          className="mt-1 text-[11px] text-vb-silver underline decoration-vb-silver-faint/60 underline-offset-2 transition-colors hover:text-vb-fg"
        >
          Заменить пароль
        </button>
      )}
    </>
  );
}

function RenewBtn({ row, a }: { row: PoolRow; a: PoolActions }) {
  return (
    <button
      type="button"
      onClick={() => a.renew(row.url)}
      className="shrink-0 rounded font-medium text-vb-emerald transition-colors hover:text-vb-emerald-bright"
    >
      Продлить
    </button>
  );
}

/** «Из магазина · до 15 декабря» - строкой под именем. */
function ShopLine({ row, a }: { row: PoolRow; a: PoolActions }) {
  if (!row.shop || (row.shop.expired && row.live === "down" && !row.coverHost)) return null;
  return (
    <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint">
      <span>Из магазина</span>
      {row.shop.until && (
        <>
          <span aria-hidden="true">·</span>
          <span className={cn(row.shop.expired && "text-vb-warn")}>{row.shop.expired ? "срок закончился" : row.shop.until}</span>
        </>
      )}
      {row.shop.expired && (
        <>
          <span aria-hidden="true">·</span>
          <RenewBtn row={row} a={a} />
        </>
      )}
    </div>
  );
}

/** «Предупреждение скрыто · Вернуть» - строкой под именем. Метка видна,
    пока прокси не работает: заработает - она снимется сама. */
export function MutedLine({ onUnmute, className }: { onUnmute: () => void; className?: string }) {
  return (
    <div
      data-testid="proxy-muted"
      title="На «Обзоре» и в статусе этот прокси не считается поломкой. Проверки идут как раньше"
      className={cn("mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint", className)}
    >
      <span>Предупреждение скрыто</span>
      <span aria-hidden="true">·</span>
      <button
        type="button"
        onClick={onUnmute}
        className="shrink-0 rounded text-vb-silver-dim transition-colors hover:text-vb-silver"
      >
        Вернуть
      </button>
    </div>
  );
}

/** Число, которое «докручивается» до значения (как спидометр). */
function CountUp({ value, format, className }: { value: number; format: (v: number) => string; className?: string }) {
  const reduced = useReducedMotion();
  const mv = useMotionValue(reduced ? value : 0);
  const text = useTransform(mv, format);
  useEffect(() => {
    if (reduced) {
      mv.set(value);
      return;
    }
    const c = animate(mv, value, { duration: 1.1, ease: [0.16, 1, 0.3, 1] });
    return () => c.stop();
  }, [value, reduced, mv]);
  return <motion.span className={className}>{text}</motion.span>;
}

/** 4,2 · 37 - дробь только у медленных. */
export const formatMbps = (v: number) => (v < 10 ? v.toFixed(1).replace(".", ",") : String(Math.round(v)));

/** «5 мин назад» для подсказки у скорости. */
export function ago(at: number): string {
  const m = Math.round((Date.now() - at) / 60_000);
  if (m < 1) return "только что";
  if (m < 60) return `${m} мин назад`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} ч назад`;
  return `${Math.round(h / 24)} дн назад`;
}

const SPEED_STEPS = [1, 5, 15, 40, 100];
const FAIL_SHOW_MS = 20_000;
const speedLevel = (mbps: number) => SPEED_STEPS.filter((t) => mbps >= t).length;

/** Мини-индикатор скорости: 5 столбиков. Во время замера бегут волной,
    после - загораются по уровню, а число докручивается до результата. */
export function SpeedMeter({ entry, busy, showLabel = true }: { entry?: SpeedEntry; busy: boolean; showLabel?: boolean }) {
  const reduced = useReducedMotion();
  const ok = entry && "mbps" in entry ? entry : undefined;
  // Неудачный замер видно 20 секунд (причина - в подсказке), потом строка
  // возвращается к «скорость -»: висеть вечно ему незачем.
  const failed = entry && "error" in entry ? entry : undefined;
  const failShown = !!failed && Date.now() - failed.at < FAIL_SHOW_MS;
  // Перерисовать, когда 20 секунд выйдут.
  const [, expire] = useState(0);
  useEffect(() => {
    if (!failed) return;
    const left = FAIL_SHOW_MS - (Date.now() - failed.at);
    if (left <= 0) return;
    const t = window.setTimeout(() => expire((x) => x + 1), left);
    return () => window.clearTimeout(t);
  }, [failed]);
  if (!busy && failed && !failShown) {
    return showLabel ? <span className="text-vb-silver-faint/70">скорость -</span> : null;
  }
  const level = ok ? speedLevel(ok.mbps) : 0;
  const slow = !!ok && ok.mbps < 2;
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1.5"
      title={ok ? `Скорость скачивания · ${ago(ok.at)}` : entry && "error" in entry ? entry.error : undefined}
    >
      <span className="flex h-3 items-end gap-[2px]" aria-hidden>
        {SPEED_STEPS.map((_, k) => (
          <motion.span
            key={k}
            className={cn(
              "w-[3px] origin-bottom rounded-[1px]",
              busy ? "bg-vb-emerald/70" : k < level ? (slow ? "bg-vb-warn" : "bg-vb-emerald") : "bg-vb-silver-faint/35",
            )}
            style={{ height: 4 + k * 2 }}
            animate={
              busy && !reduced
                ? { scaleY: [0.3, 1, 0.3] }
                : { scaleY: 1 }
            }
            transition={
              busy && !reduced
                ? { duration: 0.9, repeat: Infinity, ease: "easeInOut", delay: k * 0.09 }
                : { type: "spring", bounce: 0.5, duration: 0.5, delay: k * 0.06 }
            }
          />
        ))}
      </span>
      {showLabel &&
        (busy ? (
          <span className="text-vb-silver-dim">замеряю…</span>
        ) : ok ? (
          <span className={cn("tnum font-mono", slow ? "text-vb-warn" : "text-vb-silver-dim")}>
            <CountUp key={ok.at} value={ok.mbps} format={formatMbps} /> Мбит/с
          </span>
        ) : failed ? (
          <span className="text-vb-warn" title={failed.error}>не удалось замерить</span>
        ) : null)}
    </span>
  );
}

/** Имя прокси: тэг или IP; карандаш при наведении; правка на месте. */
function NameField({ row, a, big }: { row: PoolRow; a: PoolActions; big?: boolean }) {
  const size = big ? "text-[17px]" : "text-[13px]";
  if (a.editing === row.i) {
    return (
      <input
        type="text"
        value={a.tagInput}
        onChange={(e) => a.setTagInput(e.target.value)}
        onBlur={() => a.saveTag(row.url)}
        onKeyDown={(e) => {
          if (e.key === "Enter") a.saveTag(row.url);
          if (e.key === "Escape") a.cancelEdit();
        }}
        placeholder="Имя прокси (например, Немецкий)"
        className={cn(
          "w-full max-w-[240px] rounded-md border border-vb-emerald/50 bg-vb-surface-2 px-2 py-0.5 text-vb-fg outline-none",
          size,
        )}
        autoFocus
      />
    );
  }
  return (
    <button
      type="button"
      onClick={() => a.startEdit(row.i, row.url)}
      title={row.tag ? "Переименовать" : "Дать имя"}
      className="group/name flex min-w-0 items-center rounded-md text-left"
    >
      <span className={cn("truncate font-semibold text-vb-fg", size, !row.tag && "font-mono font-medium")}>
        {row.tag || row.shown}
      </span>
      <Pencil className="ml-0 h-3 w-0 shrink-0 text-vb-silver-faint opacity-0 transition-all group-hover/name:ml-1.5 group-hover/name:w-3 group-hover/name:opacity-100 group-focus-visible/name:ml-1.5 group-focus-visible/name:w-3 group-focus-visible/name:opacity-100" />
    </button>
  );
}

/** Страна и IP (IP - только если имя задано, иначе он уже в заголовке). */
function Where({ row, ipToo = true }: { row: PoolRow; ipToo?: boolean }) {
  return (
    <>
      {row.geoBusy && !row.countryCode ? (
        <Loader2 className="h-3 w-3 animate-spin" />
      ) : row.countryCode ? (
        <span className="inline-flex shrink-0 items-center gap-1.5">
          <CountryFlag code={row.countryCode} />
          {row.countryName ?? row.countryCode}
        </span>
      ) : null}
      {ipToo && row.tag && (
        <>
          {(row.countryCode || row.geoBusy) && <span aria-hidden="true">·</span>}
          <CopyIp text={row.shown} copy={row.host} />
        </>
      )}
    </>
  );
}

const pingText = (r: PoolRow) => (r.live === "down" ? "-" : typeof r.ms === "number" ? `${r.ms} мс` : "…");
const pingWarn = (r: PoolRow) => r.live === "slow" || r.live === "unstable";

/** Подтверждение удаления - выезжает полосой под прокси. */
function ConfirmStrip({ row, a }: { row: PoolRow; a: PoolActions }) {
  return (
    <AnimatePresence initial={false}>
      {row.confirm && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
          className="overflow-hidden"
        >
          <div className="mx-3 mb-3 flex items-center gap-2 rounded-xl border border-vb-loss/35 bg-vb-loss/[0.06] px-3 py-2">
            <span className="min-w-0 flex-1 text-[12px] leading-snug text-vb-loss">{row.confirm}</span>
            <button
              type="button"
              onClick={() => a.cancelDelete()}
              className="rounded-full px-2.5 py-1 text-[12px] font-medium text-vb-silver-dim transition-colors hover:bg-vb-surface-2 hover:text-vb-silver"
            >
              Отмена
            </button>
            <button
              type="button"
              onClick={() => a.remove(row.i)}
              disabled={a.busy !== null}
              className="rounded-full bg-vb-loss px-3 py-1 text-[12px] font-semibold text-white transition-colors hover:bg-vb-loss/85 active:scale-[0.96] disabled:opacity-40"
            >
              {a.busy === `del:${row.i}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Удалить"}
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function Backups({ row, a }: { row: PoolRow; a: PoolActions }) {
  return (
    <AnimatePresence initial={false}>
      {row.open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1, transition: { height: { duration: 0.32, ease: [0.32, 0.72, 0, 1] }, opacity: { duration: 0.2 } } }}
          exit={{ height: 0, opacity: 0, transition: { height: { duration: 0.22, ease: [0.4, 0, 0.2, 1] }, opacity: { duration: 0.14 } } }}
          className="overflow-hidden"
        >
          {a.backupsPanel(row)}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

const backupsLabel = (n: number) => `${n} ${n === 1 ? "резерв" : n < 5 ? "резерва" : "резервов"}`;

/** Клик по пустому месту строки - раскрыть резервы (кнопки и поля - мимо). */
const rowClick = (a: PoolActions, url: string) => (e: React.MouseEvent) => {
  if ((e.target as HTMLElement).closest("button, input, a")) return;
  a.toggle(url);
};

function IconBtn({ title, onClick, disabled, children, danger }: { title: string; onClick: () => void; disabled?: boolean; children: ReactNode; danger?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={title}
      className={cn(
        "rounded-lg p-1.5 text-vb-silver-faint transition-colors active:scale-[0.95] disabled:opacity-35",
        danger ? "hover:bg-vb-loss/10 hover:text-vb-loss" : "hover:bg-vb-surface-2 hover:text-vb-silver",
      )}
    >
      {children}
    </button>
  );
}

/* ─────────────── Список прокси ───────────────
   Основной - отдельной группой, остальные - списком ниже (без плашки).
   В строке слева имя и страна, справа колонка цифр: пинг над скоростью.
   При наведении цифры уступают место кнопкам - на узком окне ничего не
   наезжает. Нажали «Замерить скорость» - кнопки сразу уходят и снова видны
   цифры с замером; кнопки вернутся, когда курсор уйдёт и придёт снова. */

function RowA({ row, a }: { row: PoolRow; a: PoolActions }) {
  const [holdNumbers, setHoldNumbers] = useState(false);
  const numbers = holdNumbers || row.speedBusy;
  return (
    <div className={cn("transition-colors duration-200", row.open && "bg-vb-surface-2/25")}>
      <div
        role="button"
        tabIndex={0}
        aria-expanded={row.open}
        onClick={rowClick(a, row.url)}
        onMouseLeave={() => !row.speedBusy && setHoldNumbers(false)}
        onKeyDown={(e) => {
          if ((e.key === "Enter" || e.key === " ") && e.target === e.currentTarget) {
            e.preventDefault();
            a.toggle(row.url);
          }
        }}
        className="group flex cursor-pointer items-center gap-3 px-4 py-3 outline-none transition-colors hover:bg-vb-surface-2/40 focus-visible:bg-vb-surface-2/40"
      >
        <span title={statusTitle(row)}>
          <StatusDot row={row} />
        </span>
        <div className="min-w-0 flex-1">
          <NameField row={row} a={a} />
          <ShopLine row={row} a={a} />
          <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[11px] leading-none text-vb-silver-faint">
            <Where row={row} />
            {row.backups > 0 && !(row.live === "down" && row.coverHost) && (
              <span className={cn("ml-1 shrink-0", row.coverHost ? "text-vb-warn" : "")}>
                · {row.coverHost ? "работает резерв" : backupsLabel(row.backups)}
              </span>
            )}
          </div>
          {row.muted && row.live === "down" && <MutedLine onUnmute={() => a.unmute(row.url)} />}
          <Problem row={row} a={a} />
        </div>
        {/* Цифры ↔ кнопки: один и тот же слот, сменяются при наведении */}
        <div className="relative flex h-9 w-[132px] shrink-0 items-center justify-end">
          <div
            className={cn(
              "flex flex-col items-end gap-1 text-[11px] leading-none transition-[opacity,transform] duration-200",
              !numbers &&
                "group-focus-within:-translate-y-0.5 group-focus-within:opacity-0 group-hover:-translate-y-0.5 group-hover:opacity-0",
            )}
          >
            {row.live === "down" && !(row.shop?.expired && !row.coverHost) ? (
              // Не работает: вместо прочерков - что случилось.
              <>
                {/* Тихо, в размер цифр соседних строк: точка слева уже красная. */}
                <span className="text-vb-loss/75">{causeLabel(row.reason).toLowerCase()}</span>
                {row.coverHost && <span className="text-vb-warn">работает резерв</span>}
              </>
            ) : (
              <>
                <span className={cn("tnum font-mono", pingWarn(row) ? "text-vb-warn" : row.live === "down" ? "text-vb-loss" : "text-vb-silver")}>
                  {pingText(row)}
                </span>
                {row.live === "unstable" ? (
                  <span className="text-vb-warn">сбоит</span>
                ) : row.speed || row.speedBusy ? (
                  <SpeedMeter entry={row.speed} busy={row.speedBusy} />
                ) : (
                  <span className="text-vb-silver-faint/70">скорость -</span>
                )}
              </>
            )}
          </div>
          <div
            className={cn(
              "pointer-events-none absolute inset-y-0 right-0 flex translate-y-0.5 items-center opacity-0 transition-[opacity,transform] duration-200",
              !numbers &&
                "group-focus-within:pointer-events-auto group-focus-within:translate-y-0 group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:translate-y-0 group-hover:opacity-100",
            )}
          >
            <IconBtn
              title="Замерить скорость"
              onClick={() => {
                setHoldNumbers(true);
                (document.activeElement as HTMLElement | null)?.blur();
                a.speed(row.url);
              }} disabled={a.busy !== null || row.speedBusy || row.live === "down"}>
              {row.speedBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Gauge className="h-3.5 w-3.5" />}
            </IconBtn>
            <IconBtn title="Проверить снова" onClick={() => a.recheck(row.url)} disabled={a.busy !== null || row.geoBusy}>
              {row.geoBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            </IconBtn>
            {row.primary ? (
              <span title="Основной прокси" className="p-1.5 text-vb-emerald">
                <Star className="h-3.5 w-3.5 fill-current" />
              </span>
            ) : (
              <IconBtn title="Сделать основным" onClick={() => a.makePrimary(row.i)} disabled={a.busy !== null}>
                {a.busy === `primary:${row.i}` ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Star className="h-3.5 w-3.5" />
                )}
              </IconBtn>
            )}
            <IconBtn title="Удалить" danger onClick={() => a.askDelete(row.i)} disabled={a.busy !== null}>
              <Trash2 className="h-3.5 w-3.5" />
            </IconBtn>
          </div>
        </div>
        <ChevronDown aria-hidden className={cn("h-3.5 w-3.5 shrink-0 text-vb-silver-faint transition-transform duration-200", row.open && "rotate-180")} />
      </div>
      <ConfirmStrip row={row} a={a} />
      <Backups row={row} a={a} />
    </div>
  );
}

function GroupLabel({ children }: { children: ReactNode }) {
  return <div className="mb-1.5 mt-4 px-1 text-[11px] font-medium text-vb-silver-faint first:mt-0">{children}</div>;
}

function VariantA({ rows, a }: { rows: PoolRow[]; a: PoolActions }) {
  const [main, ...rest] = rows;
  return (
    <div>
      <GroupLabel>Основной</GroupLabel>
      <div className="surface-card overflow-hidden">
        <RowA row={main} a={a} />
      </div>
      {rest.length > 0 && (
        <>
          <GroupLabel>Другие прокси</GroupLabel>
          <div className="surface-card divide-y divide-vb-border/70 overflow-hidden">
            {rest.map((r) => (
              <RowA key={r.url} row={r} a={a} />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

export function ProxyPoolView({ rows, a }: { rows: PoolRow[]; a: PoolActions }) {
  if (rows.length === 0) return null;
  return <VariantA rows={rows} a={a} />;
}
