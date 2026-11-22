import { expect, test, type Page } from "@playwright/test";

/** Открыть стенд, собрать ошибки консоли. */
async function open(page: Page, query = "still") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto(`/mock.html?${query}`);
  // Первый заход ждёт компиляцию vite - при многих параллельных воркерах
  // дольше стандартных 5 с.
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  // Размеры (меню, попапы) - только после загрузки шрифтов: под нагрузкой
  // меню успевало измериться запасным шрифтом и потом вырастало.
  await page.evaluate(() => document.fonts.ready);
  return errors;
}

/** Открыть окно настроек приложения через шестерёнку на иконке. */
async function gear(page: Page, name: string) {
  await page.getByText(name, { exact: true }).first().hover();
  await page.getByRole("button", { name: `Настройки - ${name}` }).click();
}

async function go(page: Page, label: string) {
  await page.getByRole("tab", { name: label }).click();
}

test("обзор: IP один раз, проверка приватности с вердиктом", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByText("185.68.187.138")).toHaveCount(1);
  await expect(page.getByText("Доступна версия 0.7.0")).toBeVisible();
  await page.getByRole("button", { name: "Проверить" }).click();
  await expect(page.getByText("Всё чисто")).toBeVisible();
  // Рядом с IP - пинг прокси, а не время проверочного запроса (420 мс).
  await expect(page.getByText("74 мс")).toBeVisible();
  await expect(page.getByText("420 мс")).toHaveCount(0);
  await expect(page.getByText("Через мост ваш IP скрыт")).toBeVisible();
  await expect(page.getByText("DNS не выдаёт провайдера")).toBeVisible();
  await expect(page.getByText("Приложения из вашего списка - через мост")).toBeVisible();
  await expect(page.getByText("Остальные программы работают напрямую, как обычно")).toBeVisible();
  await page.screenshot({ path: "test-results/overview.png" });
  expect(errors).toEqual([]);
});

