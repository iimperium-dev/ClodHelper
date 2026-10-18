/*
  Раскладка «Приложений»: вид (список/сетка) и папки с порядком.
  Хранится в настройках как есть (apps_layout); здесь — умная раскладка по
  умолчанию и согласование с реальным списком (новые приложения попадают в
  подходящую папку, удалённые исчезают из папок).
*/

export type AppsView = "list" | "grid";

export interface AppFolder {
  id: string;
  name: string;
  /** id приложений в порядке показа. */
  apps: string[];
}

export interface AppsLayout {
  view: AppsView;
  folders: AppFolder[];
}

/** Умные папки по умолчанию — в этом порядке. */
const SMART: { id: string; name: string; match: RegExp }[] = [
  { id: "ai", name: "AI", match: /claude|chatgpt|codex|cursor|copilot|gemini|ollama|lm studio|perplexity/i },
  {
    id: "browsers",
    name: "Браузеры",
    match: /chrome|safari|firefox|yandex|opera|edge|arc\b|brave|browser|comet|dolphin|vivaldi|tor\b/i,
  },
  {
    id: "messengers",
    name: "Мессенджеры",
    match: /telegram|whatsapp|wechat|signal|slack|discord|viber|skype|zoom|messages|сообщения|mail/i,
  },
  {
    id: "dev",
    name: "Разработка",
    match: /code|xcode|terminal|терминал|iterm|warp|jetbrains|webstorm|pycharm|intellij|sublime|docker|postman|github/i,
  },
];
const OTHER = { id: "other", name: "Другое" };

/** В какую умную папку положить приложение по его названию. */
export function smartFolderId(name: string): string {
  return SMART.find((f) => f.match.test(name))?.id ?? OTHER.id;
}

function smartFolderName(id: string): string {
  return SMART.find((f) => f.id === id)?.name ?? OTHER.name;
}

/**
 * Привести раскладку к текущему списку: каждое видимое приложение ровно в
 * одной папке; новые — в умную папку (создаётся при необходимости, в
 * своём месте порядка); исчезнувшие — убраны. Пустые папки сохраняются
 * (пользователь мог создать её только что), но UI вне режима
 * упорядочивания их не показывает.
 */
export function reconcile(
  saved: AppsLayout | null,
  apps: { id: string; name: string }[],
): AppsLayout {
  const ids = new Set(apps.map((a) => a.id));
  const layout: AppsLayout = {
    view: saved?.view === "grid" ? "grid" : "list",
    folders: (saved?.folders ?? []).map((f) => ({
      ...f,
      apps: f.apps.filter((id, i, arr) => ids.has(id) && arr.indexOf(id) === i),
    })),
  };
  const placed = new Set(layout.folders.flatMap((f) => f.apps));
  for (const app of apps) {
    if (placed.has(app.id)) continue;
    const fid = smartFolderId(app.name);
    let folder = layout.folders.find((f) => f.id === fid);
    if (!folder) {
      folder = { id: fid, name: smartFolderName(fid), apps: [] };
      // Умные папки — в каноническом порядке относительно друг друга.
      const order = [...SMART.map((s) => s.id), OTHER.id];
      const rank = order.indexOf(fid);
      const at = layout.folders.findIndex((f) => {
        const r = order.indexOf(f.id);
        return r !== -1 && r > rank;
      });
      layout.folders.splice(at === -1 ? layout.folders.length : at, 0, folder);
    }
    folder.apps.push(app.id);
    placed.add(app.id);
  }
  return layout;
}

/** Переместить приложение: перед другим приложением или в конец папки. */
export function moveApp(
  layout: AppsLayout,
  appId: string,
  target: { beforeApp: string } | { folder: string },
): AppsLayout {
  if ("beforeApp" in target && target.beforeApp === appId) return layout;
  const folders = layout.folders.map((f) => ({ ...f, apps: f.apps.filter((id) => id !== appId) }));
  if ("beforeApp" in target) {
    const f = folders.find((x) => x.apps.includes(target.beforeApp));
    if (!f) return layout;
    f.apps.splice(f.apps.indexOf(target.beforeApp), 0, appId);
  } else {
    const f = folders.find((x) => x.id === target.folder);
    if (!f) return layout;
    f.apps.push(appId);
  }
  return { ...layout, folders };
}

export function newFolderId(): string {
  return `f-${Date.now().toString(36)}`;
}
