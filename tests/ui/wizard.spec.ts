import { expect, test, type Page } from "@playwright/test";

/*
  Мастер первого запуска и предложение автообновления на стенде mock.html.
  ?fresh — моста нет, открывается мастер; ?wizfail — проверка соединения
  падает; ?whatsnew — окно «Что нового»; ?autooffer — установка до функции
  автообновления (ещё не спрашивали). window.__autoUpdate() — что записано.
*/

function collectErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  return errors;
}

async function openWizard(page: Page, query = "") {
  await page.goto(`/mock.html?still&fresh&platform=win${query}`);
  await expect(page.getByRole("heading", { name: "Вставьте прокси" })).toBeVisible({ timeout: 20_000 });
}

const privacy = (page: Page) => page.evaluate(() => (window as unknown as { __privacy: () => Record<string, boolean> }).__privacy());
const autoUpdate = (page: Page) => page.evaluate(() => (window as unknown as { __autoUpdate: () => boolean | null }).__autoUpdate());

test("мастер: прокси → настройка → готово, три переключателя включены по умолчанию", async ({ page }) => {
  const errors = collectErrors(page);
  await openWizard(page);
  await expect(page.getByText("Шаг 1 из 3")).toBeVisible();
  const next = page.getByRole("button", { name: "Дальше" });
  await expect(next).toBeDisabled();

  const input = page.getByLabel("Строка прокси");
  await expect(input).toBeFocused();
  await input.fill("185.68.187.138:8000:user:pass");
  await expect(page.getByText("Finland")).toBeVisible();
  await expect(next).toBeEnabled();
  // Enter в поле — то же, что «Дальше».
  await input.press("Enter");

  await expect(page.getByRole("heading", { name: "Настраиваю всё сам" })).toBeVisible();
  await expect(page.getByText("Шаг 2 из 3")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Шаг 3 из 3")).toBeVisible();
  await expect(page.getByText("185.68.187.138")).toBeVisible();

  // Главная кнопка в фокусе — Enter открывает программу.
  const open = page.getByRole("button", { name: "Открыть Clod Helper" });
  await expect(open).toBeFocused();

  // Автообновление: включено и уже записано при старте установки.
  const sw = page.getByRole("switch", { name: "Обновлять автоматически" });
  await expect(sw).toHaveAttribute("aria-checked", "true");
  expect(await autoUpdate(page)).toBe(true);
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "false");
  expect(await autoUpdate(page)).toBe(false);
  // Отчёты и статистика: включены и записаны, снятие тоже пишется.
  const crash = page.getByRole("switch", { name: "Отправлять отчёты об ошибках" });
  const stats = page.getByRole("switch", { name: "Отправлять анонимную статистику" });
  await expect(crash).toHaveAttribute("aria-checked", "true");
  await expect(stats).toHaveAttribute("aria-checked", "true");
  await stats.click();
  await expect(stats).toHaveAttribute("aria-checked", "false");
  expect(await privacy(page)).toEqual({ auto_update: false, crash_reports: true, usage_stats: false });

  await open.click();
  await expect(page.getByRole("heading", { name: "Вставьте прокси" })).toHaveCount(0);
  // Новичку предложение автообновления не показываем — он уже ответил в мастере.
  await expect(page.getByText("Автообновление. Включить?")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("мастер: проверка не прошла — причина и «Повторить», назад к прокси", async ({ page }) => {
  const errors = collectErrors(page);
  await openWizard(page, "&wizfail");
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByRole("heading", { name: "Настройка не закончилась" })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole("alert")).toContainText("Прокси не ответил");
  await expect(page.getByRole("button", { name: "Повторить" })).toBeVisible();
  await page.getByRole("button", { name: "Другой прокси" }).click();
  await expect(page.getByRole("heading", { name: "Вставьте прокси" })).toBeVisible();
  // Введённая строка не пропала.
  await expect(page.getByLabel("Строка прокси")).toHaveValue(/185\.68\.187\.138/);
  expect(errors).toEqual([]);
});

test("мастер на маленьком окне: кнопка шага всегда видна", async ({ page }) => {
  await page.setViewportSize({ width: 600, height: 520 });
  await openWizard(page);
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  const next = page.getByRole("button", { name: "Дальше" });
  await expect(next).toBeInViewport();
  await next.click();
  const open = page.getByRole("button", { name: "Открыть Clod Helper" });
  await expect(open).toBeVisible({ timeout: 10_000 });
  await expect(open).toBeInViewport();
  // Ничего не вылезает вбок.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(overflow).toBe(false);
});

test("мастер без анимаций (prefers-reduced-motion) проходится целиком", async ({ page }) => {
  const errors = collectErrors(page);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/mock.html?fresh&platform=win");
  await expect(page.getByRole("heading", { name: "Вставьте прокси" })).toBeVisible({ timeout: 20_000 });
  await page.getByLabel("Строка прокси").fill("185.68.187.138:8000:user:pass");
  await page.getByRole("button", { name: "Дальше" }).click();
  await expect(page.getByRole("heading", { name: "Всё работает" })).toBeVisible({ timeout: 10_000 });
  expect(errors).toEqual([]);
});

test("после «Что нового» — предложение автообновления; «Не сейчас» записывает отказ", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto("/mock.html?still&whatsnew&autooffer&platform=win");
  await expect(page.getByText("Clod Helper обновлён")).toBeVisible({ timeout: 20_000 });
  // Пока открыто «Что нового», предложения нет — оно идёт следом.
  await expect(page.getByText("Автообновление. Включить?")).toHaveCount(0);
  await page.getByRole("button", { name: "Отлично" }).click();

  const dialog = page.getByRole("dialog", { name: "Автообновление. Включить?" });
  await expect(dialog).toBeVisible();
  const yes = dialog.getByRole("button", { name: "Включить автообновление" });
  await expect(yes).toBeFocused();
  // Крупная кнопка согласия заметно больше отказа.
  const yesBox = (await yes.boundingBox())!;
  const noBox = (await dialog.getByRole("button", { name: "Не сейчас" }).boundingBox())!;
  expect(yesBox.height).toBeGreaterThanOrEqual(44);
  expect(yesBox.width * yesBox.height).toBeGreaterThan(noBox.width * noBox.height * 4);

  expect(await autoUpdate(page)).toBe(null);
  await dialog.getByRole("button", { name: "Не сейчас" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await autoUpdate(page)).toBe(false);
  expect(errors).toEqual([]);
});

test("предложение автообновления: «Включить» записывает согласие, Esc — отказ", async ({ page }) => {
  await page.goto("/mock.html?still&autooffer&platform=win");
  const dialog = page.getByRole("dialog", { name: "Автообновление. Включить?" });
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await dialog.getByRole("button", { name: "Включить автообновление" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await autoUpdate(page)).toBe(true);

  await page.goto("/mock.html?still&autooffer&platform=win");
  await expect(dialog).toBeVisible({ timeout: 20_000 });
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(await autoUpdate(page)).toBe(false);
});

test("предложение не показывается тем, кто уже ответил", async ({ page }) => {
  await page.goto("/mock.html?still&whatsnew&platform=win");
  await expect(page.getByText("Clod Helper обновлён")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Отлично" }).click();
  await expect(page.getByText("Мост активен").first()).toBeVisible();
  await expect(page.getByText("Автообновление. Включить?")).toHaveCount(0);
});

test("настройки: переключатель автообновления", async ({ page }) => {
  await page.goto("/mock.html?still&platform=win");
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("tab", { name: "Настройки" }).click();
  const sw = page.getByRole("switch", { name: "Обновлять автоматически" });
  await expect(sw).toHaveAttribute("aria-checked", "true");
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "false");
  expect(await autoUpdate(page)).toBe(false);
});
