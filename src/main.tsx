import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Трей-попап живёт в прозрачном окне: помечаем body, чтобы CSS убрал фон
// (само окно с transparent:true, скругление и тень рисует контент).
if (new URLSearchParams(window.location.search).get("view") === "tray") {
  document.body.dataset.view = "tray";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
