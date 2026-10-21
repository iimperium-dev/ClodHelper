import { expect, test, type Page } from "@playwright/test";

/** Открыть стенд, собрать ошибки консоли. */
async function open(page: Page, query = "still") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto(`/mock.html?${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible();
  return errors;
}

/** Открыть окно настроек приложения через шестерёнку на иконке. */
async function gear(page: Page, name: string) {
  await page.getByText(name, { exact: true }).first().hover();
  await page.getByRole("button", { name: `Настройки — ${name}` }).click();
}

async function go(page: Page, label: string) {
  await page.getByRole("tab", { name: label }).click();
}

test("обзор: IP один раз, проверка приватности с вердиктом", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByText("185.68.187.138")).toHaveCount(1);
  await expect(page.getByText("Доступна версия 0.7.0")).toBeVisible();
  await page.getByRole("button", { name: "Проверить" }).click();
  await expect(page.getByText("Всё чисто")).toBeVisible();
  await expect(page.getByText("Через мост ваш IP скрыт")).toBeVisible();
  await expect(page.getByText("DNS не выдаёт провайдера")).toBeVisible();
  await expect(page.getByText("Приложения из вашего списка — через мост")).toBeVisible();
  await expect(page.getByText("Остальные программы работают напрямую, как обычно")).toBeVisible();
  await page.screenshot({ path: "test-results/overview.png" });
  expect(errors).toEqual([]);
});

test("приложения: папки, статусы, перехват расширением", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  for (const folder of ["AI", "Браузеры", "Мессенджеры", "Разработка"]) {
    await expect(page.getByText(folder, { exact: true })).toBeVisible();
  }
  await expect(page.getByText("Прокси перехватывает FoxyProxy")).toBeVisible();
  await expect(page.getByRole("button", { name: "Отключить", exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-list.png", fullPage: true });
  expect(errors).toEqual([]);
});

test("приложения: сетка и режим упорядочивания", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByRole("radio", { name: "Сетка" }).click();
  await expect(page.getByText("Telegram")).toBeVisible();
  await page.screenshot({ path: "test-results/apps-grid.png", fullPage: true });
  // ПКМ → меню → «Упорядочить»
  await page.getByText("Telegram").click({ button: "right" });
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  // Меню целиком в окне (раньше обрезалось снизу).
  const box = await menu.boundingBox();
  expect(box && box.y + box.height).toBeLessThanOrEqual(580);
  await expect(page.getByRole("menuitem", { name: "Убрать из списка" })).toBeInViewport();
  await page.screenshot({ path: "test-results/apps-context-menu.png" });
  await page.getByRole("menuitem", { name: "Упорядочить" }).click();
  await expect(page.getByRole("button", { name: "Готово" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Новая папка" })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-editing.png", fullPage: true });
  await page.getByRole("button", { name: "Готово" }).click();
  await expect(page.getByRole("button", { name: "Добавить" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("приложения: окно настроек через шестерёнку", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await gear(page, "Google Chrome");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Прокси", { exact: true })).toBeVisible();
  await expect(dialog.getByText("Всегда через мост")).toBeVisible();
  await expect(dialog.getByText("Весь трафик через прокси")).toBeVisible();
  await page.screenshot({ path: "test-results/app-settings.png" });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  expect(errors).toEqual([]);
});

test("приложения: умное добавление", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Название программы").fill("wec");
  await expect(page.getByText("WeChat")).toBeVisible();
  await page.screenshot({ path: "test-results/apps-add.png" });
  expect(errors).toEqual([]);
});

test("прокси, настройки и сайдбар", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Прокси");
  await page.waitForTimeout(800);
  await page.screenshot({ path: "test-results/proxies.png", fullPage: true });
  await expect(page.getByText("Claude main").first()).toBeVisible();
  await go(page, "Настройки");
  await expect(page.getByText("Открывать при входе")).toBeVisible();
  await page.screenshot({ path: "test-results/settings.png" });
  await page.getByRole("button", { name: "Развернуть панель" }).click();
  await expect(page.getByText("AI-прокси мост")).toBeVisible();
  await page.screenshot({ path: "test-results/sidebar-open.png" });
  expect(errors).toEqual([]);
});

test("windows: статусы по соединениям, Telegram и вид плитками", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await expect(page.getByText("Telegram").first()).toBeVisible();
  await expect(page.getByText("Через мост · 4").first()).toBeVisible();
  await expect(page.getByText("Прокси перехватывает FoxyProxy").first()).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Вид" })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-windows.png" });
  expect(errors).toEqual([]);
});

test("обзор без тумблеров; режим Claude Code в терминале — в окне Claude", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByText("Умная маршрутизация")).toHaveCount(0);
  await go(page, "Приложения");
  await expect(page.getByText("Терминал", { exact: true })).toHaveCount(0);
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Claude Code в терминале и VS Code")).toBeVisible();
  await dialog.getByText("Через прокси всё").click();
  await expect(dialog.getByText("Сохранено — действует в новых окнах терминала")).toBeVisible();
  await page.screenshot({ path: "test-results/claude-code-modes.png" });
  await dialog.getByText("Выключено", { exact: true }).click();
  await expect(dialog.getByText("Выключено — новые окна терминала пойдут напрямую")).toBeVisible();
  expect(errors).toEqual([]);
});

test("защита от утечек — в окне Claude и Telegram, не в Настройках", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Настройки");
  await expect(page.getByText("Открывать при входе")).toBeVisible();
  await expect(page.getByText("Защита от утечек")).toHaveCount(0);
  await go(page, "Приложения");
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("switch", { name: "Не пускать мимо прокси" })).toBeChecked();
  await page.screenshot({ path: "test-results/claude-guard.png" });
  await page.keyboard.press("Escape");
  await gear(page, "Telegram");
  const tg = dialog.getByRole("switch", { name: "Не пускать мимо прокси" });
  await expect(tg).not.toBeChecked();
  await tg.click();
  await expect(tg).toBeChecked();
  expect(errors).toEqual([]);
});

test("меню в строке меню: только статус и действия", async ({ page }) => {
  const errors = await open(page, "still&view=tray");
  await expect(page.getByRole("button", { name: "Тест соединения" })).toBeVisible();
  await expect(page.getByText("Терминал")).toHaveCount(0);
  await page.screenshot({ path: "test-results/tray.png" });
  expect(errors).toEqual([]);
});

test("windows: защиты от утечек нет", async ({ page }) => {
  await open(page, "still&platform=win");
  await go(page, "Приложения");
  await gear(page, "Claude Desktop");
  await expect(page.getByRole("dialog").getByText("Режим", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Не пускать мимо прокси")).toHaveCount(0);
});

test("сторож окна: прозрачная страница восстанавливается и пишется в журнал", async ({ page }) => {
  await open(page);
  // Имитируем недоигранную анимацию: страница и её секции прозрачны.
  await page.evaluate(() => {
    const p = document.querySelector("main")!.firstElementChild as HTMLElement;
    p.style.opacity = "0";
  });
  const pageOpacity = () =>
    page.evaluate(() => getComputedStyle(document.querySelector("main")!.firstElementChild!).opacity);
  expect(await pageOpacity()).toBe("0");
  await expect.poll(pageOpacity, { timeout: 6000 }).toBe("1");
  const misses = await page.evaluate(() => (window as unknown as { __mockMisses?: string[] }).__mockMisses ?? []);
  expect(misses).toContain("ui_log");
});
