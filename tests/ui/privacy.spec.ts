import { expect, test, type Page } from "@playwright/test";

/*
  Приватность и отчёты на стенде mock.html: три переключателя в мастере и
  настройках, «Сообщить о проблеме», два варианта окна для обновившихся
  (?variant=a|b), отправка ошибок интерфейса.
*/

type W = {
  __privacy: () => Record<string, boolean>;
  __reports: { text: string; contact: string | null }[];
  __uiErrors: { msg: string; stack: string }[];
};
const privacy = (page: Page) => page.evaluate(() => (window as unknown as W).__privacy());
const reports = (page: Page) => page.evaluate(() => (window as unknown as W).__reports);

async function openSettings(page: Page, query = "") {
  await page.goto(`/mock.html?still&platform=win${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("tab", { name: "Настройки" }).click();
}

async function openReport(page: Page, query = "") {
  await openSettings(page, query);
  const btn = page.getByRole("button", { name: "Сообщить о проблеме" });
  await btn.click();
  const dialog = page.getByRole("dialog", { name: "Сообщить о проблеме" });
  await expect(dialog).toBeVisible();
  return { btn, dialog };
}

test("мастер: три переключателя включены и записаны, выключение пишется", async ({ page }) => {
  await page.goto("/mock.html?still&fresh&platform=win");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 10_000 });
  for (const n of ["Обновлять автоматически", "Отправлять отчёты об ошибках", "Отправлять анонимную статистику"]) {
    await expect(page.getByRole("switch", { name: n })).toHaveAttribute("aria-checked", "true");
  }
  expect(await privacy(page)).toEqual({ auto_update: true, crash_reports: true, usage_stats: true });
  await expect(page.getByRole("button", { name: "Открыть Clod Helper" })).toBeFocused();
  await page.getByRole("switch", { name: "Отправлять отчёты об ошибках" }).click();
  expect(await privacy(page)).toEqual({ auto_update: true, crash_reports: false, usage_stats: true });
});

test("настройки: три переключателя с пояснениями", async ({ page }) => {
  await openSettings(page);
  const stats = page.getByRole("switch", { name: "Отправлять анонимную статистику" });
  await expect(page.getByRole("switch", { name: "Обновлять автоматически" })).toBeVisible();
  await expect(page.getByRole("switch", { name: "Отправлять отчёты об ошибках" })).toBeVisible();
  await expect(stats).toHaveAttribute("aria-describedby", /.+/);
  await stats.click();
  await expect(stats).toHaveAttribute("aria-checked", "false");
  expect((await privacy(page)).usage_stats).toBe(false);
  await expect(page.getByText("Версия и система раз в день. Без адресов и настроек прокси")).toBeVisible();
});

test("отчёт: успех", async ({ page }) => {
  const { dialog } = await openReport(page);
  const field = dialog.getByLabel("Что случилось");
  await expect(field).toBeFocused();
  const send = dialog.getByRole("button", { name: "Отправить" });
  await expect(send).toBeDisabled();
  await field.fill("Не открывается окно");
  await dialog.getByLabel("Как с вами связаться").fill("@name");
  await send.click();
  await expect(dialog.getByText("Отправлено. Спасибо")).toBeVisible();
  const close = dialog.getByRole("button", { name: "Закрыть" });
  await expect(close).toBeFocused();
  expect(await reports(page)).toEqual([{ text: "Не открывается окно", contact: "@name" }]);
  await close.click();
  await expect(dialog).toHaveCount(0);
});

test("отчёт: ошибка - alert, «Повторить», текст на месте", async ({ page }) => {
  const { dialog } = await openReport(page, "&reportfail");
  await dialog.getByLabel("Что случилось").fill("Сломалось");
  await dialog.getByRole("button", { name: "Отправить" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Не получилось отправить");
  await expect(dialog.getByRole("button", { name: "Повторить" })).toBeEnabled();
  await expect(dialog.getByLabel("Что случилось")).toHaveValue("Сломалось");
  expect(await reports(page)).toEqual([]);
});

test("отчёт: Esc закрывает и возвращает фокус на кнопку", async ({ page }) => {
  const { btn, dialog } = await openReport(page);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(btn).toBeFocused();
});

test("отчёт: двойной клик по «Отправить» - одна отправка", async ({ page }) => {
  const { dialog } = await openReport(page);
  await dialog.getByLabel("Что случилось").fill("Раз");
  await dialog.getByRole("button", { name: "Отправить" }).dblclick();
  await expect(dialog.getByText("Отправлено. Спасибо")).toBeVisible();
  expect(await reports(page)).toHaveLength(1);
});

test("предложение, вариант a: переключатель статистики пишется", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto("/mock.html?still&autooffer&variant=a&platform=win");
  const dialog = page.getByRole("dialog", { name: "Автообновление. Включить?" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await expect(dialog.getByText("Ещё с этой версии")).toBeVisible();
  const stats = dialog.getByRole("switch", { name: "Отправлять анонимную статистику" });
  await expect(stats).toHaveAttribute("aria-checked", "true");
  await stats.click();
  expect((await privacy(page)).usage_stats).toBe(false);
  expect(await dialog.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(false);
});

test("предложение, вариант b: ссылка ведёт в настройки", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto("/mock.html?still&autooffer&variant=b&platform=win");
  const dialog = page.getByRole("dialog", { name: "Автообновление. Включить?" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await expect(dialog.getByRole("switch")).toHaveCount(0);
  expect(await dialog.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(false);
  await dialog.getByRole("button", { name: "в настройках" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Обновления и отчёты" })).toBeVisible();
});

test("ошибка интерфейса уходит в отчёт", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.evaluate(() => {
    setTimeout(() => {
      throw new Error("тестовая ошибка интерфейса");
    }, 0);
  });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as W).__uiErrors.map((e) => e.msg)))
    .toContainEqual(expect.stringContaining("тестовая ошибка интерфейса"));
});

test("мастер на 700x580: все три переключателя и кнопка видны без прокрутки", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto("/mock.html?still&fresh&platform=win");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 10_000 });
  for (const n of ["Обновлять автоматически", "Отправлять отчёты об ошибках", "Отправлять анонимную статистику"]) {
    await expect(page.getByRole("switch", { name: n })).toBeInViewport({ ratio: 1 });
  }
  const open = page.getByRole("button", { name: "Открыть Clod Helper" });
  await expect(open).toBeInViewport({ ratio: 1 });
  // Третий переключатель не прячется под панелью с кнопкой.
  const last = (await page.getByRole("switch", { name: "Отправлять анонимную статистику" }).boundingBox())!;
  const btn = (await open.boundingBox())!;
  expect(last.y + last.height).toBeLessThan(btn.y);
});
