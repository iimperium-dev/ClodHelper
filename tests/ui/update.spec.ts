import { expect, test, type Page } from "@playwright/test";

/** Открыть стенд, собрать ошибки консоли. */
async function open(page: Page, query: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto(`/mock.html?${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  return errors;
}

test("обновление: автообновление включено - честный текст про свёрнутую программу", async ({ page }) => {
  const errors = await open(page, "still");
  await expect(page.getByText("Поставится само, когда программа будет свёрнута")).toBeVisible();
  await page.getByRole("button", { name: "Обновить", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Обновление Clod Helper" });
  await expect(dialog.getByTestId("update-auto-note")).toContainText("поставится само");
  expect(errors).toEqual([]);
});

test("обновление: автообновление выключено - обычный текст, заметки нет", async ({ page }) => {
  const errors = await open(page, "still&autooffer");
  // Предложение автообновления закрываем отказом: auto_update = false.
  await page.getByRole("button", { name: "Не сейчас" }).click();
  await expect(page.getByText("обновление в один клик")).toBeVisible();
  await page.getByRole("button", { name: "Обновить", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Обновление Clod Helper" });
  await expect(dialog.getByText("Доступна версия 0.7.0")).toBeVisible();
  await expect(dialog.getByTestId("update-auto-note")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("обновление: слишком старая установка - ручная установка со страницы релиза", async ({ page }) => {
  const errors = await open(page, "still&manual");
  await expect(page.getByText("Поставьте вручную со страницы релиза")).toBeVisible();
  await page.getByRole("button", { name: "Подробнее", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Обновление Clod Helper" });
  await expect(dialog.getByTestId("update-manual-note")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Открыть страницу" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Обновить", exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("обновление: уже идёт - без «Повторить» и без красной ошибки", async ({ page }) => {
  const errors = await open(page, "still&busyupd");
  await page.getByRole("button", { name: "Обновить", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Обновление Clod Helper" });
  await dialog.getByRole("button", { name: "Обновить", exact: true }).click();
  await expect(dialog.getByText("Обновление уже идёт")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Повторить" })).toHaveCount(0);
  await expect(dialog.getByText("Не получилось обновить")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Закрыть", exact: true }).first().click();
  await expect(dialog).toHaveCount(0);
  expect(errors).toEqual([]);
});
