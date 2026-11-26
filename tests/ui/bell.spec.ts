import { expect, test } from "@playwright/test";

/*
  Колокольчик на стенде mock.html: ?notice - одна новая рассылка,
  ?notice=read - ещё и прочитанная.
*/

test("«Очистить» убирает прочитанные и остаётся без них скрытой", async ({ page }) => {
  await page.goto("/mock.html?still&notice=read");
  await page.getByRole("button", { name: /^Уведомления/ }).click({ timeout: 20_000 });
  const panel = page.getByRole("dialog", { name: "Уведомления" });
  await expect(panel.getByText("Свой прокси за 15 минут")).toBeVisible();
  await panel.getByRole("button", { name: "Очистить" }).click();
  await expect(panel.getByText("Свой прокси за 15 минут")).toHaveCount(0);
  // Непрочитанное осталось, а прочитанных больше нет - и кнопки тоже.
  await expect(panel.getByText("Вышла версия 0.9.1")).toBeVisible();
  await expect(panel.getByRole("button", { name: "Очистить" })).toHaveCount(0);

  // Прочитали последнее - его тоже можно убрать, список пустеет.
  await panel.getByRole("button", { name: "Прочитать все" }).click();
  await panel.getByRole("button", { name: "Очистить" }).click();
  await expect(panel.getByText("Пока ничего нового")).toBeVisible();
});

test("«Не беспокоить»: на час - видно до скольки, навсегда, выключить", async ({ page }) => {
  await page.goto("/mock.html?still&notice");
  await page.getByRole("button", { name: /^Уведомления/ }).click({ timeout: 20_000 });
  const panel = page.getByRole("dialog", { name: "Уведомления" });
  await panel.getByRole("button", { name: "Не беспокоить" }).click();
  await expect(panel.getByText("В колокольчике всё сохранится")).toBeVisible();
  await panel.getByRole("group", { name: "Не беспокоить на" }).getByRole("button", { name: "1 час" }).click();
  await expect(panel.getByText(/^Не беспокоить до \d\d:\d\d/)).toBeVisible();
  // Колокольчик в заголовке перечёркнут, пока режим включён.
  await expect(page.getByTestId("bell-quiet")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __quiet: { until: number } }).__quiet.until)).toBeGreaterThan(Date.now() / 1000 + 3500);

  await panel.getByRole("button", { name: "Не беспокоить" }).click();
  await panel.getByRole("button", { name: "Навсегда" }).click();
  await expect(panel.getByText("Не беспокоить включено")).toBeVisible();
  await panel.getByRole("button", { name: "Выключить" }).click();
  await expect(panel.getByText(/Не беспокоить (до|включено)/)).toHaveCount(0);
  await expect(page.getByTestId("bell-quiet")).toHaveCount(0);
});

test("без прочитанных кнопки «Очистить» нет", async ({ page }) => {
  await page.goto("/mock.html?still&notice");
  await page.getByRole("button", { name: /^Уведомления/ }).click({ timeout: 20_000 });
  const panel = page.getByRole("dialog", { name: "Уведомления" });
  await expect(panel.getByText("Вышла версия 0.9.1")).toBeVisible();
  await expect(panel.getByRole("button", { name: "Прочитать все" })).toBeVisible();
  await expect(panel.getByRole("button", { name: "Очистить" })).toHaveCount(0);
});