test("приложения: папки, статусы, перехват расширением", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  for (const folder of ["AI", "Браузеры", "Мессенджеры", "Разработка"]) {
    await expect(page.getByText(folder, { exact: true })).toBeVisible();
  }
  await expect(page.getByText("Прокси перехватывает FoxyProxy")).toBeVisible();
  await expect(page.getByRole("button", { name: "Отключить", exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-list.png", fullPage: true });
  expect(errors).toEqual([]);
});

test("приложения: сетка и режим упорядочивания", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByRole("radio", { name: "Сетка" }).click();
  await expect(page.getByText("Telegram")).toBeVisible();
  await page.screenshot({ path: "test-results/apps-grid.png", fullPage: true });
  // ПКМ → меню → «Выбрать»
  await page.getByText("Telegram").click({ button: "right" });
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  // Меню целиком в окне (раньше обрезалось снизу).
  const box = await menu.boundingBox();
  expect(box && box.y + box.height).toBeLessThanOrEqual(580);
  await expect(page.getByRole("menuitem", { name: "Убрать из списка" })).toBeInViewport();
  await page.screenshot({ path: "test-results/apps-context-menu.png" });
  await page.getByRole("menuitem", { name: "Выбрать" }).click();
  await expect(page.getByRole("button", { name: "Готово" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Новая папка" })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-editing.png", fullPage: true });
  await page.getByRole("button", { name: "Готово" }).click();
  await expect(page.getByRole("button", { name: "Добавить" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("приложения: окно настроек через шестерёнку", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await gear(page, "Google Chrome");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Прокси", { exact: true })).toBeVisible();
  await expect(dialog.getByText("Всегда через мост")).toBeVisible();
  await expect(dialog.getByText("Весь трафик через прокси")).toBeVisible();
  await page.screenshot({ path: "test-results/app-settings.png" });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  expect(errors).toEqual([]);
});

test("окно настроек приложения в границах окна: шапка на месте, листается тело", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  const box = (await dialog.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(36); // не залезает под заголовок окна
  expect(box.y + box.height).toBeLessThanOrEqual(580);
  const title = dialog.getByText("Claude Desktop", { exact: true });
  const before = (await title.boundingBox())!;
  await dialog.locator(".overflow-y-auto").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await expect(dialog.getByText("Убрать из списка")).toBeInViewport();
  expect((await title.boundingBox())!.y).toBe(before.y);
  await page.screenshot({ path: "test-results/app-settings-scrolled.png" });
  expect(errors).toEqual([]);
});

test("приложения: умное добавление", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Название программы").fill("wec");
  await expect(page.getByText("WeChat")).toBeVisible();
  await page.screenshot({ path: "test-results/apps-add.png" });
  expect(errors).toEqual([]);
});

test("прокси, настройки и сайдбар", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Прокси");
  await page.waitForTimeout(800);
  await page.screenshot({ path: "test-results/proxies.png", fullPage: true });
  await expect(page.getByText("Claude main").first()).toBeVisible();
  // IP - без порта, пинг - своей колонкой, без лишних подсказок.
  await expect(page.getByText("185.68.187.138", { exact: true })).toBeVisible();
  await expect(page.getByText("185.68.187.138:8000")).toHaveCount(0);
  await expect(page.getByText("74 мс", { exact: true })).toBeVisible();
  await expect(page.getByText(/Изменение пула перезапускает мост/)).toHaveCount(0);
  await page.getByRole("button", { name: "Добавить" }).click();
  await expect(page.getByPlaceholder("Прокси в любом формате или ключ из магазина", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Отмена" }).click();
  await go(page, "Настройки");
  await expect(page.getByText("Открывать при входе")).toBeVisible();
  await page.screenshot({ path: "test-results/settings.png" });
  await page.getByRole("button", { name: "Развернуть панель" }).click();
  await expect(page.getByText("AI-прокси мост")).toBeVisible();
  await page.screenshot({ path: "test-results/sidebar-open.png" });
  expect(errors).toEqual([]);
});

test("прокси: замер скорости по кнопке", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Прокси");
  await page.getByText("Claude main").first().hover();
  await page.getByRole("button", { name: "Замерить скорость", exact: true }).first().click();
  await expect(page.getByText("замеряю…")).toBeVisible();
  await expect(page.getByText("46 Мбит/с", { exact: true })).toBeVisible({ timeout: 5000 });
  // Результат запоминается: после перехода на другой экран - на месте.
  await go(page, "Настройки");
  await go(page, "Прокси");
  await expect(page.getByText("46 Мбит/с", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("прокси: основной отдельно, звёздочка «сделать основным», замер без кнопок", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Прокси");
  await expect(page.getByText("Основной", { exact: true })).toBeVisible();
  await expect(page.getByText("Другие прокси", { exact: true })).toBeVisible();
  await expect(page.getByText(/Запасные/)).toHaveCount(0);
  await page.getByText("93.1.2.3").first().hover();
  await expect(page.getByRole("button", { name: "Сделать основным", exact: true }).first()).toBeVisible();
  // Замер: кнопки уходят сразу, хотя курсор ещё на строке.
  await page.getByRole("button", { name: "Замерить скорость", exact: true }).last().click();
  await expect(page.getByText("замеряю…")).toBeVisible();
  // Кнопки прозрачны (слот занят цифрами), Playwright прозрачное считает видимым - смотрим opacity.
  await expect
    .poll(() => page.getByRole("button", { name: "Сделать основным", exact: true }).first().evaluate((b) => getComputedStyle(b.parentElement!).opacity))
    .toBe("0");
  await expect(page.getByText("7,4 Мбит/с", { exact: true })).toBeVisible({ timeout: 5000 });
  expect(errors).toEqual([]);
});

test("сайты: сброс автонастройки другой программой виден сам, без переходов", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Сайты");
  await expect(page.getByText(/сбросила «Во всех браузерах»/)).toHaveCount(0);
  // Другая программа сбросила настройку, пока экран открыт.
  await page.evaluate(() => {
    (window as unknown as { __R: Record<string, unknown> }).__R.system_pac_status = {
      supported: true,
      enabled: true,
      active: false,
      foreign: "прокси 127.0.0.1:2080",
    };
  });
  // Без перехода между экранами: StatusProvider проверяет сам (~15 с).
  await expect(page.getByText(/сбросила «Во всех браузерах»/)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("tab", { name: "Сайты" })).toHaveAttribute("title", /настройку сбросила другая программа/);
  expect(errors).toEqual([]);
});

test("настройки приложения: прокси сохраняется только по кнопке, затем - перезапуск", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await gear(page, "Google Chrome");
  const d = page.getByRole("dialog");
  await d.getByRole("radio").filter({ hasText: "Прокси 2" }).click();
  await expect(d.getByRole("button", { name: "Сохранить" })).toBeVisible();
  // Отмена - выбор возвращается, ничего не сохранено.
  await d.getByRole("button", { name: "Отменить" }).click();
  await expect(d.getByRole("button", { name: "Сохранить" })).toHaveCount(0);
  await expect(d.getByRole("radio").filter({ hasText: "Авто" })).toHaveAttribute("aria-checked", "true");
  // Сохранить - приложение открыто, предлагаем перезапуск.
  await d.getByRole("radio").filter({ hasText: "Прокси 2" }).click();
  await d.getByRole("button", { name: "Сохранить" }).click();
  await expect(d.getByRole("button", { name: "Перезапустить с новым прокси" })).toBeVisible();
  await d.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.getByText("Прокси сменён - перезапустите")).toBeVisible();
  await page.getByRole("button", { name: "Перезапустить", exact: true }).click();
  await expect(page.getByText("Прокси сменён - перезапустите")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("windows: статусы по соединениям, Telegram и вид плитками", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await expect(page.getByText("Telegram").first()).toBeVisible();
  await expect(page.getByText("Через мост · 4").first()).toBeVisible();
  await expect(page.getByText("Прокси перехватывает FoxyProxy").first()).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Вид" })).toBeVisible();
  await page.screenshot({ path: "test-results/apps-windows.png" });
  expect(errors).toEqual([]);
});

test("обзор без тумблеров; режим Claude Code в терминале - в окне Claude", async ({ page }) => {
  const errors = await open(page);
  await expect(page.getByText("Умная маршрутизация")).toHaveCount(0);
  await go(page, "Приложения");
  await expect(page.getByText("Терминал", { exact: true })).toHaveCount(0);
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Claude Code в терминале и VS Code")).toBeVisible();
  await dialog.getByText("Через прокси всё").click();
  await expect(dialog.getByText("Сохранено - действует в новых окнах терминала")).toBeVisible();
  await page.screenshot({ path: "test-results/claude-code-modes.png" });
  await dialog.getByText("Выключено", { exact: true }).click();
  await expect(dialog.getByText("Выключено - новые окна терминала пойдут напрямую")).toBeVisible();
  expect(errors).toEqual([]);
});

test("защита от утечек - в окне Claude и Telegram, не в Настройках", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Настройки");
  await expect(page.getByText("Открывать при входе")).toBeVisible();
  await expect(page.getByText("Защита от утечек")).toHaveCount(0);
  await go(page, "Приложения");
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("switch", { name: "Не пускать мимо прокси" })).toBeChecked();
  await page.screenshot({ path: "test-results/claude-guard.png" });
  await page.keyboard.press("Escape");
  await gear(page, "Telegram");
  const tg = dialog.getByRole("switch", { name: "Не пускать мимо прокси" });
  await expect(tg).not.toBeChecked();
  await tg.click();
  await expect(tg).toBeChecked();
  expect(errors).toEqual([]);
});

test("меню в строке меню: только статус и действия", async ({ page }) => {
  const errors = await open(page, "still&view=tray");
  await expect(page.getByRole("button", { name: "Тест соединения" })).toBeVisible();
  await expect(page.getByText("Терминал")).toHaveCount(0);
  await page.screenshot({ path: "test-results/tray.png" });
  expect(errors).toEqual([]);
});

test("windows: защита от утечек - в окне Claude и Telegram, как на Mac", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await gear(page, "Claude Desktop");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Режим", { exact: true })).toHaveCount(0);
  await expect(dialog.getByRole("switch", { name: "Не пускать мимо прокси" })).toBeChecked();
  await page.screenshot({ path: "test-results/windows-claude-guard.png" });
  await page.keyboard.press("Escape");
  await gear(page, "Telegram");
  const tg = dialog.getByRole("switch", { name: "Не пускать мимо прокси" });
  await expect(tg).not.toBeChecked();
  await tg.click();
  await expect(tg).toBeChecked();
  expect(errors).toEqual([]);
});

test("windows: Codex - защита WebRTC; устаревшее правило обновляется кнопкой", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await gear(page, "ChatGPT Codex");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Брандмауэр закрывает WebRTC", { exact: false })).toBeVisible();
  const guard = dialog.getByRole("switch", { name: "Не пускать мимо прокси" });
  await expect(guard).not.toBeChecked();
  await guard.click();
  await expect(guard).toBeChecked();
  await page.keyboard.press("Escape");
  // Codex обновился из Store: правило стоит на прежний exe.
  await page.evaluate(() => {
    (window as unknown as { __R: Record<string, unknown> }).__R.leak_stale = "codex";
  });
  await gear(page, "ChatGPT Codex");
  await expect(dialog.getByText("Приложение обновилось", { exact: false })).toBeVisible();
  await page.screenshot({ path: "test-results/windows-codex-guard-stale.png" });
  await page.keyboard.press("Escape");
  // У Claude правило свежее - предупреждения нет.
  await gear(page, "Claude Desktop");
  await expect(dialog.getByText("Брандмауэр закрывает прямой выход к Claude и WebRTC", { exact: false })).toBeVisible();
  await expect(dialog.getByText("Приложение обновилось", { exact: false })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("mac: у Codex нет защиты брандмауэра - pf не различает программы", async ({ page }) => {
  const errors = await open(page);
  await go(page, "Приложения");
  await gear(page, "ChatGPT Codex");
  await expect(page.getByRole("dialog").getByRole("switch", { name: "Не пускать мимо прокси" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("windows: Claude - обычная строка с «Подключить», без отдельной панели", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __mockOverrides: unknown }).__mockOverrides = {
      claude_proxy_status: {
        policy: { configured: false, managed: false, conflict: null, proxy_url: null, pac_configured: false, notice: null },
        bridge_ready: true,
        bridge_error: null,
        network_enforced: false,
      },
    };
  });
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await expect(page.getByText("Прокси Claude не настроен")).toBeVisible();
  // Прежней панели «Claude Desktop и агенты» с «Настроить Claude» больше нет.
  await expect(page.getByText("Claude Desktop и агенты")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Настроить Claude" })).toHaveCount(0);
  // Тумблеров killswitch в строках тоже нет - как на Mac.
  await expect(page.getByRole("switch", { name: /Проксировать/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Подключить", exact: true }).click();
  await expect(page.getByText(/закройте его через значок в трее/)).toBeVisible();
  await expect(page.getByText(/Cmd\+Q/)).toHaveCount(0);
  await page.screenshot({ path: "test-results/windows-claude-connected.png" });
  // Защита от утечек включилась вместе с политикой.
  await gear(page, "Claude Desktop");
  await expect(page.getByRole("dialog").getByRole("switch", { name: "Не пускать мимо прокси" })).toBeChecked();
  expect(errors).toEqual([]);
});

test("windows: обновление в один клик и «Всегда через мост» из меню «Пуск»", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await expect(page.getByText("Доступна версия 0.7.0")).toBeVisible();
  await expect(page.getByRole("button", { name: "Обновить", exact: true })).toBeVisible();
  await expect(page.getByText("Скачать", { exact: true })).toHaveCount(0);
  await go(page, "Приложения");
  await gear(page, "Google Chrome");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Всегда через мост")).toBeVisible();
  await expect(dialog.getByText(/Даже из меню «Пуск»/)).toBeVisible();
  await expect(dialog.getByText(/Пароль Mac/)).toHaveCount(0);
  await page.screenshot({ path: "test-results/windows-browser-policy.png" });
  expect(errors).toEqual([]);
});

test("windows: умное добавление - программы из меню «Пуск»", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Название программы").fill("wec");
  await expect(page.getByText("WeChat")).toBeVisible();
  // Папка программы - по Windows-пути, без имени exe.
  await expect(page.getByText("C:\\Program Files\\Tencent\\Weixin", { exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/windows-apps-add.png" });
  expect(errors).toEqual([]);
});

test("windows: Telegram через чужой локальный прокси - честный статус и «Подключить»", async ({ page }) => {
  const errors = await open(page, "still&platform=win&tg=other");
  await go(page, "Приложения");
  // Раньше: «Открыт · сейчас без соединений».
  await expect(page.getByText("Через другой прокси, мимо моста")).toBeVisible();
  await page.getByRole("button", { name: "Подключить", exact: true }).click();
  // Пока Telegram не подключился через мост - подсказка в строке.
  await expect(page.getByText("В окне Telegram нажмите «Подключить»")).toBeVisible();
  await expect(page.getByText("Готово: Telegram идёт через мост, мимо прокси не пойдёт")).toBeVisible();
  expect(errors).toEqual([]);
});

test("Telegram без связи из-за защиты - видно в строке, «Убрать из списка» снимает защиту", async ({ page }) => {
  const errors = await open(page, "still&tg=blocked");
  await go(page, "Приложения");
  // Раньше: спокойное «Открыт · сейчас без соединений».
  await expect(page.getByText("Без связи: в Telegram не включён прокси")).toBeVisible();
  await expect(page.getByRole("button", { name: "Подключить", exact: true })).toBeVisible();
  const row = page.getByText("Telegram", { exact: true });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Убрать из списка" }).click();
  await expect(row).toHaveCount(0);
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Название программы").fill("teleg");
  await page.getByRole("option").filter({ hasText: "Telegram" }).getByRole("button").click();
  await expect(row).toBeVisible();
  await expect(page.getByText("Без связи: в Telegram не включён прокси")).toHaveCount(0);
  await gear(page, "Telegram");
  await expect(page.getByRole("dialog").getByRole("switch", { name: "Не пускать мимо прокси" })).not.toBeChecked();
  expect(errors).toEqual([]);
});

test("windows: убранный Telegram находится поиском и возвращается в список", async ({ page }) => {
  const errors = await open(page, "still&platform=win");
  await go(page, "Приложения");
  const row = page.getByText("Telegram", { exact: true });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Убрать из списка" }).click();
  await expect(row).toHaveCount(0);
  // Раньше окно поиска отбрасывало его: путь совпадал с убранной строкой.
  await page.getByRole("button", { name: "Добавить" }).click();
  await page.getByPlaceholder("Название программы").fill("teleg");
  const option = page.getByRole("option").filter({ hasText: "Telegram" });
  await expect(option).toBeVisible();
  await option.getByRole("button").click();
  await expect(row).toBeVisible();
  expect(errors).toEqual([]);
});

test("сторож окна: прозрачная страница восстанавливается и пишется в журнал", async ({ page }) => {
  await open(page);
  // Имитируем недоигранную анимацию: страница и её секции прозрачны.
  await page.evaluate(() => {
    const p = document.querySelector("main")!.firstElementChild as HTMLElement;
    p.style.opacity = "0";
  });
  const pageOpacity = () =>
    page.evaluate(() => getComputedStyle(document.querySelector("main")!.firstElementChild!).opacity);
  expect(await pageOpacity()).toBe("0");
  await expect.poll(pageOpacity, { timeout: 6000 }).toBe("1");
  const misses = await page.evaluate(() => (window as unknown as { __mockMisses?: string[] }).__mockMisses ?? []);
  expect(misses).toContain("ui_log");
});

test("браузер: расширение снимает защиту WebRTC - понятное предупреждение", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __mockOverrides: unknown }).__mockOverrides = {
      apps_traffic: [
        { id: "custom-1", running: true, via_bridge: 9, direct: 0, proxy_extension: { id: "tux", name: "Tuxler", webrtc_only: true } },
      ],
    };
  });
  const errors = await open(page);
  await go(page, "Приложения");
  await expect(page.getByText("Tuxler выдаёт ваш IP через WebRTC")).toBeVisible();
  await expect(page.getByText("Прокси перехватывает Tuxler")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Отключить", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("первый запуск: «Сначала очистка» открывает Инструменты без прокси и ведёт обратно в мастер", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 700, height: 580 });
  await page.goto("/mock.html?still&fresh&platform=win");
  await expect(page.getByText("Вставьте прокси").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: /Сначала очистка/ }).click();
  await expect(page.getByRole("heading", { name: "Инструменты" })).toBeVisible();
  await expect(page.getByText("Очистка локальных данных Claude")).toBeVisible();
  await expect(page.getByText(/Прокси ещё не добавлен/)).toBeVisible();
  // Полоса остаётся при переходах по разделам, пока прокси не добавлен.
  await go(page, "Обзор");
  await expect(page.getByText(/Прокси ещё не добавлен/)).toBeVisible();
  await page.getByRole("button", { name: "Добавить прокси" }).click();
  await expect(page.getByText("Вставьте прокси").first()).toBeVisible();
  expect(errors).toEqual([]);
});
