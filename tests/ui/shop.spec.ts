import { expect, test, type Page } from "@playwright/test";

/*
  Купленный прокси по ссылке clodhelper://add?key=... на стенде mock.html.
  ?deeplink - программа открыта ссылкой (новый адрес), ?deeplink=update - тот
  же адрес уже в пуле, ?deeplink=fail - магазин не ответил. window.__shopLink()
  - ссылка пришла в уже запущенную программу. __poolWrites - что записано в пул.
*/

type Win = {
  __poolWrites?: string[][];
  __shopClaims: string[];
  __checked?: string[];
  __shopLink: () => void;
  __R: { proxy_labels_get: Record<string, string> };
};

function collectErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && !m.text().includes("flagcdn") && errors.push(m.text()));
  return errors;
}

const poolWrites = (page: Page) => page.evaluate(() => (window as unknown as Win).__poolWrites ?? []);
const dialog = (page: Page) => page.getByRole("dialog", { name: "Добавить купленный прокси?" });

test("ссылка: окно показывает страну, срок и адрес без пароля, «Не сейчас» ничего не меняет", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto("/mock.html?still&platform=win&deeplink");
  const d = dialog(page);
  await expect(d).toBeVisible({ timeout: 20_000 });
  await expect(d.getByText("Нидерланды")).toBeVisible();
  await expect(d.getByText("45.142.7.19:8000")).toBeVisible();
  await expect(d.getByText("Действует до 22 декабря 2026")).toBeVisible();
  await expect(d.getByText("Покупка на b***@mail.ru")).toBeVisible();
  // Пароль и логин нигде в окне не видны.
  await expect(d).not.toContainText("s3cret");
  await expect(d).not.toContainText("buyer");
  await expect(d).not.toContainText("—");
  await expect(page.getByRole("button", { name: "Добавить" })).toBeVisible();

  await d.getByRole("button", { name: "Не сейчас" }).click();
  await expect(d).toBeHidden();
  expect(await poolWrites(page)).toEqual([]);
  expect(await page.evaluate(() => (window as unknown as Win).__shopClaims.length)).toBe(1);
  expect(errors).toEqual([]);
});

test("ссылка: «Добавить» дописывает прокси в пул, ставит имя и сразу проверяет", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto("/mock.html?still&platform=win&deeplink");
  const d = dialog(page);
  await expect(d).toBeVisible({ timeout: 20_000 });
  await d.getByRole("button", { name: "Добавить" }).click();

  const done = page.getByRole("dialog", { name: "Прокси добавлен" });
  await expect(done).toBeVisible();
  await expect(done.getByText("Работает, 74 мс")).toBeVisible();
  expect(await poolWrites(page)).toEqual([
    ["http://u:p@185.68.187.138:8000", "socks5://u:p@93.1.2.3:1080", "http://buyer:s3cret@45.142.7.19:8000"],
  ]);
  const w = await page.evaluate(() => {
    const x = window as unknown as Win;
    return { labels: x.__R.proxy_labels_get, checked: x.__checked ?? [] };
  });
  expect(w.labels["http://buyer:s3cret@45.142.7.19:8000"]).toBe("Нидерланды");
  expect(w.checked).toContain("http://buyer:s3cret@45.142.7.19:8000");

  // «К прокси» - страница уже со свежим пулом.
  await done.getByRole("button", { name: "К прокси" }).click();
  await expect(done).toBeHidden();
  await expect(page.getByText("45.142.7.19").first()).toBeVisible();
  expect(errors).toEqual([]);
});

test("ссылка: тот же адрес уже в пуле - запись обновляется, а не дублируется", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win&deeplink=update");
  const d = dialog(page);
  await expect(d).toBeVisible({ timeout: 20_000 });
  await expect(d.getByText("уже есть, обновится")).toBeVisible();
  await d.getByRole("button", { name: "Добавить" }).click();
  await expect(page.getByRole("dialog", { name: "Прокси добавлен" })).toBeVisible();
  expect(await poolWrites(page)).toEqual([["http://buyer:s3cret@185.68.187.138:8000", "socks5://u:p@93.1.2.3:1080"]]);
  // Своё имя едет за записью и не перетирается именем из магазина.
  const labels = await page.evaluate(() => (window as unknown as Win).__R.proxy_labels_get);
  expect(labels["http://buyer:s3cret@185.68.187.138:8000"]).toBe("Claude main");
  expect(labels["http://u:p@185.68.187.138:8000"]).toBeUndefined();
});

test("ссылка в запущенную программу: окно появляется поверх открытой страницы", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await expect(dialog(page)).toBeHidden();
  await page.evaluate(() => (window as unknown as Win).__shopLink());
  await expect(dialog(page)).toBeVisible();
  await expect(dialog(page).getByText("45.142.7.19:8000")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog(page)).toBeHidden();
  expect(await poolWrites(page)).toEqual([]);
});

test("ссылка: магазин не ответил - ошибка и «Повторить»", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win&deeplink=fail");
  const d = dialog(page);
  await expect(d).toBeVisible({ timeout: 20_000 });
  await expect(d.getByRole("alert")).toHaveText("Нет связи с магазином. Проверьте интернет и попробуйте ещё раз");
  await d.getByRole("button", { name: "Повторить" }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as Win).__shopClaims.length)).toBe(2);
  await d.getByRole("button", { name: "Закрыть" }).click();
  await expect(d).toBeHidden();
  expect(await poolWrites(page)).toEqual([]);
});

test("ссылка в мастере: прокси подставляется в поле, пул не трогается", async ({ page }) => {
  await page.goto("/mock.html?still&fresh&platform=win&deeplink");
  const d = dialog(page);
  await expect(d).toBeVisible({ timeout: 20_000 });
  await d.getByRole("button", { name: "Добавить" }).click();
  await expect(d).toBeHidden();
  await expect(page.getByRole("heading", { name: "Вставьте прокси" })).toBeVisible();
  await expect(page.getByLabel("Строка прокси")).toHaveValue("http://buyer:s3cret@45.142.7.19:8000");
  await expect(page.getByRole("button", { name: "Дальше" })).toBeEnabled();
  expect(await poolWrites(page)).toEqual([]);
});
