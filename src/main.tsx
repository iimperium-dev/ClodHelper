import React, { type ComponentType } from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import { startUiWatchdog } from "./lib/uiWatchdog";

// Трей-попап и уведомление об обновлении живут в прозрачных окнах: помечаем
// body, чтобы CSS убрал фон (скругление и тень рисует контент).
const view = new URLSearchParams(window.location.search).get("view");
if (view === "tray" || view === "toast" || view === "updating") {
  document.body.dataset.view = view;
}

// Нативное контекстное меню WebView2 («Назад/Печать/Проверить») чужеродно для
// десктоп-приложения - глушим везде, кроме полей ввода (там нужны
// Копировать/Вставить). В dev Shift+ПКМ оставляет доступ к «Проверить».
window.addEventListener("contextmenu", (e) => {
  if (import.meta.env.DEV && e.shiftKey) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable='true']")) return;
  e.preventDefault();
});

// Сторож окна: чёрный экран после долгой работы → журнал + восстановление.
startUiWatchdog();

/*
  Каждое окно грузит только свой код. Трей-попап, уведомление и заставка
  обновления - те же index.html с ?view=..., но без страниц главного окна,
  мастера и диалогов: им достаётся небольшой отдельный кусок JS.
*/
function loadRoot(): Promise<ComponentType> {
  if (view === "tray") return import("./TrayRoot").then((m) => m.TrayRoot);
  if (view === "toast") return import("./pages/UpdateToast").then((m) => m.UpdateToast);
  if (view === "updating") return import("./pages/UpdateSplash").then((m) => m.UpdateSplash);
  return import("./App").then((m) => m.default);
}

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);
loadRoot().then(
  (Root) =>
    root.render(
      <React.StrictMode>
        <Root />
      </React.StrictMode>,
    ),
  (err) => {
    // Свой файл окна не загрузился (повреждена установка) - честно говорим.
    console.error(err);
    root.render(
      <div className="grid h-screen place-items-center p-6 text-center text-[13px] text-vb-silver">
        Окно не открылось. Перезапустите программу.
      </div>,
    );
  },
);
