import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { startUiWatchdog } from "./lib/uiWatchdog";

// Трей-попап и уведомление об обновлении живут в прозрачных окнах: помечаем
// body, чтобы CSS убрал фон (скругление и тень рисует контент).
const view = new URLSearchParams(window.location.search).get("view");
if (view === "tray" || view === "toast" || view === "updating") {
  document.body.dataset.view = view;
}

// Нативное контекстное меню WebView2 («Назад/Печать/Проверить») чужеродно для
// десктоп-приложения — глушим везде, кроме полей ввода (там нужны
// Копировать/Вставить). В dev Shift+ПКМ оставляет доступ к «Проверить».
window.addEventListener("contextmenu", (e) => {
  if (import.meta.env.DEV && e.shiftKey) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable='true']")) return;
  e.preventDefault();
});

// Сторож окна: чёрный экран после долгой работы → журнал + восстановление.
startUiWatchdog();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
