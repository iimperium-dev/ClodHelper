import { Suspense, useEffect, useRef, useState, type MutableRefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { onOpenUpdateDialog, openUpdateDialog } from "../lib/updateBus";
import { UpdateDialog, whenIdle } from "../lazyViews";
import type { UpdateInfo } from "../lib/types";

/*
  Окно обновления грузится отдельно: при запуске оно не нужно. Код берётся,
  когда главное окно уже нарисовано и свободно, или сразу, если обновление
  попросили открыть раньше. Просьба, пришедшая до того, как окно начало
  слушать, не теряется: её повторяем сразу после монтирования.
*/
export function UpdateDialogHost() {
  const [mounted, setMounted] = useState(false);
  const pending = useRef<UpdateInfo | null>(null);
  /** UpdateDialog смонтирован и сам слушает просьбы. */
  const ready = useRef(false);

  useEffect(() => {
    const ask = (info: UpdateInfo) => {
      if (ready.current) return;
      pending.current = info;
      setMounted(true);
    };
    const offBus = onOpenUpdateDialog(ask);
    // Своя подписка UpdateDialog на событие Tauri ставится не сразу -
    // пересылаем его в шину и после монтирования (повтор безвреден).
    const un = listen<UpdateInfo>("update:open", (e) => (ready.current ? openUpdateDialog(e.payload) : ask(e.payload)));
    const cancel = whenIdle(() => setMounted(true), 3000);
    return () => {
      offBus();
      cancel();
      void un.then((f) => f()).catch(() => {});
    };
  }, []);

  if (!mounted) return null;
  return (
    <Suspense fallback={null}>
      <UpdateDialog />
      <Replay pending={pending} ready={ready} />
    </Suspense>
  );
}

/** Стоит после UpdateDialog: его эффекты (подписки) уже отработали. */
function Replay({ pending, ready }: { pending: MutableRefObject<UpdateInfo | null>; ready: MutableRefObject<boolean> }) {
  useEffect(() => {
    ready.current = true;
    const info = pending.current;
    pending.current = null;
    if (info) openUpdateDialog(info);
  }, [pending, ready]);
  return null;
}
