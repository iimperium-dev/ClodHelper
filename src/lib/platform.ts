/*
  Платформа, на которой запущено приложение. WebView macOS (WKWebView)
  отдаёт «Macintosh» в userAgent — отдельный плагин ради одного флага не нужен.
*/
export const isMac = /Macintosh|Mac OS X/.test(navigator.userAgent);

/** Название ОС для текстов интерфейса («при входе в Windows/macOS»). */
export const osName = isMac ? "macOS" : "Windows";
