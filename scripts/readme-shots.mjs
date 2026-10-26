// Снимки интерфейса для README со стенда mock.html (подставные данные,
// режим ?readme — нейтральные подписи, без баннера обновления).
// Запуск: npx vite --port 1420 (в другом окне), затем node scripts/readme-shots.mjs
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";

const OUT = ".github/assets";
mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch();
// Размер окна программы по умолчанию (tauri.conf.json): 700×580.
const page = await browser.newPage({
  viewport: { width: 760, height: 600 },
  deviceScaleFactor: 2,
  colorScheme: "dark",
});
await page.goto("http://localhost:1420/mock.html?still&platform=win&readme");
await page.waitForTimeout(1500);

const nav = async (label) => {
  await page.getByRole("tab", { name: label }).click();
  await page.waitForTimeout(900);
};

const shots = process.argv.slice(2).length ? process.argv.slice(2) : ["overview", "proxies", "sites"];
for (const s of shots) {
  if (s === "overview") {
    await nav("Обзор");
    await page.getByRole("button", { name: "Проверить" }).click();
    await page.waitForTimeout(1200);
  }
  if (s === "proxies") await nav("Прокси");
  if (s === "sites") await nav("Сайты");
  if (s === "tools") await nav("Инструменты");
  await page.screenshot({ path: `${OUT}/${s}.png` });
  console.log("✓", s);
}
await browser.close();
