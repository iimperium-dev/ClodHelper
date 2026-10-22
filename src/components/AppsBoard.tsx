import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  animate,
  AnimatePresence,
  LayoutGroup,
  motion,
  useMotionValue,
  useReducedMotion,
} from "framer-motion";
import { FolderPlus, Trash2 } from "lucide-react";
import { AppRow, type RowAction, type RowStatus } from "./AppRow";
import { AppTile } from "./AppTile";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { moveApp, newFolderId, type AppsLayout } from "../lib/appLayout";
import { cn } from "../lib/cn";
import type { AppProfileStatus } from "../lib/types";

export interface RowInfo {
  status: RowStatus | null;
  action: RowAction | null;
  menu: RowAction[];
}

interface Props {
  apps: AppProfileStatus[];
  layout: AppsLayout;
  onLayout: (l: AppsLayout) => void;
  editing: boolean;
  onEditing: (v: boolean) => void;
  describe: (app: AppProfileStatus) => RowInfo;
  icons: Record<string, string>;
  onLaunch: (id: string) => Promise<void>;
  onRemove: (id: string) => void;
  onSettings: (id: string) => void;
}

const LONG_PRESS_MS = 450;
const SPRING = { type: "spring", bounce: 0.18, duration: 0.42 } as const;
const LAND = { type: "spring", bounce: 0.2, duration: 0.32 } as const;

/** Нажатие на приложение: ещё не перетаскивание — ждём сдвига или долгого нажатия. */
interface Press {
  id: string;
  x0: number;
  y0: number;
  /** Где внутри элемента схватили — «призрак» держится за ту же точку. */
  offX: number;
  offY: number;
  w: number;
  h: number;
  /** Можно тащить: режим упорядочивания уже включён или сработало долгое нажатие. */
  armed: boolean;
  fired: boolean;
  dragging: boolean;
  timer: number | null;
}

type Target = { beforeApp: string } | { folder: string };

const sameLayout = (a: AppsLayout, b: AppsLayout) =>
  a.folders.length === b.folders.length &&
  a.folders.every((f, i) => f.id === b.folders[i].id && f.apps.join() === b.folders[i].apps.join());

/** Ближайший прокручиваемый предок — для автопрокрутки у краёв при перетаскивании. */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let n = el?.parentElement; n; n = n.parentElement) {
    const oy = getComputedStyle(n).overflowY;
    if ((oy === "auto" || oy === "scroll") && n.scrollHeight > n.clientHeight) return n;
  }
  return null;
}

