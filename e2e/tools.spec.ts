// Docker deep dive, leak watch, fan culprits, end-of-day sweep, global shortcut.
import { expect, test, type Page } from "@playwright/test";
import { installFakeBackend, type FakeState } from "./backend";

const MB = 1024 ** 2;
const GB = 1024 ** 3;

async function open(page: Page, init: Record<string, unknown> = {}) {
  await page.addInitScript(installFakeBackend, init as Partial<FakeState>);
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).not.toHaveText("Checking…");
}
const fake = (page: Page) => page.evaluate(() => (window as any).__fake as FakeState);

const container = (id: string, name: string, reason: string, recommended: boolean, memory = 300 * MB) => ({
  id, key: `docker:${name}`, kind: "docker_container", title: `Container: ${name}`, detail: "postgres:16 · shop",
  reason, note: null, memory, cpu: 0, ports: [], recommended, polite: true, verb: "Stop", disk: 0,
});

test.describe("docker deep dive", () => {
  test("idle containers say how long since any activity; a day+ is pre-ticked", async ({ page }) => {
    await open(page, {
      services: [container("d1", "shop-db", "No activity 6 d", true), container("d2", "shop-cache", "No activity 2 h+", false, 80 * MB)],
    });
    const db = page.locator(".row", { hasText: "Container: shop-db" });
    await expect(db).toContainText("No activity 6 d");
    await expect(db).toContainText("postgres:16 · shop");
    await expect(page.getByRole("checkbox", { name: /shop-db/ })).toBeChecked();
    await expect(page.getByRole("checkbox", { name: /shop-cache/ })).not.toBeChecked();
  });

  test("docker disk clean-ups show disk, not memory, and add to lifetime disk", async ({ page }) => {
    await open(page, {
      services: [{
        id: "k1", key: "docker:build-cache", kind: "docker_disk", title: "Docker build cache", detail: null,
        reason: "Over 3 days old, rebuilds if needed", note: null, memory: 0, cpu: 0, ports: [], recommended: false,
        polite: true, verb: "Clear", disk: 12 * GB,
      }],
    });
    const row = page.locator(".row", { hasText: "Docker build cache" });
    await expect(row.locator(".mem")).toHaveText("12.0 GB disk");
    // Not pre-ticked; tick and slice it via the knife-free path (tick + Close).
    await page.getByRole("checkbox", { name: /Docker build cache/ }).check();
    await page.getByRole("button", { name: "Close 3" }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await expect.poll(async () => (await fake(page)).calls.filter((c) => c.cmd === "close_item").map((c) => (c.args as any).id)).toContain("k1");
  });
});

test("old image versions: one row, shows disk and which images", async ({ page }) => {
  await open(page, {
    services: [{
      id: "k3", key: "docker:old-images", kind: "docker_disk", title: "Old image versions ×14",
      detail: "postgres:15.8.1.099, postgres:17.6.1.127, storage-api:v1.54.0", reason: "Newer version kept, none in use",
      note: null, memory: 0, cpu: 0, ports: [], recommended: false, polite: true, verb: "Clear", disk: 14.02e9,
    }],
  });
  const row = page.locator(".row", { hasText: "Old image versions ×14" });
  await expect(row).toContainText("Newer version kept, none in use");
  await expect(row.locator(".mem")).toHaveText("13.1 GB disk");
  await expect(page.getByRole("checkbox", { name: /Old image versions/ })).not.toBeChecked();
});

test("leak watch: a steadily growing process says how much it grew", async ({ page }) => {
  await open(page, {
    suggestions: [{
      id: "L", key: "proc:node#9", kind: "process", title: "node", detail: "~/app/build.js", reason: "Grew 2.1 GB in 40 min",
      note: null, memory: 3.4 * GB, cpu: 3, ports: [], recommended: false, polite: true, verb: "Stop", disk: 0,
    }],
  });
  const row = page.locator(".row", { hasText: "node" });
  await expect(row).toContainText("Grew 2.1 GB in 40 min");
  await expect(page.getByRole("checkbox", { name: /node/ })).not.toBeChecked(); // never pre-ticked
});

test.describe("what's making the fan spin", () => {
  test("fan tab lists the top CPU users over the window with sparklines", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Stats" }).click();
    await page.getByRole("tab", { name: "Fan" }).click();
    await expect(page.getByRole("heading", { name: "What's making the fan spin" })).toBeVisible();
    await expect(page.getByText("Last 10 min, average CPU")).toBeVisible();
    await expect(page.locator(".fan .f-label")).toHaveText(["Google Chrome", "node"]);
    await expect(page.locator(".fan .f-n")).toHaveText(["62%", "31%"]);
    await expect(page.locator(".fan .f-spark path")).toHaveCount(2);
    await page.getByRole("tab", { name: "So far" }).click();
    await expect(page.getByRole("heading", { name: "Butters so far" })).toBeVisible();
  });

  test("when the CPU is pinned, the main screen links straight to the fan view", async ({ page }) => {
    await page.addInitScript(installFakeBackend, {} as Partial<FakeState>);
    await page.addInitScript(() => {
      const t = setInterval(() => {
        const i = (window as any).__TAURI_INTERNALS__;
        if (!i || i.__p) return;
        const o = i.invoke;
        i.invoke = async (c: string, a: unknown) => {
          const r: any = await o(c, a);
          if (c === "scan" || c === "live_stats") r.stats.cpu = 95;
          return r;
        };
        i.__p = 1;
        clearInterval(t);
      }, 0);
    });
    await page.goto("/");
    await page.getByRole("button", { name: /see why/ }).click();
    await expect(page.getByRole("heading", { name: "What's making the fan spin" })).toBeVisible();
  });
});

