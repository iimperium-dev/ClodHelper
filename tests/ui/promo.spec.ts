import { expect, test } from "@playwright/test";

/* Баннер на «Обзоре»: объявление из админки с оформлением. */

test("баннер виден, кнопка считает нажатие, крестик скрывает и считает закрытие", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 700, height: 720 });
  await page.goto("/mock.html?still&promo=violet");
  const banner = page.getByRole("region", { name: "Свой прокси за 5 минут" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  await expect(banner.getByText("Новое")).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__noticeEvents ?? [])).toContainEqual({ id: "promo-demo", ev: "shown" });

  await banner.getByRole("button", { name: "Смотреть" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__noticeEvents ?? [])).toContainEqual({ id: "promo-demo", ev: "clicked" });

  await banner.getByRole("button", { name: "Скрыть" }).click();
  await expect(banner).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).__noticeEvents ?? [])).toContainEqual({ id: "promo-demo", ev: "dismissed" });
  expect(errors).toEqual([]);
});

test("без баннера на «Обзоре» нет лишнего блока", async ({ page }) => {
  await page.goto("/mock.html?still");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("button", { name: "Скрыть" })).toHaveCount(0);
});
