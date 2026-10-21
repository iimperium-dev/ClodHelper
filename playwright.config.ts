import { defineConfig, devices } from "@playwright/test";

// Тесты интерфейса на стенде mock.html (подставной бэкенд Tauri): страницы
// открываются без ошибок, ключевые сценарии работают, снимки — в test-results.
export default defineConfig({
  // В CI первый заход ждёт компиляцию vite — один повтор от случайных таймаутов.
  retries: process.env.CI ? 1 : 0,
  testDir: "tests/ui",
  outputDir: "test-results",
  timeout: 30_000,
  fullyParallel: true,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:1420",
    viewport: { width: 700, height: 580 },
    deviceScaleFactor: 2,
    colorScheme: "dark",
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: "npx vite --port 1420 --strictPort",
    url: "http://localhost:1420/mock.html",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
