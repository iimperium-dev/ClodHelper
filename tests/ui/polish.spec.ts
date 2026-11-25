import { expect, test, type Page } from "@playwright/test";

// Подтверждения в два клика, индикация действий, ошибки, которые не гаснут
// раньше, чем их прочитали, и доступность окон (0.9.3).

async function open(page: Page, query = "still", size = { width: 700, height: 580 }) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize(size);
  await page.goto(`/mock.html?${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.evaluate(() => document.fonts.ready);
  return errors;
}

const go = (page: Page, label: string) => page.getByRole("tab", { name: label }).click();
const ask = (page: Page) => page.getByRole("group", { name: "Подтверждение" });

test("приложения: «Убрать из списка» спрашивает, Esc и клик мимо - отмена", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  const chrome = page.getByText("Google Chrome", { exact: true });
  await chrome.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Убрать из списка" }).click();
  await expect(ask(page).getByText("Убрать Google Chrome из списка?")).toBeVisible();
  // Своё приложение без защиты - лишнего предупреждения нет.
  await expect(ask(page).getByText("Защита от прямого соединения снимется")).toHaveCount(0);
  // Фокус - на «Отмена»: Enter по привычке ничего не убирает.
  await expect(ask(page).getByRole("button", { name: "Отмена" })).toBeFocused();
  await page.screenshot({ path: "test-results/apps-remove-confirm.png" });
  await page.keyboard.press("Escape");
  await expect(ask(page)).toHaveCount(0);
  await expect(chrome).toBeVisible();

  await chrome.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Убрать из списка" }).click();
  await expect(ask(page)).toBeVisible();
  // Висит, пока не ответили (у прокси раньше сам уходил через 4 с).
  await page.waitForTimeout(4500);
  await expect(ask(page)).toBeVisible();
  await page.getByRole("heading", { name: "Приложения" }).click();
  await expect(ask(page)).toHaveCount(0);
  await expect(chrome).toBeVisible();
  expect(errors).toEqual([]);
});

test("приложения: Claude - предупреждение о защите, ошибка видна и строка остаётся", async ({ page }) => {
  const errors = await open(page, "still&removefail");
  await go(page, "Приложения");
  await page.getByText("Claude Desktop", { exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Убрать из списка" }).click();
  await expect(ask(page).getByText("Защита от прямого соединения снимется")).toBeVisible();
  await ask(page).getByRole("button", { name: "Убрать" }).click();
  // Раньше ошибка проглатывалась: строка молча оставалась на месте.
  await expect(ask(page).getByRole("alert")).toHaveText("Нет прав, чтобы снять защиту от прямого соединения");
  await expect(page.getByText("Claude Desktop", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("приложения: окно настроек - убрать в два клика, Esc снимает только вопрос", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByText("Telegram", { exact: true }).first().hover();
  const gearBtn = page.getByRole("button", { name: "Настройки - Telegram" });
  await gearBtn.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog.getByRole("button", { name: "Закрыть" })).toBeFocused();
  await dialog.locator(".overflow-y-auto").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await dialog.getByText("Убрать из списка").click();
  await expect(ask(page).getByText("Убрать Telegram из списка?")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(ask(page)).toHaveCount(0);
  await expect(dialog).toBeVisible();
  // Фокус вернулся туда, откуда спросили.
  await expect(dialog.getByRole("button", { name: "Убрать из списка" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  // Фокус вернулся на шестерёнку.
  await expect(gearBtn).toBeFocused();

  await gearBtn.click();
  await dialog.locator(".overflow-y-auto").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await dialog.getByText("Убрать из списка").click();
  await ask(page).getByRole("button", { name: "Убрать" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Telegram", { exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("приложения: в сетке «−» тоже спрашивает", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByRole("radio", { name: "Сетка" }).click();
  await page.getByText("VS Code", { exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Выбрать" }).click();
  await page.getByRole("button", { name: "Убрать VS Code" }).click();
  await expect(ask(page).getByText("Убрать VS Code из списка?")).toBeVisible();
  await page.screenshot({ path: "test-results/apps-grid-remove-confirm.png" });
  await ask(page).getByRole("button", { name: "Убрать" }).click();
  await expect(page.getByText("VS Code", { exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("прокси: вопрос об удалении не исчезает сам", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Прокси");
  // Кнопки строки появляются при наведении.
  await page.getByText("93.1.2.3", { exact: true }).first().hover();
  await page.getByRole("button", { name: "Удалить", exact: true }).last().click();
  await expect(ask(page).getByText(/Удалить прокси\?/)).toBeVisible();
  await page.waitForTimeout(4500);
  await expect(ask(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(ask(page)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("обзор: «Остановить мост» со спиннером и подписью", async ({ page }) => {
  const errors = await open(page);
  await page.getByRole("button", { name: "Остановить мост" }).click();
  await expect(page.getByRole("button", { name: "Останавливаю мост" })).toBeVisible();
  await expect(page.getByText("Останавливаю…")).toBeVisible();
  await expect(page.getByText("Мост остановлен")).toBeVisible({ timeout: 5000 });
  await expect(page.getByText("Приложения идут напрямую")).toBeVisible();
  expect(errors).toEqual([]);
});

test("обзор: проверка утечек не ответила, а связь есть - строка «Связь есть»", async ({ page }) => {
  const errors = await open(page, "still&checkfail");
  await page.getByRole("button", { name: "Проверить" }).click();
  await expect(page.getByText("Связь есть, ответ за 420 мс")).toBeVisible();
  await expect(page.getByText("· проверка утечек не ответила")).toBeVisible();
  // IP по-прежнему один раз - в строке выхода.
  await expect(page.getByText("185.68.187.138", { exact: true })).toHaveCount(1);
  expect(errors).toEqual([]);
});

test("обзор: запасные прокси - понятной фразой", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByText("Выход через мост · ещё 1 в запасе")).toBeVisible();
  expect(errors).toEqual([]);
});

test("настройки: мост не запускается сам - кнопка «Исправить»", async ({ page }) => {
  const errors = await open(page, "still&notask");
  await go(page, "Настройки");
  const row = page.getByText("Мост запускается сам").locator("xpath=../..");
  await expect(row.getByText("нет", { exact: true })).toBeVisible();
  await row.getByRole("button", { name: "Исправить" }).click();
  await expect(row.getByText("да", { exact: true })).toBeVisible({ timeout: 5000 });
  await expect(row.getByRole("button", { name: "Исправить" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("настройки: «Исправить» не вышло - причина видна", async ({ page }) => {
  const errors = await open(page, "still&notask&taskfail");
  await go(page, "Настройки");
  await page.getByRole("button", { name: "Исправить" }).click();
  await expect(page.getByRole("alert")).toHaveText("Планировщик заданий отказал: нет прав администратора");
  expect(errors).toEqual([]);
});

test("трей: ошибка теста переносится и не гаснет, подписи метрик без капса", async ({ page }) => {
  const errors = await open(page, "still&view=tray&testfail", { width: 332, height: 420 });
  const label = page.getByText("напрямую", { exact: true });
  expect(await label.evaluate((el) => getComputedStyle(el).textTransform)).toBe("none");
  expect(await label.evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(10);
  await page.getByRole("button", { name: "Тест соединения" }).click();
  const err = page.getByText(/не ответил за 10 секунд/);
  await expect(err).toBeVisible();
  // Целиком, обычным шрифтом, в несколько строк.
  const st = await err.evaluate((el) => ({
    font: getComputedStyle(el).fontFamily,
    clipped: el.scrollWidth > el.clientWidth,
    h: el.getBoundingClientRect().height,
  }));
  expect(st.font).not.toMatch(/Mono/);
  expect(st.clipped).toBe(false);
  expect(st.h).toBeGreaterThan(20);
  await page.waitForTimeout(6500);
  await expect(err).toBeVisible();
  await page.screenshot({ path: "test-results/tray-test-error.png" });
  expect(errors).toEqual([]);
});

test("трей: остановка моста со спиннером", async ({ page }) => {
  const errors = await open(page, "still&view=tray", { width: 332, height: 420 });
  await page.getByRole("button", { name: "Остановить мост" }).click();
  await expect(page.getByRole("button", { name: "Останавливаю мост" })).toBeVisible();
  await expect(page.getByText("Мост остановлен")).toBeVisible({ timeout: 5000 });
  await expect(page.getByText("приложения идут напрямую")).toBeVisible();
  expect(errors).toEqual([]);
});

test("сайты: ошибка не гаснет за 4 с и закрывается крестиком", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Сайты");
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Адрес или ссылка: example.com").fill("не сайт");
  await page.getByPlaceholder("Адрес или ссылка: example.com").press("Enter");
  const toast = page.getByRole("alert").filter({ hasText: "не похоже на адрес сайта" });
  await expect(toast).toBeVisible();
  await page.waitForTimeout(4500);
  await expect(toast).toBeVisible();
  await toast.getByRole("button", { name: "Закрыть подсказку" }).click();
  await expect(toast).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("окна: «Что нового» - фокус на «Отлично», Esc закрывает", async ({ page }) => {
  const errors = await open(page, "still&whatsnew");
  const dialog = page.getByRole("dialog", { name: "Что нового" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog.getByRole("button", { name: "Отлично" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Отлично" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  expect(errors).toEqual([]);
});

test("окна: обновление - фокус на «Обновить», после закрытия - обратно", async ({ page }) => {
  const errors = await open(page);
  const open_ = page.getByRole("button", { name: "Обновить" }).first();
  await open_.click();
  const dialog = page.getByRole("dialog", { name: "Обновление Clod Helper" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog.getByRole("button", { name: "Обновить" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(open_).toBeFocused();
  expect(errors).toEqual([]);
});