/*
  Приложения папками — списком или сеткой, с режимом упорядочивания как в
  iOS: долгое нажатие (или правый клик → «Упорядочить») — иконки
  покачиваются; схваченное приложение едет за курсором, а соседи живьём
  расступаются, освобождая место (в том числе в другой папке). Отпустили —
  приложение мягко «садится» в своё новое место. Клик по пустому месту —
  выход из режима.
*/
export function AppsBoard({
  apps,
  layout,
  onLayout,
  editing,
  onEditing,
  describe,
  icons,
  onLaunch,
  onRemove,
  onSettings,
}: Props) {
  const reduced = !!useReducedMotion();
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const byId = new Map(apps.map((a) => [a.id, a]));
  const grid = layout.view === "grid";

  // ── Перетаскивание ──
  const [drag, setDrag] = useState<{ id: string; w: number; h: number; landing: boolean } | null>(null);
  const [preview, setPreview] = useState<AppsLayout | null>(null);
  const shown = preview ?? layout;
  const gx = useMotionValue(0);
  const gy = useMotionValue(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const press = useRef<Press | null>(null);
  const previewRef = useRef<AppsLayout | null>(null);
  const pointer = useRef({ x: 0, y: 0 });
  const scroller = useRef<HTMLElement | null>(null);
  const raf = useRef<number | null>(null);

  // Последние значения — для слушателей окна, живущих дольше одного рендера.
  const latest = useRef({ layout, grid, editing, onLayout, onEditing, apps, describe, onLaunch, reduced });
  latest.current = { layout, grid, editing, onLayout, onEditing, apps, describe, onLaunch, reduced };

  /** Куда встанет приложение, если отпустить в точке (x, y). null — оставить как есть. */
  const targetAt = (x: number, y: number, id: string, cur: AppsLayout): Target | null => {
    for (const el of document.elementsFromPoint(x, y)) {
      const h = el as HTMLElement;
      const app = h.dataset?.dropApp;
      if (app) {
        if (app === id) return null; // над своим же местом — уже на месте
        const f = cur.folders.find((x) => x.apps.includes(app));
        if (!f) return null;
        const r = h.getBoundingClientRect();
        // Сетка — до/после по горизонтали, список — по вертикали.
        const after = latest.current.grid ? x > r.left + r.width / 2 : y > r.top + r.height / 2;
        if (!after) return { beforeApp: app };
        const rest = f.apps.filter((a) => a !== id);
        const next = rest[rest.indexOf(app) + 1];
        return next ? { beforeApp: next } : { folder: f.id };
      }
      const folder = h.dataset?.dropFolder;
      if (folder) {
        // Пустое место своей папки — не дёргаем в конец; чужой — в её конец.
        if (cur.folders.find((f) => f.id === folder)?.apps.includes(id)) return null;
        return { folder };
      }
    }
    return null;
  };

  const retarget = () => {
    const p = press.current;
    const cur = previewRef.current;
    if (!p?.dragging || !cur) return;
    const t = targetAt(pointer.current.x, pointer.current.y, p.id, cur);
    if (!t) return;
    const next = moveApp(cur, p.id, t);
    if (sameLayout(next, cur)) return;
    previewRef.current = next;
    setPreview(next);
  };

  // Автопрокрутка, пока курсор у верхнего/нижнего края области.
  const tick = () => {
    const s = scroller.current;
    if (s && press.current?.dragging) {
      const r = s.getBoundingClientRect();
      const y = pointer.current.y;
      const edge = 56;
      const v = y < r.top + edge ? -(r.top + edge - y) / 4 : y > r.bottom - edge ? (y - (r.bottom - edge)) / 4 : 0;
      if (v) {
        s.scrollTop += Math.max(-18, Math.min(18, v));
        retarget();
      }
    }
    raf.current = requestAnimationFrame(tick);
  };

  // Слушатели окна — стабильные обёртки: компонент перерисовывается во время
  // перетаскивания, а снять нужно ровно те функции, что повесили.
  const handlers = useRef({ move: (_e: PointerEvent) => {}, up: () => {} });
  const stable = useRef({
    move: (e: PointerEvent) => handlers.current.move(e),
    up: () => handlers.current.up(),
  }).current;

  const stopListening = () => {
    window.removeEventListener("pointermove", stable.move);
    window.removeEventListener("pointerup", stable.up);
    window.removeEventListener("pointercancel", stable.up);
    if (raf.current !== null) cancelAnimationFrame(raf.current);
    raf.current = null;
  };

  function onMove(e: PointerEvent) {
    const p = press.current;
    if (!p) return;
    pointer.current = { x: e.clientX, y: e.clientY };
    const dist = Math.hypot(e.clientX - p.x0, e.clientY - p.y0);
    if (!p.dragging) {
      if (!p.armed) {
        // Сдвинули до долгого нажатия — это не «схватить».
        if (dist > 6) {
          if (p.timer !== null) window.clearTimeout(p.timer);
          press.current = null;
          stopListening();
        }
        return;
      }
      if (dist < 4) return;
      p.dragging = true;
      if (p.timer !== null) window.clearTimeout(p.timer);
      previewRef.current = latest.current.layout;
      setPreview(latest.current.layout);
      setDrag({ id: p.id, w: p.w, h: p.h, landing: false });
      scroller.current = scrollParent(rootRef.current);
      raf.current = requestAnimationFrame(tick);
    }
    gx.set(e.clientX - p.offX);
    gy.set(e.clientY - p.offY);
    retarget();
  }

  async function onUp() {
    const p = press.current;
    press.current = null;
    stopListening();
    if (!p) return;
    if (p.timer !== null) window.clearTimeout(p.timer);
    const L = latest.current;
    if (!p.dragging) {
      // Короткий тап (не в режиме упорядочивания) — открыть приложение.
      if (!p.fired && !L.editing && L.grid) {
        const app = L.apps.find((a) => a.id === p.id);
        if (app?.installed) {
          const info = L.describe(app);
          void (info.action ? info.action.run() : L.onLaunch(app.id));
        }
      }
      return;
    }
    const final = previewRef.current ?? L.layout;
    if (!sameLayout(final, L.layout)) L.onLayout(final);
    // «Призрак» мягко садится в свою ячейку, затем исчезает.
    const slot = document
      .querySelector(`[data-drop-app="${CSS.escape(p.id)}"]`)
      ?.getBoundingClientRect();
    if (slot && !L.reduced) {
      setDrag((d) => (d ? { ...d, landing: true } : d));
      await Promise.all([animate(gx, slot.left, LAND), animate(gy, slot.top, LAND)]);
    }
    previewRef.current = null;
    setPreview(null);
    setDrag(null);
  }

  handlers.current = { move: onMove, up: () => void onUp() };

  const startPress = (e: React.PointerEvent, id: string, el: HTMLElement) => {
    if (e.button !== 0 || press.current) return;
    const r = el.getBoundingClientRect();
    const p: Press = {
      id,
      x0: e.clientX,
      y0: e.clientY,
      offX: e.clientX - r.left,
      offY: e.clientY - r.top,
      w: r.width,
      h: r.height,
      armed: latest.current.editing,
      fired: false,
      dragging: false,
      timer: null,
    };
    if (!p.armed) {
      p.timer = window.setTimeout(() => {
        p.armed = true;
        p.fired = true;
        latest.current.onEditing(true);
      }, LONG_PRESS_MS);
    }
    press.current = p;
    pointer.current = { x: e.clientX, y: e.clientY };
    window.addEventListener("pointermove", stable.move);
    window.addEventListener("pointerup", stable.up);
    window.addEventListener("pointercancel", stable.up);
  };

  useEffect(() => () => stopListening(), []); // eslint-disable-line react-hooks/exhaustive-deps

  // Клик по пустому месту выключает режим упорядочивания (как в iOS).
  const exitOnEmpty = useCallback(
    (e: PointerEvent) => {
      if (e.button !== 0 || press.current) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest("[data-drop-app], button, input, a, [role=dialog], [role=menu], [data-keep-editing]")) return;
      onEditing(false);
    },
    [onEditing],
  );
  useEffect(() => {
    if (!editing) return;
    window.addEventListener("pointerdown", exitOnEmpty, true);
    return () => window.removeEventListener("pointerdown", exitOnEmpty, true);
  }, [editing, exitOnEmpty]);

  const openMenu = (e: React.MouseEvent, app: AppProfileStatus, info: RowInfo) => {
    e.preventDefault();
    const current = layout.folders.find((f) => f.apps.includes(app.id))?.id;
    const items: MenuItem[] = [];
    if (app.installed) {
      items.push(
        info.action
          ? { label: info.action.label, run: () => void info.action!.run() }
          : { label: "Открыть", run: () => void onLaunch(app.id) },
      );
      for (const m of info.menu.filter((m) => !m.danger || !/Удалить|Убрать/.test(m.label))) {
        items.push({ label: m.label, run: () => void m.run(), danger: m.danger });
      }
    }
    items.push({ divider: true, label: "", run: () => {} });
    items.push({ header: true, label: "Переместить в папку", run: () => {} });
    for (const f of layout.folders) {
      items.push({
        label: f.name,
        checked: f.id === current,
        run: () => onLayout(moveApp(layout, app.id, { folder: f.id })),
      });
    }
    items.push({ divider: true, label: "", run: () => {} });
    items.push({ label: "Настройки…", run: () => onSettings(app.id) });
    items.push({ label: "Упорядочить", run: () => onEditing(true) });
    items.push({ label: "Убрать из списка", danger: true, run: () => onRemove(app.id) });
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const addFolder = () =>
    onLayout({
      ...layout,
      folders: [...layout.folders, { id: newFolderId(), name: "Новая папка", apps: [] }],
    });

  const renameFolder = (id: string, name: string) =>
    onLayout({ ...layout, folders: layout.folders.map((f) => (f.id === id ? { ...f, name } : f)) });

  const deleteFolder = (id: string) =>
    onLayout({ ...layout, folders: layout.folders.filter((f) => f.id !== id) });

  // Папка, куда переезжает схваченное приложение (если не своя) — подсвечиваем.
  const fromFolder = drag ? layout.folders.find((f) => f.apps.includes(drag.id))?.id : undefined;
  const intoFolder = drag ? shown.folders.find((f) => f.apps.includes(drag.id))?.id : undefined;
  const hotFolder = intoFolder !== fromFolder ? intoFolder : undefined;

  const folders = shown.folders.filter((f) => editing || f.apps.some((id) => byId.has(id)));

  const content = (app: AppProfileStatus, info: RowInfo, ghost = false) =>
    grid ? (
      <AppTile
        name={app.name}
        icon={app.location ? icons[app.location] : undefined}
        installed={app.installed}
        status={info.status}
        editing={editing}
        onRemove={() => onRemove(app.id)}
        onSettings={ghost ? undefined : () => onSettings(app.id)}
      />
    ) : (
      <AppRow
        app={app}
        status={info.status}
        action={info.action}
        icon={app.location ? icons[app.location] : undefined}
        editing={editing}
        onRemove={() => onRemove(app.id)}
        onSettings={ghost ? undefined : () => onSettings(app.id)}
        onLaunch={() => onLaunch(app.id)}
      />
    );

  const dragged = drag ? byId.get(drag.id) : undefined;

  return (
    <LayoutGroup>
      <div ref={rootRef} className="space-y-5">
        {folders.map((folder) => {
          const items = folder.apps.map((id) => byId.get(id)).filter(Boolean) as AppProfileStatus[];
          const hot = folder.id === hotFolder;
          return (
            <motion.section
              key={folder.id}
              layout={!reduced}
              transition={SPRING}
              data-drop-folder={folder.id}
              className="relative rounded-2xl"
            >
              {/* Папка-цель: мягкое свечение сверху вниз вместо рамки */}
              <AnimatePresence>
                {hot && (
                  <motion.div
                    aria-hidden
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.2 }}
                    className="pointer-events-none absolute -inset-2 rounded-[20px] bg-[radial-gradient(120%_80%_at_50%_0%,rgba(16,185,129,0.10),transparent_70%)]"
                  />
                )}
              </AnimatePresence>

              {/* Заголовок папки: в режиме упорядочивания — переименование */}
              <div className="relative mb-1.5 flex h-6 items-center gap-2 px-1" data-drop-folder={folder.id}>
                {editing && !drag ? (
                  <input
                    value={folder.name}
                    onChange={(e) => renameFolder(folder.id, e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                    className="min-w-0 flex-1 bg-transparent text-[12px] font-medium text-vb-silver outline-none"
                  />
                ) : (
                  <span
                    className={cn(
                      "text-[12px] font-medium transition-colors duration-200",
                      hot ? "text-vb-emerald" : editing ? "text-vb-silver" : "text-vb-silver-faint",
                    )}
                  >
                    {folder.name}
                  </span>
                )}
                {editing && !drag && items.length === 0 && (
                  <button
                    type="button"
                    onClick={() => deleteFolder(folder.id)}
                    aria-label="Удалить пустую папку"
                    className="rounded p-0.5 text-vb-silver-faint hover:text-vb-loss"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>

              <div
                data-drop-folder={folder.id}
                className={cn(
                  "relative",
                  grid
                    ? "flex min-h-[64px] flex-wrap gap-x-1 gap-y-2"
                    : "surface-card min-h-[52px] divide-y divide-vb-border/70 overflow-visible",
                  hot && !grid && "border-vb-emerald/25",
                )}
              >
                {items.length === 0 && (
                  <div className="flex h-[52px] w-full items-center justify-center text-[12px] text-vb-silver-faint">
                    Перетащите сюда приложение
                  </div>
                )}
                {items.map((app) => {
                  const info = describe(app);
                  return (
                    <BoardItem
                      key={app.id}
                      id={app.id}
                      grid={grid}
                      editing={editing}
                      reduced={reduced}
                      placeholder={drag?.id === app.id}
                      onPress={startPress}
                      onContextMenu={(e) => openMenu(e, app, info)}
                    >
                      {content(app, info)}
                    </BoardItem>
                  );
                })}
              </div>
            </motion.section>
          );
        })}

        <AnimatePresence>
          {editing && !drag && (
            <motion.button
              type="button"
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 6 }}
              onClick={addFolder}
              className="flex w-full items-center justify-center gap-2 rounded-xl border border-dashed border-vb-border py-3 text-[13px] text-vb-silver-dim transition-colors hover:border-vb-emerald/50 hover:text-vb-silver"
            >
              <FolderPlus className="h-4 w-4" />
              Новая папка
            </motion.button>
          )}
        </AnimatePresence>
      </div>

      {/* «Призрак» схваченного приложения — едет за курсором поверх всего */}
      {drag &&
        dragged &&
        createPortal(
          <motion.div
            aria-hidden
            style={{ x: gx, y: gy, width: drag.w, height: drag.h }}
            className="pointer-events-none fixed left-0 top-0 z-[80]"
          >
            <motion.div
              initial={{ scale: 1, rotate: 0 }}
              animate={
                drag.landing
                  ? { scale: 1, rotate: 0, boxShadow: "0 0 0 rgba(0,0,0,0)" }
                  : {
                      scale: grid ? 1.12 : 1.025,
                      rotate: grid ? -3 : 0,
                      boxShadow: grid ? "0 0 0 rgba(0,0,0,0)" : "0 18px 40px rgba(0,0,0,0.55)",
                    }
              }
              transition={{ type: "spring", bounce: 0.25, duration: 0.3 }}
              className={cn(
                "h-full w-full",
                !grid && "rounded-xl border border-vb-border-strong bg-vb-surface",
                grid && "drop-shadow-[0_14px_22px_rgba(0,0,0,0.55)]",
              )}
            >
              {content(dragged, describe(dragged), true)}
            </motion.div>
          </motion.div>,
          document.body,
        )}

      <AnimatePresence>
        {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      </AnimatePresence>
    </LayoutGroup>
  );
}

/*
  Элемент доски: нажатие передаётся доске (она решает — тап, долгое
  нажатие или перетаскивание). В режиме упорядочивания плитки покачиваются
  (только плитки: так в iOS; строки списка не качаются). Пока приложение
  тащат, на его месте — мягкий «слот», куда оно сядет.
*/
function BoardItem({
  id,
  grid,
  editing,
  reduced,
  placeholder,
  onPress,
  onContextMenu,
  children,
}: {
  id: string;
  grid: boolean;
  editing: boolean;
  reduced: boolean;
  placeholder: boolean;
  onPress: (e: React.PointerEvent, id: string, el: HTMLElement) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  children: React.ReactNode;
}) {
  // Своя фаза покачивания у каждой иконки — как в iOS, не синхронно.
  const phase = useRef(Math.random() * 0.12);
  const wobble = editing && grid && !reduced && !placeholder;

  return (
    <motion.div
      layout={!reduced}
      layoutId={`app-${id}`}
      transition={SPRING}
      data-drop-app={id}
      onPointerDown={(e) => onPress(e, id, e.currentTarget)}
      onContextMenu={onContextMenu}
      className={cn("relative select-none", grid && "touch-none", editing && "cursor-grab")}
    >
      <AnimatePresence>
        {placeholder && (
          <motion.span
            aria-hidden
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            className={cn(
              "pointer-events-none absolute border border-dashed border-vb-emerald/35 bg-vb-emerald/[0.05]",
              grid ? "left-[calc(50%-26px)] top-2 h-[52px] w-[52px] rounded-[22%]" : "inset-1.5 rounded-xl",
            )}
          />
        )}
      </AnimatePresence>
      <motion.div
        animate={wobble ? { rotate: [-1.6, 1.6, -1.6] } : { rotate: 0 }}
        transition={
          wobble
            ? { duration: 0.26, repeat: Infinity, ease: "easeInOut", delay: phase.current }
            : { duration: 0.15 }
        }
        className={cn("transition-opacity duration-150", placeholder && "opacity-0")}
      >
        {children}
      </motion.div>
    </motion.div>
  );
}
