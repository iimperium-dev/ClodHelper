import { lazy, type ComponentType } from "react";

/*
  Разделы, шаги мастера и редкие окна грузятся отдельными кусками JS, когда
  нужны. Главное окно открывается быстрее, а трей и уведомления их вообще
  не грузят. preload() - взять кусок заранее (раздел будет нужен вот-вот):
  переход тогда без пустого кадра.
*/

type Lazy<P> = ReturnType<typeof lazy<ComponentType<P>>> & { preload: () => Promise<unknown> };

function lazyNamed<P>(load: () => Promise<ComponentType<P>>): Lazy<P> {
  let p: Promise<{ default: ComponentType<P> }> | undefined;
  const preload = () =>
    (p ??= load().then(
      (c) => ({ default: c }),
      (e) => {
        // Не запоминаем неудачу: «Повторить» попробует загрузить снова.
        p = undefined;
        throw e;
      },
    ));
  return Object.assign(lazy(preload), { preload });
}

export const Overview = lazyNamed(() => import("./pages/Overview").then((m) => m.Overview));
export const Proxies = lazyNamed(() => import("./pages/Proxies").then((m) => m.Proxies));
export const Apps = lazyNamed(() => import("./pages/Apps").then((m) => m.Apps));
export const Sites = lazyNamed(() => import("./pages/Sites").then((m) => m.Sites));
export const Tools = lazyNamed(() => import("./pages/Tools").then((m) => m.Tools));
export const Settings = lazyNamed(() => import("./pages/Settings").then((m) => m.Settings));

export const StepProxy = lazyNamed(() => import("./wizard/StepProxy").then((m) => m.StepProxy));
export const StepInstall = lazyNamed(() => import("./wizard/StepInstall").then((m) => m.StepInstall));
export const StepApps = lazyNamed(() => import("./wizard/StepApps").then((m) => m.StepApps));
export const StepDone = lazyNamed(() => import("./wizard/StepDone").then((m) => m.StepDone));

export const UpdateDialog = lazyNamed(() => import("./components/UpdateDialog").then((m) => m.UpdateDialog));

const PAGES = [Overview, Proxies, Apps, Sites, Tools, Settings];
const STEPS = [StepProxy, StepInstall, StepApps, StepDone];

/** Взять все разделы заранее, когда окно уже нарисовано и свободно. */
export function preloadPages() {
  for (const p of PAGES) void p.preload().catch(() => {});
}

export function preloadWizard() {
  for (const s of STEPS) void s.preload().catch(() => {});
}

/** Колбэк, когда браузер свободен (WebKit на macOS без requestIdleCallback). */
export function whenIdle(fn: () => void, timeout = 1500): () => void {
  if (typeof window.requestIdleCallback === "function") {
    const id = window.requestIdleCallback(fn, { timeout });
    return () => window.cancelIdleCallback(id);
  }
  const id = window.setTimeout(fn, 300);
  return () => window.clearTimeout(id);
}
