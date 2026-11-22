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

test("обложка открывает окно просмотра: листание, видео, кнопки, закрытие", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 700, height: 720 });
  await page.goto("/mock.html?still&promo&media");
  const banner = page.getByRole("region", { name: "Свой прокси за 5 минут" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  // Обложка - первое фото из кэша программы, счётчик слайдов.
  await expect(banner.locator("img[src='/mock-assets/cover.jpg']")).toBeVisible();
  await expect(banner.getByText("1 / 3")).toBeVisible();

  await banner.getByRole("button", { name: "Фото и видео" }).click();
  const dlg = page.getByRole("dialog", { name: "Свой прокси за 5 минут" });
  await expect(dlg).toBeVisible();
  await expect(dlg.locator("img[src='/mock-assets/cover.jpg']")).toBeVisible();

  // Вперёд клавишей - видео со своим плеером.
  await page.keyboard.press("ArrowRight");
  await expect(dlg.locator("video[src='/mock-assets/clip.mp4']")).toBeVisible();
  await expect(dlg.getByRole("button", { name: "Слайд 2" })).toHaveAttribute("aria-current", "true");
  await dlg.getByRole("button", { name: "Смотреть" }).first().click();
  await expect(dlg.getByRole("slider", { name: "Перемотка" })).toBeVisible();

  // Кнопкой - третье фото, по кругу - снова первое.
  await dlg.getByRole("button", { name: "Дальше" }).click();
  await expect(dlg.locator("img[src='/mock-assets/photo2.jpg']")).toBeVisible();
  await dlg.getByRole("button", { name: "Дальше" }).click();
  await expect(dlg.getByRole("button", { name: "Слайд 1" })).toHaveAttribute("aria-current", "true");

  // Кнопка баннера в окне считает нажатие.
  await dlg.getByRole("button", { name: "Подробнее" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__noticeEvents ?? [])).toContainEqual({ id: "promo-demo", ev: "clicked" });

  await page.keyboard.press("Escape");
  await expect(dlg).toHaveCount(0);
  await expect(banner).toBeVisible();
  expect(errors).toEqual([]);
});

test("без обложки - кнопка «Посмотреть», файлы качаются только при открытии", async ({ page }) => {
  await page.goto("/mock.html?still&promo&media=nocover");
  const banner = page.getByRole("region", { name: "Свой прокси за 5 минут" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  await expect(banner.locator("img")).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__mediaAsked ?? [])).toEqual([]);

  await banner.getByRole("button", { name: "Посмотреть" }).click();
  const dlg = page.getByRole("dialog");
  await expect(dlg.locator("img[src='/mock-assets/cover.jpg']")).toBeVisible();
  await dlg.getByRole("button", { name: "Закрыть" }).click();
  await expect(dlg).toHaveCount(0);
});

test("баннер по ситуации: только пока не работает ни один прокси", async ({ page }) => {
  await page.goto("/mock.html?still&promo&when=proxy_down");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(800);
  await expect(page.getByRole("region", { name: "Свой прокси за 5 минут" })).toHaveCount(0);

  await page.goto("/mock.html?still&promo&when=proxy_down&down=all");
  await expect(page.getByRole("region", { name: "Свой прокси за 5 минут" })).toBeVisible({ timeout: 20_000 });

  // Порог в минутах: прокси только что отказал - ещё рано.
  await page.goto("/mock.html?still&promo&when=proxy_down&down=all&after=30");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(800);
  await expect(page.getByRole("region", { name: "Свой прокси за 5 минут" })).toHaveCount(0);
});

test("баннер «в мастере»: под полем прокси, не на «Обзоре», уходит при вводе", async ({ page }) => {
  await page.goto("/mock.html?still&promo&when=setup");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(800);
  await expect(page.getByRole("region", { name: "Свой прокси за 5 минут" })).toHaveCount(0);

  await page.goto("/mock.html?still&promo&when=setup&fresh&media");
  const banner = page.getByRole("region", { name: "Свой прокси за 5 минут" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  // В мастере без обложки: фото и видео по кнопке.
  await expect(banner.getByRole("button", { name: "Фото и видео" })).toHaveCount(0);
  await expect(banner.getByRole("button", { name: "Посмотреть" })).toBeVisible();
  await page.getByLabel("Строка прокси").fill("1.2.3.4:8000:u:p");
  await expect(banner).toHaveCount(0);
});
