import { expect, test, type Page } from "@playwright/test";

/*
  «Обзор»: предупреждение о прокси, который человек сознательно не оплатил,
  можно скрыть. Строка уходит с «Обзора» и из статуса в боковой панели,
  на «Прокси» видна пометка «Предупреждение скрыто» с кнопкой «Вернуть».
  Имя прокси - главным, адрес - второй строкой.
*/

type Win = { __R: { proxy_alerts_muted_get: string[] } };

async function open(page: Page, query: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto(`/mock.html?still&platform=win&${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Развернуть панель" }).click();
  return errors;
}

const OLD = "http://u:p@193.8.114.42:8080";
const issue = (page: Page) => page.getByTestId("proxy-issue");

test("имя главным, адрес второй строкой; без имени главным становится адрес", async ({ page }) => {
  const errors = await open(page, "down=1&oldname");
  await expect(issue(page).getByTestId("proxy-issue-title")).toHaveText("CD - old2");
  await expect(issue(page).getByTestId("proxy-issue-addr")).toHaveText("193.8.114.42");
  await page.screenshot({ path: "test-results/mute-name-addr.png" });

  await page.goto("/mock.html?still&platform=win&down=1");
  await expect(issue(page).getByTestId("proxy-issue-title")).toHaveText("193.8.114.42");
  await expect(issue(page).getByTestId("proxy-issue-addr")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("«Скрыть»: строки нет на «Обзоре», статус не считает поломкой, на «Прокси» - «Вернуть»", async ({ page }) => {
  const errors = await open(page, "down=1&oldname");
  const sidebar = page.locator("aside");
  await expect(sidebar).toContainText("1 прокси не отвечает");

  await issue(page).getByRole("button").first().click();
  await page.getByTestId("proxy-issue-mute").click();
  await expect(issue(page)).toHaveCount(0);
  await expect(page.getByTestId("proxy-muted-note")).toContainText("CD - old2");
  await expect(sidebar).not.toContainText("не отвечает");
  expect(await page.evaluate(() => (window as unknown as Win).__R.proxy_alerts_muted_get)).toEqual([OLD]);
  await page.screenshot({ path: "test-results/mute-hidden.png" });

  // «Прокси»: прокси на месте и так же «не отвечает», но с пометкой.
  await page.getByRole("tab", { name: "Прокси" }).click();
  const mark = page.getByTestId("proxy-muted");
  await expect(mark).toHaveCount(1);
  await expect(page.getByText("CD - old2")).toBeVisible();
  await mark.getByRole("button", { name: "Вернуть" }).click();
  await expect(mark).toHaveCount(0);
  await expect(sidebar).toContainText("1 прокси не отвечает");
  expect(errors).toEqual([]);
});

test("скрытое в настройках не показывается после запуска", async ({ page }) => {
  const errors = await open(page, "down=1&oldname&muted");
  await expect(issue(page)).toHaveCount(0);
  await expect(page.locator("aside")).not.toContainText("не отвечает");
  expect(errors).toEqual([]);
});
