import { expect, test, type Page } from "@playwright/test";

/*
  «Открыть» у Claude Desktop, открытого без прокси: вместо ошибки «Claude
  Desktop уже запущен…» - вопрос с кнопкой «Перезапустить». Это не сбой и в
  отчёты не уходит. ?claudedirect - Claude открыт без прокси, ?relaunchfail -
  перезапуск не вышел. window.__launched / __relaunched / __uiErrors - для проверки.
*/

type W = Window & { __launched?: string[]; __relaunched?: string[]; __uiErrors: unknown[] };

const OFFER = "Claude Desktop открыт без прокси. Перезапустить его с прокси?";
const NOT_CLOSED = "Claude Desktop не закрылся - закройте его и нажмите «Открыть»";

async function openApps(page: Page, query: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto(`/mock.html?still&platform=win${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("tab", { name: "Приложения" }).click();
  await expect(page.getByText("Claude Desktop", { exact: true }).first()).toBeVisible();
  return errors;
}

const w = (page: Page) =>
  page.evaluate(() => {
    const x = window as unknown as W;
    return { launched: x.__launched ?? [], relaunched: x.__relaunched ?? [], uiErrors: x.__uiErrors };
  });

/** Кнопка «Открыть» в строке Claude Desktop. */
const openClaudeRow = (page: Page) =>
  page
    .locator("div.px-4.py-3")
    .filter({ has: page.getByText("Claude Desktop", { exact: true }) })
    .getByRole("button", { name: "Открыть" });

test("список: Claude открыт без прокси - вопрос и «Перезапустить», без отчёта об ошибке", async ({ page }) => {
  const errors = await openApps(page, "&claudedirect");
  await openClaudeRow(page).click();
  const note = page.getByRole("status").filter({ hasText: OFFER });
  await expect(note).toBeVisible();
  // Старой просьбы закрыть через трей больше нет.
  await expect(page.getByText(/через значок в трее/)).toHaveCount(0);
  await note.getByRole("button", { name: "Перезапустить" }).click();
  await expect(note).toHaveCount(0);
  const s = await w(page);
  expect(s.relaunched).toEqual(["claude-desktop"]);
  expect(s.uiErrors).toEqual([]);
  // После перезапуска «Открыть» просто открывает.
  await openClaudeRow(page).click();
  await expect(page.getByText(OFFER)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("перезапуск не вышел: «не закрылся - закройте его и нажмите «Открыть»», без кнопки", async ({ page }) => {
  await openApps(page, "&claudedirect&relaunchfail");
  await openClaudeRow(page).click();
  await page.getByRole("status").getByRole("button", { name: "Перезапустить" }).click();
  const alert = page.getByRole("alert").filter({ hasText: NOT_CLOSED });
  await expect(alert).toBeVisible();
  await expect(alert.getByRole("button", { name: "Перезапустить" })).toHaveCount(0);
  await alert.getByRole("button", { name: "Скрыть" }).click();
  await expect(alert).toHaveCount(0);
});

test("плитки: то же предложение по нажатию на плитку, ничего не уходит как сбой интерфейса", async ({ page }) => {
  const errors = await openApps(page, "&claudedirect");
  await page.getByRole("radio", { name: "Сетка" }).click();
  await page.getByText("Claude Desktop", { exact: true }).first().click();
  const note = page.getByRole("status").filter({ hasText: OFFER });
  await expect(note).toBeVisible();
  expect((await w(page)).launched).toEqual(["claude-desktop"]);
  await note.getByRole("button", { name: "Перезапустить" }).click();
  await expect(note).toHaveCount(0);
  expect((await w(page)).uiErrors).toEqual([]);
  expect(errors).toEqual([]);
});

test("плитки: настоящая ошибка перезапуска видна на странице", async ({ page }) => {
  await openApps(page, "&claudedirect&relaunchfail");
  await page.getByRole("radio", { name: "Сетка" }).click();
  await page.getByText("Claude Desktop", { exact: true }).first().click();
  await page.getByRole("status").getByRole("button", { name: "Перезапустить" }).click();
  await expect(page.getByRole("alert").filter({ hasText: NOT_CLOSED })).toBeVisible();
  expect((await w(page)).uiErrors).toEqual([]);
});

test("мастер: «Открыть Claude» у открытого без прокси - кнопка становится «Перезапустить Claude»", async ({ page }) => {
  await page.goto("/mock.html?still&fresh&platform=win&claudedirect");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass", { timeout: 20_000 });
  await page.getByRole("button", { name: "Дальше" }).click();
  await page.getByRole("button", { name: "Открыть Claude" }).click({ timeout: 10_000 });
  await expect(page.getByText("Claude Desktop подключён, нужен перезапуск")).toBeVisible();
  await page.getByRole("button", { name: "Перезапустить Claude" }).click();
  await expect(page.getByText("Перезапущен")).toBeVisible();
});
