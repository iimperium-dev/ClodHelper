import { expect, test, type Page } from "@playwright/test";

/*
  Мастер и приложения на стенде mock.html: Claude Desktop подключается на
  обеих ОС, недостающие Claude Desktop и Codex мастер предлагает поставить
  и ждёт установку. ?noclaude / ?nocodex - их нет; window.__install(id) -
  установка закончилась; window.__opened - какие магазины открывались;
  ?claudeerr - не подключился; ?claudeopen - был открыт.
*/

type W = Window & {
  __install: (id: string) => void;
  __opened?: string[];
  __noticeEvents?: unknown[];
  __noticeEventsHeld?: unknown[];
  __setupAsked?: { noticeSeen: boolean }[];
};

function collectErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  return errors;
}

async function throughInstall(page: Page, query: string) {
  await page.goto(`/mock.html?still&fresh${query}`);
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass", { timeout: 20_000 });
  await page.getByRole("button", { name: "Дальше" }).click();
}

const install = (page: Page, id: string) => page.evaluate((x) => (window as unknown as W).__install(x), id);
const opened = (page: Page) => page.evaluate(() => (window as unknown as W).__opened ?? []);

test("Windows: мастер подключает Claude Desktop, итог честный, «Открыть Claude»", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto("/mock.html?wizslow&fresh&platform=win");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass", { timeout: 20_000 });
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByText("Подключаю Claude Desktop - Windows спросит разрешение")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 15_000 });
  // Всё стоит - шага «Приложения» нет.
  await expect(page.getByText("Шаг 3 из 3")).toBeVisible();
  await expect(page.getByText("Claude Desktop подключён и откроется уже через мост")).toBeVisible();
  await page.getByRole("button", { name: "Открыть Claude" }).click();
  await expect(page.getByText("Открыт", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("Claude Desktop был открыт: «Перезапустить Claude»", async ({ page }) => {
  await throughInstall(page, "&platform=win&claudeopen");
  await expect(page.getByText("Claude Desktop подключён, нужен перезапуск")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Перезапустить Claude" }).click();
  await expect(page.getByText("Перезапущен")).toBeVisible();
  await expect(page.getByText("Claude Desktop подключён и идёт через мост")).toBeVisible();
});

test("Claude Desktop не подключился: мастер идёт дальше и говорит, где подключить", async ({ page }) => {
  await throughInstall(page, "&platform=win&claudeerr");
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Claude Desktop пока идёт напрямую. Подключите его в «Приложениях»")).toBeVisible();
  await expect(page.getByRole("button", { name: "Открыть Claude" })).toHaveCount(0);
});

test("нет обоих: «Установить» открывает магазин, «Жду установку…», затем «Установлено» и дальше сам", async ({ page }) => {
  const errors = collectErrors(page);
  await throughInstall(page, "&platform=win&noclaude&nocodex&wizslow");
  // На шаге установки Claude Desktop пропущен честно.
  await expect(page.getByText("Claude Desktop не установлен")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole("heading", { name: "Claude Desktop и Codex" })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Шаг 3 из 4")).toBeVisible();

  const claude = page.locator("[data-app=claude]");
  const codex = page.locator("[data-app=codex]");
  await claude.getByRole("button", { name: "Установить" }).click();
  await expect(claude.getByText("Жду установку…")).toBeVisible();
  await codex.getByRole("button", { name: "Установить" }).click();
  await expect(codex.getByText("Жду установку…")).toBeVisible();
  expect(await opened(page)).toEqual(["claude", "codex"]);
  // Пока ставится - можно пропустить.
  await expect(page.getByRole("button", { name: "Пропустить" })).toBeVisible();

  await install(page, "codex");
  await expect(codex.getByText("Установлено")).toBeVisible({ timeout: 8_000 });
  // Claude ещё ставится - мастер ждёт.
  await expect(claude.getByText("Жду установку…")).toBeVisible();
  await install(page, "claude");
  await expect(claude.getByText("Установлено и подключено")).toBeVisible({ timeout: 8_000 });
  // Ставить больше нечего - «Готово» само.
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 5_000 });
  await expect(page.getByText("Шаг 4 из 4")).toBeVisible();
  await expect(page.getByText("Claude Desktop подключён и откроется уже через мост")).toBeVisible();
  expect(errors).toEqual([]);
});

test("установка замечается и при возврате в окно, не только по таймеру", async ({ page }) => {
  await throughInstall(page, "&platform=win&nocodex");
  const codex = page.locator("[data-app=codex]");
  await codex.getByRole("button", { name: "Установить" }).click({ timeout: 10_000 });
  await install(page, "codex");
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(codex.getByText("Установлено")).toBeVisible({ timeout: 1_500 });
});

test("пропуск: Codex не ставим, Claude Desktop уже есть", async ({ page }) => {
  await throughInstall(page, "&platform=win&nocodex");
  await expect(page.locator("[data-app=claude]").getByText("Уже есть")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Пропустить" }).click();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible();
  expect(await opened(page)).toEqual([]);
});

test("Mac: Claude Desktop не поставили - строка без кнопки, подсказка про «Приложения»", async ({ page }) => {
  await throughInstall(page, "&noclaude");
  await expect(page.getByText("Подключаю Claude Desktop - macOS спросит пароль")).toHaveCount(0);
  await expect(page.getByText(/поставить сейчас с сайта разработчика/)).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Пропустить" }).click();
  await expect(page.getByText("Claude Desktop нет на компьютере. Поставите позже - подключите в «Приложениях»")).toBeVisible();
});

test("Claude Desktop поставили, но не подключился: можно позже", async ({ page }) => {
  await throughInstall(page, "&platform=win&noclaude&claudeerr");
  const claude = page.locator("[data-app=claude]");
  await claude.getByRole("button", { name: "Установить" }).click({ timeout: 10_000 });
  await install(page, "claude");
  await expect(claude.getByText("Установлено, подключить можно в «Приложениях»")).toBeVisible({ timeout: 8_000 });
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 5_000 });
  await expect(page.getByText("Claude Desktop пока идёт напрямую. Подключите его в «Приложениях»")).toBeVisible();
});

test("баннер мастера до согласия: показан, но показ и нажатие никуда не ушли", async ({ page }) => {
  await page.goto("/mock.html?still&fresh&promo&when=setup");
  const banner = page.getByRole("region", { name: "Свой прокси за 5 минут" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  // Спрашивали отдельной командой мастера, до показа переключателей.
  const asked = await page.evaluate(() => (window as unknown as W).__setupAsked ?? []);
  expect(asked.length).toBeGreaterThan(0);
  expect(asked.every((a) => a.noticeSeen === false)).toBe(true);
  await banner.getByRole("button", { name: "Смотреть" }).click();
  const held = () => page.evaluate(() => (window as unknown as W).__noticeEventsHeld ?? []);
  await expect.poll(held).toContainEqual({ id: "promo-demo", ev: "clicked" });
  expect(await page.evaluate(() => (window as unknown as W).__noticeEvents ?? [])).toEqual([]);
});
