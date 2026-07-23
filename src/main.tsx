import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Трей-попап живёт в прозрачном окне: помечаем body, чтобы CSS убрал фон
// (само окно с transparent:true, скругление и тень рисует контент).
if (new URLSearchParams(window.location.search).get("view") === "tray") {
  document.body.dataset.view = "tray";
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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
