import { expect, test, type Page } from "@playwright/test";

/*
  «Обзор»: почему прокси не работает и что сделать (одна фраза, одна кнопка),
  и подсказка про VPN рядом - без ложных тревог.
*/

async function open(page: Page, query: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 700, height: 640 });
  await page.goto(`/mock.html?still&${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.evaluate(() => document.fonts.ready);
  return errors;
}

const issue = (page: Page) => page.getByTestId("proxy-issue").first();
const expand = (page: Page) => issue(page).getByRole("button").first().click();

test("пароль не принят: строка короткая, по клику - причина и «Заменить пароль»", async ({ page }) => {
  const errors = await open(page, "cause=auth&vpn=wg");
  await expect(issue(page)).toContainText("не отвечает");
  // Подробности - только по раскрытию.
  await expect(page.getByTestId("proxy-cause")).toHaveCount(0);
  // VPN не виноват в неверном пароле - подсказки про VPN нет.
  await expect(page.getByTestId("vpn-notice")).toHaveCount(0);
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("не принял логин или пароль");
  await page.screenshot({ path: "test-results/cause-auth.png" });
  await page.getByRole("button", { name: "Заменить пароль" }).click();
  // «Прокси»: сразу открыта замена, в поле - старая строка.
  const card = page.getByTestId("proxy-replace");
  await expect(card).toBeVisible();
  await expect(card).toContainText("93.1.2.3");
  await card.getByRole("textbox").fill("socks5://u:newpass@93.1.2.3:1080");
  await card.getByRole("button", { name: "Сохранить" }).click();
  await expect(card).toHaveCount(0);
  const replaced = await page.evaluate(() => (window as unknown as { __replaced: unknown[] }).__replaced);
  expect(replaced).toEqual([{ old: "socks5://u:p@93.1.2.3:1080", new: "socks5://u:newpass@93.1.2.3:1080" }]);
  expect(errors).toEqual([]);
});

test("прокси молчит при включённом VPN: подсказка про VPN и «Добавить запасной прокси»", async ({ page }) => {
  const errors = await open(page, "cause=silent&vpn=wg");
  const vpn = page.getByTestId("vpn-notice");
  await expect(vpn).toContainText("Включён VPN: WireGuard");
  await expect(vpn).toContainText("исключения VPN");
  await expect(vpn.getByRole("button", { name: "Скопировать адреса" })).toBeVisible();
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("молчит");
  await page.screenshot({ path: "test-results/cause-silent-vpn.png" });
  await page.getByRole("button", { name: "Добавить запасной прокси" }).click();
  await expect(page.getByText("Новый прокси")).toBeVisible();
  expect(errors).toEqual([]);
});

test("сервер выключен - «Проверить снова»; мост не может - «Перезапустить мост»", async ({ page }) => {
  let errors = await open(page, "cause=closed");
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("Сервер прокси не отвечает");
  await page.getByRole("button", { name: "Проверить снова" }).click();
  await expect(page.getByRole("button", { name: "Проверить снова" })).toBeEnabled();
  expect(errors).toEqual([]);

  errors = await open(page, "cause=5xx");
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("не смог открыть сайт");
  await expect(page.getByRole("button", { name: "Проверить снова" })).toBeVisible();

  errors = await open(page, "cause=bridge");
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("Мост не может подключиться");
  await issue(page).getByRole("button", { name: "Перезапустить мост" }).click();
  await expect(issue(page).getByRole("button", { name: "Перезапустить мост" })).toBeEnabled();
  await page.screenshot({ path: "test-results/cause-bridge.png" });
  expect(errors).toEqual([]);
});

test("прокси в России, а через него Claude: отдельная строка и совет", async ({ page }) => {
  const errors = await open(page, "ru");
  await expect(issue(page)).toContainText("в России");
  await expect(issue(page)).toContainText("Claude через него не откроется");
  await expand(page);
  await expect(page.getByTestId("proxy-cause")).toContainText("Нужен прокси в другой стране");
  await expect(page.getByRole("button", { name: "Добавить запасной прокси" })).toBeVisible();
  // Прокси работает - VPN тут ни при чём.
  await expect(page.getByTestId("vpn-notice")).toHaveCount(0);
  await page.screenshot({ path: "test-results/cause-ru.png" });
  expect(errors).toEqual([]);
});

test("VPN включён, но всё работает - никакой тревоги", async ({ page }) => {
  const errors = await open(page, "vpn=wg");
  await expect(page.getByText("185.68.187.138")).toBeVisible();
  await expect(page.getByTestId("vpn-notice")).toHaveCount(0);
  await expect(page.getByTestId("proxy-issue")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("безымянный туннель при неработающем прокси - просто «Включён VPN»", async ({ page }) => {
  const errors = await open(page, "vpn=tun&down=1");
  const vpn = page.getByTestId("vpn-notice");
  await expect(vpn).toContainText("Включён VPN");
  await expect(vpn).not.toContainText("Включён VPN:");
  expect(errors).toEqual([]);
});

test("SOTA VPN стёр настройки прокси - «Вернуть настройки»", async ({ page }) => {
  const errors = await open(page, "vpn=sota");
  const vpn = page.getByTestId("vpn-notice");
  await expect(vpn).toContainText("Пропали настройки прокси");
  await expect(vpn).toContainText("SOTA VPN");
  await page.screenshot({ path: "test-results/vpn-sota.png" });
  await vpn.getByRole("button", { name: "Вернуть настройки" }).click();
  await expect(vpn).toHaveCount(0);
  expect(errors).toEqual([]);
});
