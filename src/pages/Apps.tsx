import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { LayoutGrid, List, Loader2, Plus } from "lucide-react";
import { Button } from "../components/Button";
import { type RowAction, type RowStatus } from "../components/AppRow";
import { AppSearchAdd } from "../components/AppSearchAdd";
import { AppsBoard } from "../components/AppsBoard";
import { AppSettingsModal } from "../components/AppSettingsModal";
import { moveApp } from "../lib/appLayout";
import { reconcile, type AppsLayout, type AppsView } from "../lib/appLayout";
import {
  appsAddCustom,
  appsCreateShortcut,
  appsIcons,
  appsLayoutGet,
  appsLayoutSet,
  appsLaunch,
  appsOpenExtension,
  appsList,
  appsRelaunch,
  appsRemove,
  appsTraffic,
  claudeProxyDisable,
  claudeProxyEnable,
  claudeProxyStatus,
  telegramSetup,
  telegramStatus,
} from "../lib/api";
import type {
  AppProfileStatus,
  AppTraffic,
  ClaudeProxyStatus,
  TelegramStatus,
} from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { isMac } from "../lib/platform";
import { cn } from "../lib/cn";

/** Статус по соединениям: раз в 5с, после действия — чаще (ждём переключения). */
const TRAFFIC_POLL_MS = 5_000;
const TRAFFIC_FAST_MS = 1_500;
const FAST_TICKS = 20;

const CLAUDE_ID = "claude-desktop";
const TELEGRAM_ID = "telegram";
/** Как полностью перезапустить Claude Desktop, чтобы он перечитал прокси. */
const RESTART_CLAUDE = isMac
  ? "Перезапустите Claude Desktop (Cmd+Q и открыть снова)."
  : "Перезапустите Claude Desktop (закройте его через значок в трее и откройте снова).";

const n = (k: number) => `${k} ${k % 10 === 1 && k % 100 !== 11 ? "соединение" : k % 10 >= 2 && k % 10 <= 4 && (k % 100 < 10 || k % 100 >= 20) ? "соединения" : "соединений"}`;

/** Общая часть статуса: что показывают живые соединения процесса. */
function trafficStatus(t: AppTraffic | undefined, idle: string): RowStatus {
  if (!t?.running) return { tone: "muted", text: idle };
  if (t.via_bridge > 0 && t.direct === 0) return { tone: "ok", text: `Через мост · ${n(t.via_bridge)}` };
  if (t.direct > 0 && t.via_bridge === 0) return { tone: "warn", text: "Напрямую, мимо моста" };
  if (t.direct > 0) return { tone: "warn", text: `Частично мимо моста · напрямую ${t.direct}` };
  return { tone: "muted", text: "Открыт · сейчас без соединений" };
}