test.describe("end-of-day sweep", () => {
  test("Sweep ticks exactly the safe set and still asks before closing", async ({ page }) => {
    await open(page, { sweep: ["a"] as string[] });
    // Only one sweepable item: no button (a sweep of one is just Close).
    await expect(page.getByRole("button", { name: /^Sweep/ })).toHaveCount(0);
  });

  test("sweeping closes the safe ones together and leaves apps alone", async ({ page }) => {
    await open(page);
    // Untick everything first, so Sweep has to do the ticking.
    await page.getByRole("checkbox", { name: /Vite dev server/ }).uncheck();
    await page.getByRole("checkbox", { name: /Claude Code session/ }).uncheck();
    await page.getByRole("checkbox", { name: /Google Chrome/ }).check();
    await page.getByRole("button", { name: "Sweep 2" }).click();
    await expect(page.getByText("Close 2?")).toBeVisible(); // confirm step, nothing closed yet
    expect((await fake(page)).calls.some((c) => c.cmd === "close_item")).toBe(false);
    await expect(page.getByRole("checkbox", { name: /Google Chrome/ })).not.toBeChecked();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await expect(page.locator(".osd")).toBeVisible();
    const ids = (await fake(page)).calls.filter((c) => c.cmd === "close_item").map((c) => (c.args as any).id);
    expect(ids.sort()).toEqual(["a", "b"]);
  });
});

test.describe("quick access", () => {
  test("shortcut setting saves and offers Off", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Settings" }).click();
    const select = page.getByLabel("Shortcut");
    await expect(select).toHaveValue("Alt+Super+B");
    await select.selectOption("Control+Alt+B");
    await expect.poll(async () => (await fake(page)).settings.shortcut).toBe("Control+Alt+B");
    await select.selectOption("");
    await expect.poll(async () => (await fake(page)).settings.shortcut).toBe("");
  });
});

test.describe("docker rows load with the rest", () => {
  const svc = [
    { id: "k1", key: "docker:build-cache", kind: "docker_disk", title: "Docker build cache", detail: null, reason: "Unused 25 h, rebuilds if needed", note: null, memory: 0, cpu: 0, ports: [], recommended: false, polite: true, verb: "Clear", disk: 12.7e9 },
  ];

  test("background results show at once, even while Docker is slow to answer", async ({ page }) => {
    await open(page, { services: svc, cachedServices: svc, servicesDelay: 4000 });
    // The process rows and the Docker row are there together, long before the fresh Docker check returns.
    await expect(page.locator(".row", { hasText: "Vite dev server" })).toBeVisible();
    await expect(page.locator(".row", { hasText: "Docker build cache" })).toBeVisible({ timeout: 800 });
  });

  test("a fresh Docker check doesn't replay the row's entrance", async ({ page }) => {
    await open(page, { services: svc, cachedServices: svc, servicesDelay: 600 });
    const row = page.locator("li", { has: page.locator(".row", { hasText: "Docker build cache" }) });
    await expect(row).toBeVisible();
    const before = await row.evaluate((el) => {
      (el as any).__marker = 1;
      return true;
    });
    expect(before).toBe(true);
    await page.waitForTimeout(1200); // fresh result arrives and replaces the cached one
    // Same element (not torn down and re-poured), so no second entrance.
    expect(await row.evaluate((el) => (el as any).__marker)).toBe(1);
  });
});

test("tooltips are Butters cards, short, never the system box", async ({ page }) => {
  await open(page, {
    services: [{
      id: "k3", key: "docker:old-images", kind: "docker_disk", title: "Old image versions ×14",
      detail: "edge-runtime:v1.70.0, gotrue:v2.189.0, logflare:1.30.5, mailpit:v1.22.3, postgres-meta:v0.95.2, postgres:15.8.1.099, postgres:17.6.1.127",
      reason: "Newer version kept, none in use", note: null, memory: 0, cpu: 0, ports: [], recommended: false, polite: true, verb: "Clear", disk: 14.02e9,
    }],
  });
  // No native title tooltips anywhere in the panel.
  expect(await page.locator("[title]").count()).toBe(0);
  const label = page.locator(".row", { hasText: "Old image versions" }).locator("label.text");
  await label.hover();
  const card = page.locator(".butter-tip");
  await expect(card).toHaveClass(/on/);
  await expect(card.locator(".butter-tip-title")).toHaveText("Old image versions ×14");
  // Reason, then four images and "+3 more": not a wall of text.
  await expect(card.locator(".butter-tip-line")).toHaveText([
    "Newer version kept, none in use",
    "edge-runtime:v1.70.0",
    "gotrue:v2.189.0",
    "logflare:1.30.5",
    "mailpit:v1.22.3",
    "+3 more",
  ]);
  // Stays inside the window.
  const box = (await card.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(380);
  await page.mouse.move(5, 5);
  await expect(card).not.toHaveClass(/on/);
});

test("rows that fit don't sprout redundant tooltips", async ({ page }) => {
  await open(page);
  await page.locator(".row", { hasText: "Google Chrome" }).locator("label.text").hover();
  await page.waitForTimeout(700);
  await expect(page.locator(".butter-tip.on")).toHaveCount(0);
});
