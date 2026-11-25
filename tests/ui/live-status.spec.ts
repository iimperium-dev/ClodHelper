import { expect, test, type Page } from "@playwright/test";

/*
  Живой статус моста (lib/status.tsx):
  - после долгого перерыва (скрытым окнам монитор снимки не шлёт) снимок
    считается первым: счётчики сбоев за перерыв не делают прокси «нестабильным»;
  - скрытое окно копит снимки без перерисовки и показывает их, когда его открыли;
  - пока программа отвечает на первые вопросы, окно не пустое: «Проверяю…».
*/

type Health = { uptime_sec: number; upstreams: { ok: number; fail: number }[] };
type Win = { __R: { bridge_health: Health }; __emit: (e: string, p: unknown) => void; __hidden?: boolean };

async function open(page: Page, query = "") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto(`/mock.html?still&platform=win&${query}`);
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Развернуть панель" }).click();
  return errors;
}

/** Снимок моста событием; опрос отдаёт тот же снимок (как настоящий мост). */
async function send(page: Page, patch: { ok?: number; fail?: number; uptime?: number }) {
  await page.evaluate((p) => {
    const w = window as unknown as Win;
    const h = structuredClone(w.__R.bridge_health);
    if (p.ok !== undefined) h.upstreams[0].ok = p.ok;
    if (p.fail !== undefined) h.upstreams[0].fail = p.fail;
    if (p.uptime !== undefined) h.uptime_sec = p.uptime;
    w.__R.bridge_health = h;
    w.__emit("bridge-health", structuredClone(h));
  }, patch);
}

test("долгий перерыв в снимках: сбои за перерыв не делают прокси нестабильным", async ({ page }) => {
  await page.clock.install();
  const errors = await open(page);
  await send(page, { ok: 1000, fail: 6 });
  await page.clock.fastForward(5_000);
  await send(page, { ok: 1100, fail: 6 });
  // Окно 10 минут было скрыто - снимков не было. За это время 34 сбоя.
  await page.clock.fastForward(10 * 60_000);
  await send(page, { ok: 1200, fail: 40 });
  await page.waitForTimeout(400);
  await expect(page.getByText("Прокси нестабилен")).toHaveCount(0);
  // Свежие сбои после перерыва - уже правда: «нестабилен».
  await page.clock.fastForward(5_000);
  await send(page, { ok: 1250, fail: 70 });
  await expect(page.getByText("Прокси нестабилен").first()).toBeVisible();
  expect(errors).toEqual([]);
});

test("скрытое окно не перерисовывается, а при показе - сразу свежие цифры", async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as Win;
    w.__hidden = false;
    Object.defineProperty(document, "hidden", { get: () => w.__hidden });
    Object.defineProperty(document, "visibilityState", { get: () => (w.__hidden ? "hidden" : "visible") });
    document.hasFocus = () => !w.__hidden;
  });
  const errors = await open(page);
  const sidebar = page.getByRole("complementary");
  await expect(sidebar.getByText(/^7ч 46м/)).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as Win).__hidden = true;
  });
  await send(page, { uptime: 3600 });
  await page.waitForTimeout(400);
  await expect(sidebar.getByText(/^7ч 46м/)).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as Win).__hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(sidebar.getByText(/^1ч 0м/)).toBeVisible();
  expect(errors).toEqual([]);
});

test("медленный запуск: знак программы и «Проверяю…», потом «Обзор»", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/mock.html?still&platform=win&slowboot");
  await expect(page.getByRole("status").getByText("Проверяю…")).toBeVisible();
  await page.screenshot({ path: "test-results/boot-checking.png" });
  await expect(page.getByText("Мост активен").first()).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole("status").getByText("Проверяю…")).toHaveCount(0);
  expect(errors).toEqual([]);
});