/*
  Приложения: один список одинаковых строк. У каждой — статус по ЖИВЫМ
  соединениям (через мост / мимо моста) и одна кнопка, если что-то не так.
  Claude Desktop и Telegram — такие же строки; их настройка — та же кнопка
  «Подключить», а не отдельные панели. macOS и Windows — одна логика:
  соединения считаются по lsof / netstat.
*/
export function Apps() {
  const [apps, setApps] = useState<AppProfileStatus[] | null>(null);
  const [adding, setAdding] = useState(false);

  const [traffic, setTraffic] = useState<Record<string, AppTraffic>>({});
  const [claude, setClaude] = useState<ClaudeProxyStatus | null>(null);
  const [telegram, setTelegram] = useState<TelegramStatus | null>(null);
  const [icons, setIcons] = useState<Record<string, string>>({});
  const [savedLayout, setSavedLayout] = useState<AppsLayout | null | undefined>(undefined);
  const [editing, setEditing] = useState(false);
  const [settingsId, setSettingsId] = useState<string | null>(null);

  // Раскладка (вид, папки, порядок): загрузка один раз, сохранение с паузой.
  useEffect(() => {
    appsLayoutGet()
      .then((l) => setSavedLayout((l as AppsLayout | null) ?? null))
      .catch(() => setSavedLayout(null));
  }, []);
  const visible = (apps ?? []).filter((a) => !a.hidden);
  const layout = reconcile(savedLayout ?? null, visible);
  const saveTimer = useRef<number | null>(null);
  const updateLayout = (l: AppsLayout) => {
    setSavedLayout(l);
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => void appsLayoutSet(l).catch(() => {}), 400);
  };
  // Esc выходит из режима упорядочивания.
  useEffect(() => {
    if (!editing) return;
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setEditing(false);
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [editing]);
  const fastTicks = useRef(0);

  const load = useCallback(async () => {
    setApps(await appsList().catch(() => []));
  }, []);

  // Иконки своих приложений (встроенные рисуются брендовыми плитками).
  // Каждый путь запрашиваем один раз: если иконку не удалось достать, ответ
  // пустой — повторный запрос по изменению icons зациклил бы опрос.
  const askedIcons = useRef(new Set<string>());
  useEffect(() => {
    if (!apps) return;
    const need = apps
      .map((a) => a.location)
      .filter((l): l is string => !!l && !askedIcons.current.has(l));
    if (need.length === 0) return;
    need.forEach((l) => askedIcons.current.add(l));
    appsIcons(need)
      .then((got) => setIcons((old) => ({ ...old, ...got })))
      .catch(() => {});
  }, [apps]);

  const loadTraffic = useCallback(async () => {
    const list = await appsTraffic().catch(() => [] as AppTraffic[]);
    setTraffic(Object.fromEntries(list.map((t) => [t.id, t])));
  }, []);

  const loadSetup = useCallback(async () => {
    await Promise.all([
      claudeProxyStatus().then(setClaude).catch(() => {}),
      telegramStatus().then(setTelegram).catch(() => {}),
    ]);
  }, []);

  useEffect(() => {
    void load();
    void loadSetup();
    void loadTraffic();
    // Скрытое окно не опрашиваем; после действия — чаще, пока не переключится.
    let timer = 0;
    const tick = () => {
      if (!document.hidden) void loadTraffic();
      const fast = fastTicks.current > 0;
      if (fast) fastTicks.current -= 1;
      timer = window.setTimeout(tick, fast ? TRAFFIC_FAST_MS : TRAFFIC_POLL_MS);
    };
    timer = window.setTimeout(tick, TRAFFIC_POLL_MS);
    return () => window.clearTimeout(timer);
  }, [load, loadSetup, loadTraffic]);

  /** После действия: обновить настройки и чаще смотреть соединения. */
  const after = async <T,>(p: Promise<T>): Promise<T> => {
    const r = await p;
    fastTicks.current = FAST_TICKS;
    void loadSetup();
    void loadTraffic();
    return r;
  };

  const shortcutMenu = (app: AppProfileStatus): RowAction[] => [
    {
      label: "Ярлык на рабочий стол",
      run: () => appsCreateShortcut(app.id, "desktop").then(() => "Ярлык на рабочем столе"),
    },
    {
      label: isMac ? "Ярлык в «Программы»" : "Ярлык в меню Пуск",
      run: () =>
        appsCreateShortcut(app.id, "start_menu").then(() =>
          isMac ? "Ярлык в «Программах» — перетащите его в Dock" : "Ярлык в Пуске",
        ),
    },
  ];

  /** Статус, главная кнопка и меню строки — по соединениям. */
  /** Строка + «Убрать из списка» у любого приложения (встроенные прячутся,
      вернуть — через «Добавить»). */
  const describe = (
    app: AppProfileStatus,
  ): { status: RowStatus | null; action: RowAction | null; menu: RowAction[] } => {
    const r = describeInner(app);
    return {
      ...r,
      menu: [
        ...r.menu.filter((m) => m.label !== "Удалить из списка"),
        { label: "Убрать из списка", run: () => removeApp(app.id), danger: true },
      ],
    };
  };

  const describeInner = (
    app: AppProfileStatus,
  ): { status: RowStatus | null; action: RowAction | null; menu: RowAction[] } => {
    const remove: RowAction[] = app.builtin
      ? []
      : [{ label: "Удалить из списка", run: () => removeApp(app.id), danger: true }];
    const t = traffic[app.id];
    const relaunch: RowAction = {
      label: "Перезапустить через мост",
      run: () => after(appsRelaunch(app.id)),
    };
    if (app.id === CLAUDE_ID) {
      const policy = claude?.policy;
      const menu = shortcutMenu(app);
      // macOS стирает управляемые настройки — их возвращает служба; политику
      // HKLM на Windows система не трогает, служба не нужна.
      if (isMac && policy?.configured && policy.auto_restore === false) {
        menu.push({
          label: "Включить автовосстановление",
          run: () => after(claudeProxyEnable()).then((r) => r.policy.notice ?? "Автовосстановление включено"),
        });
      }
      if (policy?.managed) {
        menu.push({
          label: "Отключить прокси Claude",
          run: () => after(claudeProxyDisable()).then((r) => r.policy.notice ?? "Прежние настройки Claude восстановлены"),
          danger: true,
        });
      }
      if (policy && !policy.configured) {
        if (policy.conflict) {
          return { status: { tone: "warn", text: "Задан чужой прокси Claude — Helper его не меняет" }, action: null, menu };
        }
        return {
          status: { tone: "warn", text: "Прокси Claude не настроен" },
          action: {
            label: "Подключить",
            run: () =>
              after(claudeProxyEnable()).then(
                (r) => r.policy.notice ?? `Готово, защита от утечек включена. ${RESTART_CLAUDE}`,
              ),
          },
          menu,
        };
      }
      const st = trafficStatus(t, "Настроен · откроется через мост");
      if (st.tone === "warn") return { status: { tone: "warn", text: "Открыт до настройки — идёт мимо моста" }, action: relaunch, menu };
      if (st.tone === "muted" && t?.running) return { status: { tone: "ok", text: "Через мост" }, action: null, menu };
      return { status: st, action: null, menu };
    }

    if (app.id === TELEGRAM_ID) {
      const connect: RowAction = {
        label: "Подключить",
        run: () => after(telegramSetup()).then(() => "В окне Telegram нажмите «Подключить»"),
      };
      const menu: RowAction[] = [{ label: "Открыть окно прокси в Telegram", run: connect.run }];
      if (telegram && !telegram.socks_ready) {
        return { status: { tone: "warn", text: "Мосту нужно обновление — «Обновить» в Обзоре" }, action: null, menu: [] };
      }
      // Свой локальный прокси (tg-ws-proxy и т.п.) ведёт Telegram мимо моста.
      if (t?.running && t.via_bridge === 0 && t.direct === 0 && t.other_proxy) {
        return { status: { tone: "warn", text: "Через другой прокси, мимо моста" }, action: connect, menu };
      }
      const st = trafficStatus(t, "Не запущен");
      return { status: st, action: st.tone === "warn" ? connect : null, menu };
    }

    // Браузер: расширение (FoxyProxy, SwitchyOmega…) перебивает --proxy-server.
    if (t?.proxy_extension) {
      const ext = t.proxy_extension;
      return {
        status: { tone: "warn", text: `Прокси перехватывает ${ext.name}` },
        action: {
          label: "Отключить",
          run: () =>
            after(appsOpenExtension(app.id, ext.id)).then(
              () => `Выключите ${ext.name} в открывшейся вкладке — перезапуск не нужен`,
            ),
        },
        menu: [...shortcutMenu(app), ...remove],
      };
    }
    const st = trafficStatus(t, "Откроется через мост");
    return { status: st, action: st.tone === "warn" ? relaunch : null, menu: [...shortcutMenu(app), ...remove] };
  };

  const removeApp = async (id: string) => {
    try {
      setApps(await appsRemove(id));
    } catch {
      await load();
    }
  };

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[520px] flex-col px-6 py-6"
    >
      <motion.header variants={fadeInUp} className="flex items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">
            Приложения
          </h1>
          <p className="text-[12px] text-vb-silver-faint">
            {editing ? "Перетаскивайте между папками" : "Всё, что идёт через ваш прокси"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {!editing && (
            <ViewSwitch
              value={layout.view}
              onChange={(view) => updateLayout({ ...layout, view })}
            />
          )}
          {editing ? (
            <Button variant="primary" size="sm" onClick={() => setEditing(false)}>
              Готово
            </Button>
          ) : (
            <Button variant="secondary" size="sm" onClick={() => setAdding((v) => !v)}>
              <Plus className="h-3.5 w-3.5" />
              Добавить
            </Button>
          )}
        </div>
      </motion.header>

      {/* Умный поиск среди установленных программ (Spotlight / меню «Пуск») */}
      {adding && (
        <AppSearchAdd
          existing={visible.map((a) => a.location ?? "").filter(Boolean)}
          onAdd={async (name, path) => {
            setApps(await appsAddCustom(name, path));
            setAdding(false);
            void loadTraffic();
          }}
          onClose={() => setAdding(false)}
        />
      )}

      {/* Список — одна поверхность, hairline-разделители */}
      <motion.section variants={fadeInUp} className="mt-5">
        {apps === null ? (
          <div className="surface-card flex items-center justify-center gap-2 p-8 text-[13px] text-vb-silver-dim">
            <Loader2 className="h-4 w-4 animate-spin" />
            Загружаю приложения…
          </div>
        ) : apps.length === 0 ? (
          <div className="surface-card p-8 text-center text-[13px] text-vb-silver-dim">
            Приложений пока нет — добавьте своё через кнопку выше.
          </div>
        ) : (
          <AppsBoard
            apps={visible}
            layout={layout}
            onLayout={updateLayout}
            editing={editing}
            onEditing={setEditing}
            describe={describe}
            icons={icons}
            onLaunch={(id) => after(appsLaunch(id))}
            onRemove={(id) => void removeApp(id)}
            onSettings={setSettingsId}
          />
        )}
      </motion.section>
      {(() => {
        const app = visible.find((a) => a.id === settingsId) ?? null;
        return (
          <AppSettingsModal
            app={app}
            icon={app?.location ? icons[app.location] : undefined}
            status={app ? describe(app).status : null}
            folders={layout.folders}
            onMoveToFolder={(fid) => app && updateLayout(moveApp(layout, app.id, { folder: fid }))}
            onRemove={() => {
              if (app) void removeApp(app.id);
              setSettingsId(null);
            }}
            onRelaunch={() => (app ? after(appsRelaunch(app.id)) : Promise.resolve())}
            onChanged={(list) => {
              if (list) setApps(list);
              void loadSetup();
              void loadTraffic();
            }}
            onClose={() => setSettingsId(null)}
          />
        );
      })()}
    </motion.div>
  );
}

/** Переключатель вида — сегмент в духе iOS: подложка перетекает пружиной. */
function ViewSwitch({ value, onChange }: { value: AppsView; onChange: (v: AppsView) => void }) {
  const opts: { id: AppsView; label: string; icon: typeof List }[] = [
    { id: "list", label: "Список", icon: List },
    { id: "grid", label: "Сетка", icon: LayoutGrid },
  ];
  return (
    <div className="flex rounded-lg border border-vb-border bg-vb-surface p-0.5" role="radiogroup" aria-label="Вид">
      {opts.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          role="radio"
          aria-checked={value === id}
          aria-label={label}
          title={label}
          onClick={() => onChange(id)}
          className={cn(
            "relative flex h-7 w-8 items-center justify-center rounded-md transition-colors",
            value === id ? "text-vb-fg" : "text-vb-silver-faint hover:text-vb-silver",
          )}
        >
          {value === id && (
            <motion.span
              layoutId="view-switch"
              className="absolute inset-0 rounded-md bg-vb-surface-2"
              transition={{ type: "spring", bounce: 0.2, duration: 0.35 }}
            />
          )}
          <Icon className="relative h-3.5 w-3.5" strokeWidth={2} />
        </button>
      ))}
    </div>
  );
}
