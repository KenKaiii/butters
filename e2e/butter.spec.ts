// The butter features: gauges, physics melt, knife, score, toast, texture.
import { expect, test, type Page } from "@playwright/test";
import { installFakeBackend, type FakeState } from "./backend";

const GB = 1024 ** 3;

async function open(page: Page, init: Record<string, unknown> = {}) {
  await page.addInitScript(installFakeBackend, init as Partial<FakeState>);
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).not.toHaveText("Checking…");
}
const fake = (page: Page) => page.evaluate(() => (window as any).__fake as FakeState);

test("memory is a butter stick that melts as RAM fills", async ({ page }) => {
  await open(page);
  const stick = page.locator(".stick .stick-body");
  await expect(page.getByRole("meter", { name: "Memory used" })).toBeVisible();
  const scale = async () => stick.evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a);
  await page.waitForTimeout(1200);
  const s = await scale();
  expect(s).toBeGreaterThan(0.06);
  expect(s).toBeLessThan(0.5); // ~18 of 24 GB used -> about a quarter of the stick left
});

test("CPU is sizzling butter: the fill is the value, bubbles only inside it", async ({ page }) => {
  await open(page);
  const meter = page.getByRole("meter", { name: "CPU in use" });
  await expect(meter).toBeVisible();
  // Nothing on the bar looks like a draggable handle.
  await expect(meter.locator(".pat")).toHaveCount(0);
  await page.waitForTimeout(1200);
  const pct = Number(await meter.getAttribute("aria-valuenow"));
  expect(pct).toBeGreaterThan(20);
  // Fill length matches the number.
  const fill = await meter.locator(".fill").evaluate((el) => parseFloat(getComputedStyle(el).getPropertyValue("--cpu")));
  expect(Math.abs(fill * 100 - pct)).toBeLessThan(3);
  // Every sizzling bubble sits inside the filled part.
  const bubbles = await meter.locator(".bubble.on").evaluateAll((els) => els.map((e) => parseFloat((e as HTMLElement).style.left)));
  expect(bubbles.length).toBeGreaterThan(0);
  for (const at of bubbles) expect(at).toBeLessThan(pct);
});

test("closing drops butter into the footer puddle", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Close 2" }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await expect.poll(() => page.locator(".drop-layer .drop").count(), { timeout: 3000 }).toBeGreaterThan(0);
  await expect(page.locator(".session-puddle")).toHaveClass(/filled/, { timeout: 4000 });
  await expect(page.locator(".session-puddle")).toHaveAttribute("title", /freed this session/);
  // Drops clean themselves up.
  await expect.poll(() => page.locator(".drop-layer .drop").count(), { timeout: 4000 }).toBe(0);
});

async function swipe(page: Page, title: string, fraction: number) {
  const row = page.locator(".row", { hasText: title });
  const box = (await row.boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + 60, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(box.x + 60 + (box.width * fraction * i) / 8, y);
  await page.mouse.up();
  return row;
}

test("knife: a full swipe arms Slice, and Slice closes it politely", async ({ page }) => {
  await open(page);
  const before = await page.getByRole("checkbox", { name: /Google Chrome/ }).isChecked();
  await swipe(page, "Google Chrome", 0.6);
  const sliceBtn = page.getByRole("button", { name: "Slice?" });
  await expect(sliceBtn).toBeVisible();
  // The drag didn't tick the box.
  expect(await page.getByRole("checkbox", { name: /Google Chrome/ }).isChecked()).toBe(before);
  // Nothing is closed until Slice is clicked.
  expect((await fake(page)).calls.some((c) => c.cmd === "close_item")).toBe(false);
  await sliceBtn.click();
  await expect(page.locator(".osd")).toBeVisible();
  const call = (await fake(page)).calls.find((c) => c.cmd === "close_item")!;
  expect((call.args as any).id).toBe("c");
  expect((call.args as any).force).toBe(false);
});

test("knife: a short swipe springs back; Escape disarms", async ({ page }) => {
  await open(page);
  const row = await swipe(page, "Google Chrome", 0.12);
  await expect(page.getByRole("button", { name: "Slice?" })).toHaveCount(0);
  await expect.poll(() => row.evaluate((el) => getComputedStyle(el).transform)).toMatch(/none|matrix\(1, 0, 0, 1, 0, 0\)/);
  await swipe(page, "Google Chrome", 0.6);
  await expect(page.getByRole("button", { name: "Slice?" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Slice?" })).toHaveCount(0);
  expect((await fake(page)).calls.some((c) => c.cmd === "close_item")).toBe(false);
});

test("footer shows today's buttery score and the week", async ({ page }) => {
  await open(page);
  await page.getByRole("checkbox", { name: /Vite dev server/ }).uncheck();
  await page.getByRole("checkbox", { name: /Claude Code session/ }).uncheck();
  const line = page.locator(".smooth-line");
  await expect(line).toContainText("Buttery 87");
  await expect(line).toContainText("3.2 GB this week");
  await expect(page.locator(".spark rect")).toHaveCount(7);
  await expect(page.locator(".spark rect.today")).toHaveCount(1);
});

test("toast mode: nothing to close and smooth -> toast gets buttered", async ({ page }) => {
  await open(page, { suggestions: [] });
  await expect(page.locator(".toast-mode")).toBeVisible();
  await expect(page.getByText("All buttered up")).toBeVisible();
  // Left-aligned with the section heading, like every other row.
  // Compare the toaster to where the heading's text starts (inside its padding).
  const toastLeft = (await page.locator(".toast-mode svg").boundingBox())!.x;
  const textLeft = await page.locator("h2.section").evaluate((el) => el.getBoundingClientRect().left + parseFloat(getComputedStyle(el).paddingLeft));
  expect(Math.abs(toastLeft - textLeft)).toBeLessThan(3);
  await expect.poll(async () => (await fake(page)).sounds.filter((s) => s === "open").length, { timeout: 3000 }).toBe(2);
});

test("toast mode stays away when there's work or strain", async ({ page }) => {
  await open(page);
  await expect(page.locator(".toast-mode")).toHaveCount(0);
  await page.close();
});

test("texture: grain and sheen, warmer under strain", async ({ page }) => {
  await open(page, { level: "strained" });
  const panel = page.locator(".panel");
  const grain = await panel.evaluate((el) => getComputedStyle(el, "::before").backgroundImage);
  expect(grain).toContain("feTurbulence");
  await expect.poll(() => panel.evaluate((el) => getComputedStyle(el).getPropertyValue("--warmth").trim())).toBe("1");
});

test("texture: cool when smooth", async ({ page }) => {
  await open(page);
  await expect.poll(() => page.locator(".panel").evaluate((el) => getComputedStyle(el).getPropertyValue("--warmth").trim())).toBe("0");
  void GB;
});
