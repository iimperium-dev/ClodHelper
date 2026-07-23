import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";

/*
  Кастомный titlebar вместо нативной «брови» Windows (decorations: false).
  - data-tauri-drag-region — перетаскивание окна (тянуть можно за бар и
    за заголовок; кнопки без атрибута — клики работают как клики).
  - Крестик прячет в трей (CloseRequested перехватывается в lib.rs) —
    полный выход по-прежнему через меню трея.
  - Кнопки в стиле Windows 11: широкие зоны, ховер; close — красный.
*/
export function Titlebar() {
  const win = getCurrentWindow();

  return (
    <header
      data-tauri-drag-region
      className="z-sticky flex h-9 shrink-0 select-none items-center justify-between border-b border-vb-border/60 bg-vb-bg/90"
    >
      {/* Слева пусто: имя приложения живёт в бренд-блоке сайдбара.
          Пустая зона остаётся drag-областью. */}
      <div data-tauri-drag-region className="flex-1" />

      <div className="flex h-full items-stretch">
        <button
          type="button"
          aria-label="Свернуть"
          onClick={() => void win.minimize()}
          className="flex w-11 items-center justify-center text-vb-silver-dim transition-colors duration-150 hover:bg-white/[0.06] hover:text-vb-fg"
        >
          <Minus className="h-3.5 w-3.5" strokeWidth={2.25} />
        </button>
        <button
          type="button"
          aria-label="Развернуть"
          onClick={() => void win.toggleMaximize()}
          className="flex w-11 items-center justify-center text-vb-silver-dim transition-colors duration-150 hover:bg-white/[0.06] hover:text-vb-fg"
        >
          <Square className="h-3 w-3" strokeWidth={2.25} />
        </button>
        <button
          type="button"
          aria-label="Закрыть (в трей)"
          onClick={() => void win.close()}
          className="flex w-11 items-center justify-center text-vb-silver-dim transition-colors duration-150 hover:bg-vb-loss hover:text-white"
        >
          <X className="h-3.5 w-3.5" strokeWidth={2.25} />
        </button>
      </div>
    </header>
  );
}
