import { expect, test, type Page } from "@playwright/test";

/*
  Приватность и отчёты на стенде mock.html: три переключателя в мастере и
  настройках, «Сообщить о проблеме», окно для обновившихся со ссылкой
  «в настройках», отправка ошибок интерфейса.
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
  await expect(page.getByText("Раз в день: версия, система, работал ли прокси, как обновилась программа. Без адресов и настроек прокси")).toBeVisible();
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

test("предложение для обновившихся: строка про отчёты, ссылка ведёт в настройки без ответа", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 580 });
  // Без параметров стенда: ?whatsnew&autooffer показывает строку со ссылкой.
  await page.goto("/mock.html?still&whatsnew&autooffer&platform=win");
  await expect(page.getByText("Clod Helper обновлён")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Отлично" }).click();
  const dialog = page.getByRole("dialog", { name: "Автообновление. Включить?" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await expect(dialog.getByText(/отправляет отчёты об ошибках и анонимную статистику/)).toBeVisible();
  await expect(dialog.getByRole("switch")).toHaveCount(0);
  expect(await dialog.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(false);
  await dialog.getByRole("button", { name: "в настройках" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Обновления и отчёты" })).toBeVisible();
  // Ответ не записан: auto_update остаётся пустым.
  expect(await page.evaluate(() => (window as unknown as { __autoUpdate: () => boolean | null }).__autoUpdate())).toBeNull();
  // Не трогал - при следующем запуске предложение снова показывается.
  await page.goto("/mock.html?still&autooffer&platform=win");
  await expect(page.getByRole("dialog", { name: "Автообновление. Включить?" })).toBeVisible({ timeout: 20_000 });
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

test("отчёт: лимит 2000 символов и счётчик после 1600", async ({ page }) => {
  const { dialog } = await openReport(page);
  const field = dialog.getByLabel("Что случилось");
  await expect(field).toHaveAttribute("maxlength", "2000");
  await expect(dialog.getByText(/из 2000/)).toHaveCount(0);
  await field.fill("а".repeat(1650));
  await expect(dialog.getByText("1650 из 2000")).toBeVisible();
});

test("колокольчик: подпись кнопки link_label, событие clicked уходит один раз", async ({ page }) => {
  await page.goto("/mock.html?still&notice&platform=win");
  await page.getByRole("button", { name: /Уведомления/ }).click();
  const panel = page.getByRole("dialog", { name: "Уведомления" });
  const link = panel.getByRole("button", { name: "Что нового", exact: true });
  await expect(link).toBeVisible();
  await expect(panel.getByRole("button", { name: "Подробнее", exact: true })).toHaveCount(0);
  await link.click();
  const ev = await page.evaluate(() => (window as unknown as { __noticeEvents?: unknown[] }).__noticeEvents);
  expect(ev).toEqual([{ id: "whats-new-0-9-1", ev: "clicked" }]);
});

// ─── ничего не уходит до согласия (флаг privacy_notice_seen) ─────────────────

type N = { __noticeSeen: () => boolean; __privacyWrites: { key: string; on: boolean }[] };
const noticeSeen = (page: Page) => page.evaluate(() => (window as unknown as N).__noticeSeen());
const writes = (page: Page) => page.evaluate(() => (window as unknown as N).__privacyWrites);

test("мастер: первый шаг не пишет переключатели, «Готово» пишет их и ставит флаг", async ({ page }) => {
  await page.goto("/mock.html?still&fresh&platform=win&wizslow");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByRole("heading", { name: "Настраиваю всё сам" })).toBeVisible();
  expect(await writes(page)).toEqual([]);
  expect(await noticeSeen(page)).toBe(false);
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 15_000 });
  await expect.poll(() => noticeSeen(page)).toBe(true);
  expect((await writes(page)).map((w) => w.key).sort()).toEqual(["auto_update", "crash_reports", "usage_stats"]);
  // Выбор человека пишется при каждом щелчке.
  await page.getByRole("switch", { name: "Отправлять анонимную статистику" }).click();
  expect((await writes(page)).at(-1)).toEqual({ key: "usage_stats", on: false });
});

test("окно предложения для обновившихся ставит флаг", async ({ page }) => {
  await page.goto("/mock.html?still&autooffer&platform=win");
  expect(await noticeSeen(page)).toBe(false);
  await expect(page.getByRole("dialog", { name: "Автообновление. Включить?" })).toBeVisible({ timeout: 20_000 });
  await expect.poll(() => noticeSeen(page)).toBe(true);
  // Плашки при этом нет: строка про отчёты уже была в окне.
  await expect(page.getByRole("region", { name: "Отчёты и статистика" })).toHaveCount(0);
});

test("плашка: показ ставит флаг, «Понятно» закрывает, после перезагрузки её нет", async ({ page }) => {
  await page.goto("/mock.html?still&noticebanner&platform=win");
  const banner = page.getByRole("region", { name: "Отчёты и статистика" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  await expect(banner).toContainText("С этой версии программа отправляет отчёты об ошибках и анонимную статистику. Выключить можно в настройках");
  await expect.poll(() => noticeSeen(page)).toBe(true);
  await banner.getByRole("button", { name: "Понятно" }).click();
  await expect(banner).toHaveCount(0);
  await page.reload();
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("region", { name: "Отчёты и статистика" })).toHaveCount(0);
});

test("плашка: «Настройки» открывает настройки", async ({ page }) => {
  await page.goto("/mock.html?still&noticebanner&platform=win");
  const banner = page.getByRole("region", { name: "Отчёты и статистика" });
  await banner.getByRole("button", { name: "Настройки" }).click();
  await expect(banner).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Обновления и отчёты" })).toBeVisible();
});

test("плашки нет, если флаг уже стоит", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("region", { name: "Отчёты и статистика" })).toHaveCount(0);
});

test("настройки: «Что именно отправляется» раскрывается и сворачивается", async ({ page }) => {
  await openSettings(page);
  const toggle = page.getByRole("button", { name: "Что именно отправляется" });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByText("неделя установки (только в первый раз и раз в неделю)")).toHaveCount(0);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByText(/неделя установки \(только в первый раз и раз в неделю\)/)).toBeVisible();
  await expect(page.getByText(/какой это сбой \(программа, окно, мост, обновление\)/)).toBeVisible();
  await expect(page.getByText("показано, нажато, закрыто")).toBeVisible();
  await expect(page.getByText(/спрашивает сервер объявлений/)).toBeVisible();
  await toggle.click();
  await expect(page.getByText("показано, нажато, закрыто")).toHaveCount(0);
});

test("отчёт: «Посмотреть, что уйдёт» показывает очищенное тело", async ({ page }) => {
  const { dialog } = await openReport(page);
  await dialog.getByLabel("Что случилось").fill("Не открывается окно");
  await dialog.getByRole("button", { name: "Посмотреть, что уйдёт" }).click();
  const pre = dialog.getByLabel("Что уйдёт на сервер");
  await expect(pre).toBeVisible();
  await expect(pre).toContainText('"text": "Не открывается окно"');
  await expect(pre).toContainText('"nonce"');
  expect(await pre.evaluate((el) => getComputedStyle(el).fontFamily.toLowerCase())).toContain("mono");
  expect(await pre.evaluate((el) => getComputedStyle(el).overflowY)).toBe("auto");
  // Отправка по-прежнему доступна, предпросмотр можно скрыть.
  await dialog.getByRole("button", { name: "Скрыть" }).click();
  await expect(pre).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Отправить" })).toBeEnabled();
});
