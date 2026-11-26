import { expect, test } from "@playwright/test";

/*
  Обход VPN на стенде mock.html: ?vpn=happ|wg - какой VPN включён,
  ?bypass=ok|blocked - мост ходит мимо него или VPN не пускает.
*/

test("мост обошёл VPN - спокойная строка вместо предупреждения", async ({ page }) => {
  await page.goto("/mock.html?still&vpn=happ&bypass=ok&cause=silent");
  await expect(page.getByTestId("vpn-calm")).toContainText("Включён VPN Happ. Мост ходит к прокси мимо него", {
    timeout: 20_000,
  });
  await expect(page.getByTestId("vpn-notice")).toHaveCount(0);
});

test("VPN не пускает и прокси молчит - предупреждение открывает окно с инструкцией", async ({ page }) => {
  await page.goto("/mock.html?still&vpn=wg&bypass=blocked&cause=silent");
  const notice = page.getByTestId("vpn-notice");
  await expect(notice).toContainText("не пускает мост к прокси напрямую", { timeout: 20_000 });
  await notice.getByRole("button", { name: "Как добавить исключение" }).click();

  const dialog = page.getByRole("dialog", { name: "Исключения VPN" });
  await expect(dialog.getByText("VPN WireGuard не пускает Clod Helper к прокси напрямую")).toBeVisible();
  await expect(dialog.getByText("185.68.187.138")).toBeVisible();
  // Не знаем, что это Happ, - сначала спросим.
  const happ = dialog.getByTestId("vpn-happ");
  await expect(happ.getByRole("button", { name: "Добавить исключения в Happ" })).toHaveCount(0);
  await happ.getByRole("button", { name: "Да, Happ" }).click();
  await expect(happ.getByText("Кнопка сработает, только если VPN правда Happ")).toBeVisible();
  await happ.getByRole("button", { name: "Добавить исключения в Happ" }).click();
  await expect(happ.getByText("Happ открыт")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __happ: string[] }).__happ)).toContain("185.68.187.138");

  await dialog.getByRole("button", { name: "Понятно" }).click();
  await expect(dialog).toHaveCount(0);
});

test("клик по уведомлению: VPN - Happ, кнопка сразу, без вопроса", async ({ page }) => {
  await page.goto("/mock.html?still&vpn=happ&bypass=blocked");
  await page.getByRole("button", { name: /^Уведомления/ }).waitFor({ timeout: 20_000 });
  await page.evaluate(() => (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit("vpn:help", null));
  const happ = page.getByRole("dialog", { name: "Исключения VPN" }).getByTestId("vpn-happ");
  await expect(happ.getByText("Happ можно настроить одной кнопкой")).toBeVisible();
  await expect(happ.getByRole("button", { name: "Да, Happ" })).toHaveCount(0);
  await expect(happ.getByRole("button", { name: "Добавить исключения в Happ" })).toBeVisible();
});

test("уведомление про VPN берёт текст из адреса", async ({ page }) => {
  const t = encodeURIComponent("VPN Happ не пускает мимо себя");
  const s = encodeURIComponent("Нажмите, покажем, что делать");
  await page.goto(`/mock.html?view=toast&n=vpn-blocked&t=${t}&s=${s}`);
  await expect(page.getByText("VPN Happ не пускает мимо себя")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText("Нажмите, покажем, что делать")).toBeVisible();
});
