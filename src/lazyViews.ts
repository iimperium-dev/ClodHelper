import { createElement, use, type ComponentType } from "react";
import type { AppView } from "./lib/types";

/*
  Разделы, шаги мастера и редкие окна грузятся отдельными кусками JS, когда
  нужны. Главное окно открывается быстрее, а трей и уведомления их вообще
  не грузят. preload() - взять кусок заранее (раздел будет нужен вот-вот).

  Не React.lazy: тот даже уже загруженный кусок при первом показе
  «подвешивает» на кадр, и раздел рисуется отложенным рендером, который
  может начаться заново - а разделы забирают одноразовые просьбы («сразу
  открыть замену пароля») в первом рендере и при повторе их теряли.
  Здесь загруженный раздел рисуется сразу, как обычный компонент.
*/

export type LazyView<P> = ComponentType<P> & {
  preload: () => Promise<unknown>;
  loaded: () => boolean;
};

/** Сбросы неудачных загрузок: «Повторить» в PageBoundary пробует снова. */
const resets = new Set<() => void>();

/** Забыть неудачные загрузки (кнопка «Повторить»). */
export function resetFailedViews() {
  for (const r of resets) r();
}

function lazyNamed<P extends object>(load: () => Promise<ComponentType<P>>): LazyView<P> {
  let C: ComponentType<P> | undefined;
  let p: Promise<ComponentType<P>> | undefined;
  // Неудачу помним до «Повторить»: иначе рендер после заглушки начинал бы
  // загрузку заново, и вместо ошибки крутилось бы «Загружаю…».
  let failed = false;
  resets.add(() => {
    if (failed) {
      failed = false;
      p = undefined;
    }
  });
  const preload = () =>
    (p ??= load().then(
      (c) => (C = c),
      (e) => {
        failed = true;
        throw e;
      },
    ));
  function View(props: P) {
    const Comp = C ?? use(preload());
    return createElement(Comp, props);
  }
  return Object.assign(View, { preload, loaded: () => C !== undefined });
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

const PAGES: Partial<Record<AppView, LazyView<never>>> = {
  overview: Overview as LazyView<never>,
  proxies: Proxies as LazyView<never>,
  apps: Apps as LazyView<never>,
  sites: Sites as LazyView<never>,
  tools: Tools as LazyView<never>,
  settings: Settings as LazyView<never>,
};
const STEPS = [StepProxy, StepInstall, StepApps, StepDone];

/** Код раздела уже загружен (или это не раздел) - его можно показать сразу. */
export function pageReady(view: AppView): boolean {
  return PAGES[view]?.loaded() ?? true;
}

/** Загрузить код раздела; ошибку покажет PageBoundary при показе. */
export function loadPage(view: AppView): Promise<unknown> {
  return PAGES[view]?.preload().catch(() => {}) ?? Promise.resolve();
}

/** Взять все разделы заранее, когда окно уже нарисовано и свободно. */
export function preloadPages() {
  for (const p of Object.values(PAGES)) void p.preload().catch(() => {});
}

export function preloadWizard() {
  for (const s of STEPS) void s.preload().catch(() => {});
}

/**
 * Колбэк чуть позже, когда окно уже нарисовано и первые ответы пришли.
 * Простой таймер, не requestIdleCallback: его нет в WebKit на macOS, а
 * подменённые часы в тестах стенда с ним сбоят.
 */
export function whenIdle(fn: () => void, delay = 800): () => void {
  const id = window.setTimeout(fn, delay);
  return () => window.clearTimeout(id);
}
