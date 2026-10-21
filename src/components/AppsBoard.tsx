import { useRef, useState } from "react";
import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "framer-motion";
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

/*
  Приложения папками — списком или сеткой, с режимом упорядочивания как в
  iOS: долгое нажатие (или правый клик → «Упорядочить») — иконки
  покачиваются, их можно перетаскивать между местами и папками, «−» убирает
  из списка. Перестройка — layout-анимацией (элементы плавно перетекают).
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
  const reduced = useReducedMotion();
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [dropFolder, setDropFolder] = useState<string | null>(null);
  const byId = new Map(apps.map((a) => [a.id, a]));
  const grid = layout.view === "grid";

  /** Куда отпустили: на приложение (встать перед ним) или на папку (в конец). */
  const dropTarget = (x: number, y: number, self: string) => {
    for (const el of document.elementsFromPoint(x, y)) {
      const h = el as HTMLElement;
      const app = h.dataset?.dropApp;
      if (app && app !== self) return { beforeApp: app } as const;
      const folder = h.dataset?.dropFolder;
      if (folder) return { folder } as const;
    }
    return null;
  };

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

  const folders = layout.folders.filter((f) => editing || f.apps.some((id) => byId.has(id)));

  return (
    <LayoutGroup>
      <div className="space-y-5">
        {folders.map((folder) => {
          const items = folder.apps.map((id) => byId.get(id)).filter(Boolean) as AppProfileStatus[];
          return (
            <motion.section
              key={folder.id}
              layout={!reduced}
              transition={SPRING}
              data-drop-folder={folder.id}
              className={cn(
                "rounded-2xl transition-colors duration-150",
                dropFolder === folder.id && "bg-vb-emerald/[0.06] ring-1 ring-vb-emerald/30",
              )}
            >
              {/* Заголовок папки: в режиме упорядочивания — переименование */}
              <div className="mb-1.5 flex h-6 items-center gap-2 px-1" data-drop-folder={folder.id}>
                {editing ? (
                  <input
                    value={folder.name}
                    onChange={(e) => renameFolder(folder.id, e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                    className="min-w-0 flex-1 bg-transparent text-[12px] font-medium text-vb-silver outline-none"
                  />
                ) : (
                  <span className="text-[12px] font-medium text-vb-silver-faint">{folder.name}</span>
                )}
                {editing && items.length === 0 && (
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
                  grid
                    ? "flex min-h-[64px] flex-wrap gap-x-1 gap-y-2"
                    : "surface-card min-h-[52px] divide-y divide-vb-border/70 overflow-visible",
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
                      reduced={!!reduced}
                      onLongPress={() => onEditing(true)}
                      onContextMenu={(e) => openMenu(e, app, info)}
                      onDragOver={(x, y) => {
                        const t = dropTarget(x, y, app.id);
                        setDropFolder(t && "folder" in t ? (t.folder ?? null) : null);
                      }}
                      onDrop={(x, y) => {
                        setDropFolder(null);
                        const t = dropTarget(x, y, app.id);
                        if (t) onLayout(moveApp(layout, app.id, t));
                      }}
                      onTap={() => {
                        if (!grid || !app.installed) return;
                        void (info.action ? info.action.run() : onLaunch(app.id));
                      }}
                    >
                      {grid ? (
                        <AppTile
                          name={app.name}
                          icon={app.location ? icons[app.location] : undefined}
                          installed={app.installed}
                          status={info.status}
                          editing={editing}
                          onRemove={() => onRemove(app.id)}
                          onSettings={() => onSettings(app.id)}
                        />
                      ) : (
                        <AppRow
                          app={app}
                          status={info.status}
                          action={info.action}
                          menu={info.menu}
                          icon={app.location ? icons[app.location] : undefined}
                          editing={editing}
                          onRemove={() => onRemove(app.id)}
                          onSettings={() => onSettings(app.id)}
                          onLaunch={() => onLaunch(app.id)}
                        />
                      )}
                    </BoardItem>
                  );
                })}
              </div>
            </motion.section>
          );
        })}

        <AnimatePresence>
          {editing && (
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

      <AnimatePresence>
        {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      </AnimatePresence>
    </LayoutGroup>
  );
}

/*
  Элемент доски: долгое нажатие → режим упорядочивания; в нём —
  покачивание (только плитки: так в iOS; строки списка не качаются) и
  перетаскивание с «прилипанием» к месту и подсветкой папки-цели.
*/
function BoardItem({
  id,
  grid,
  editing,
  reduced,
  onLongPress,
  onContextMenu,
  onDragOver,
  onDrop,
  onTap,
  children,
}: {
  id: string;
  grid: boolean;
  editing: boolean;
  reduced: boolean;
  onLongPress: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onDragOver: (x: number, y: number) => void;
  onDrop: (x: number, y: number) => void;
  onTap: () => void;
  children: React.ReactNode;
}) {
  const timer = useRef<number | null>(null);
  const start = useRef<{ x: number; y: number } | null>(null);
  const fired = useRef(false);
  // Своя фаза покачивания у каждой иконки — как в iOS, не синхронно.
  const phase = useRef(Math.random() * 0.12);

  const cancel = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  };

  return (
    <motion.div
      layout={!reduced}
      layoutId={`app-${id}`}
      transition={SPRING}
      data-drop-app={id}
      drag={editing}
      dragSnapToOrigin
      dragElastic={0.85}
      whileDrag={{ scale: grid ? 1.12 : 1.02, zIndex: 50, cursor: "grabbing" }}
      onDrag={(e) => {
        const p = e as PointerEvent;
        onDragOver(p.clientX, p.clientY);
      }}
      onDragEnd={(e) => {
        const p = e as PointerEvent;
        onDrop(p.clientX, p.clientY);
      }}
      onPointerDown={(e) => {
        if (editing || e.button !== 0) return;
        fired.current = false;
        start.current = { x: e.clientX, y: e.clientY };
        timer.current = window.setTimeout(() => {
          fired.current = true;
          onLongPress();
        }, LONG_PRESS_MS);
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (s && Math.hypot(e.clientX - s.x, e.clientY - s.y) > 6) cancel();
      }}
      onPointerUp={() => {
        cancel();
        if (!editing && !fired.current) onTap();
      }}
      onPointerLeave={cancel}
      onContextMenu={onContextMenu}
      className={cn("relative select-none", grid ? "touch-none" : "", editing && "cursor-grab")}
      style={{ position: "relative" }}
    >
      <motion.div
        animate={
          editing && grid && !reduced
            ? { rotate: [-1.6, 1.6, -1.6] }
            : { rotate: 0 }
        }
        transition={
          editing && grid && !reduced
            ? { duration: 0.26, repeat: Infinity, ease: "easeInOut", delay: phase.current }
            : { duration: 0.15 }
        }
      >
        {children}
      </motion.div>
    </motion.div>
  );
}
