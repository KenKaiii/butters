// Lifetime stats: everything Butters has freed since it was installed.
import { expect, test, type Page } from "@playwright/test";
import { installFakeBackend, type FakeState } from "./backend";

async function open(page: Page, init: Record<string, unknown> = {}) {
  await page.addInitScript(installFakeBackend, init as Partial<FakeState>);
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).not.toHaveText("Checking…");
}

test("stats screen shows lifetime totals from the header button", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Stats" }).click();
  const view = page.getByRole("region", { name: "Stats" });
  await expect(view.getByRole("heading", { name: "Butters so far" })).toBeVisible();
  await expect(view.getByLabel("41.2 GB freed in total")).toBeVisible();
  await expect(view.locator(".num")).toHaveText("41.2", { timeout: 4000 }); // counts up, then lands
  await expect(view).toContainText("Since");
  await expect(view).toContainText("12 days");
  await expect(view.locator(".cell", { hasText: "Closed" })).toContainText("128");
  await expect(view.locator(".cell", { hasText: "Best day" })).toContainText("6.1 GB");
  await expect(view.locator(".cell", { hasText: "Today" })).toContainText("Buttery 87");
  // Kinds, most first, in plain words.
  await expect(view.locator(".k-label")).toHaveText(["Dev servers", "Agent sessions", "MCP servers", "Browsers", "Containers", "Apps"]);
  // Back, and Escape also goes back.
  await view.getByRole("button", { name: "Back" }).click();
  await expect(page.getByRole("heading", { name: "Running smooth" })).toBeVisible();
  await page.getByRole("button", { name: "Stats" }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Running smooth" })).toBeVisible();
});

test("closing things adds to the lifetime total", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Close 2" }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  // The savings card mentions the all-time total.
  await expect(page.locator(".osd")).toContainText("since install");
  await page.locator(".scrim").click();
  await page.getByRole("button", { name: "Stats" }).click();
  const view = page.getByRole("region", { name: "Stats" });
  await expect(view.locator(".cell", { hasText: "Closed" })).toContainText("130");
  // 41.2 GB + 400 MB + 600 MB ≈ 42.2 GB
  await expect(view.locator(".num")).toHaveText("42.2", { timeout: 4000 });
});

test("footer score opens stats", async ({ page }) => {
  await open(page);
  await page.getByRole("checkbox", { name: /Vite dev server/ }).uncheck();
  await page.getByRole("checkbox", { name: /Claude Code session/ }).uncheck();
  await page.locator(".smooth-line").click();
  await expect(page.getByRole("heading", { name: "Butters so far" })).toBeVisible();
});

test("fresh install: nothing closed yet", async ({ page }) => {
  await open(page, {
    smoothness: { score: null, freedToday: 0, freedWeek: 0, week: [0, 0, 0, 0, 0, 0, 0], lifetime: { since: Math.floor(Date.now() / 1000), freed: 0, closed: 0, by_kind: {}, best_day: 0 } },
  });
  await page.getByRole("button", { name: "Stats" }).click();
  await expect(page.getByText("Nothing closed yet")).toBeVisible();
});

test("stats lines stay on one line", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Stats" }).click();
  await expect(page.locator(".k-label").first()).toBeVisible();
  const lines = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".since, .label, .cell strong, .k-label")].map((n) => ({
      text: n.textContent?.trim() ?? "",
      lines: Math.round(n.offsetHeight / parseFloat(getComputedStyle(n).lineHeight || "16")),
    })),
  );
  for (const l of lines) expect(l.lines, l.text).toBeLessThanOrEqual(1);
});

test("stats opens in place: no drop-in from below, no re-count when switching tabs", async ({ page }) => {
  await open(page);
  const headTop = async () => (await page.getByRole("heading", { level: 1 }).boundingBox())!.y;
  const mainTop = await headTop();
  await page.getByRole("button", { name: "Stats" }).click();
  // While it fades in, the Stats heading never sits below where it rests
  // (before the fix it started ~180 px lower). Sampled across the transition.
  const heading = page.getByRole("heading", { name: "Butters so far" });
  await heading.waitFor();
  const samples: number[] = [];
  for (let i = 0; i < 8; i++) {
    samples.push((await heading.boundingBox())!.y);
    await page.waitForTimeout(30);
  }
  for (const y of samples) expect(Math.abs(y - mainTop)).toBeLessThan(8);
  const num = page.locator(".hero .num");
  await expect(num).toHaveText("41.2", { timeout: 2000 });
  // Switching tabs doesn't replay the count from a low number.
  await page.getByRole("tab", { name: "Fan" }).click();
  await page.getByRole("tab", { name: "So far" }).click();
  await page.waitForTimeout(40);
  expect(Number(await num.textContent())).toBeGreaterThan(40);
});
