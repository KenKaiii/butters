// Deep clean: developer leftovers on disk, in three groups, cleaned only after a confirm.
import { expect, test, type Page } from "@playwright/test";
import { installFakeBackend, type FakeState } from "./backend";

const MB = 1024 ** 2;
const GB = 1024 ** 3;

const row = (id: string, key: string, title: string, group: string, category: string, removal: string, disk: number, recommended: boolean, extra: Record<string, unknown> = {}) => ({
  id, key, kind: "deep_clean", title, detail: null, reason: "Downloads again when needed", note: null, memory: 0, cpu: 0,
  ports: [], recommended, polite: true, verb: "Clean", disk, group, category, upTo: false, removal, ...extra,
});

const DEEP = [
  row("c1-0", "xcode:derived-data", "Xcode build files", "rebuilds", "caches", "delete", 8.6 * GB, true, { reason: "Rebuilds on the next build" }),
  row("c1-1", "cache:pnpm", "pnpm store", "rebuilds", "caches", "delete", 16 * GB, false, { upTo: true, note: "pnpm is running" }),
  row("c1-2", "project:/Users/me/code/blobbies", "blobbies build files", "unused", "projects", "trash", 17.2 * GB, true, { reason: "Project untouched 4 months", detail: "node_modules, src-tauri/target" }),
  row("c1-3", "sim:907B5197-3CD8-4F8A-A7AF-9A83D10BE6D4", "Simulator: iPhone 15", "unused", "mobile", "delete", 3 * GB, false, { reason: "Last opened 2 months ago", note: "Deleted for good" }),
  row("c1-4", "xcode:archives", "Old Xcode archives ×3", "yours", "mobile", "trash", 5.3 * GB, false, { note: "Needed to read their crash reports" }),
];

async function open(page: Page, init: Record<string, unknown> = {}) {
  await page.addInitScript(installFakeBackend, { deep: DEEP, ...init } as Partial<FakeState>);
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).not.toHaveText("Checking…");
}
const fake = (page: Page) => page.evaluate(() => (window as any).__fake as FakeState);
/** Ids the panel asked the backend to clean, across every run. */
const closed = async (page: Page) =>
  (await fake(page)).calls.filter((c) => c.cmd === "deep_clean_run" || c.cmd === "close_item").flatMap((c) => (c.args as { ids?: string[]; id?: string }).ids ?? [(c.args as { id: string }).id]);

test("opens from the header with a real progress bar, then tabs by kind", async ({ page }) => {
  await open(page, { deepPolls: 3 });
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await expect(view.getByRole("heading", { name: "Deep clean" })).toBeVisible();
  // While looking: a bar with a value, what it's doing, and how much so far.
  const bar = view.getByRole("progressbar", { name: "Deep clean progress" });
  await expect(bar).toHaveAttribute("aria-valuenow", "55");
  await expect(view).toContainText("42 of 120 folders · 38.2 GB so far");
  await expect(view.getByRole("status").filter({ hasText: "Measuring" })).toBeVisible();
  // Then one tab per kind, with counts; empty kinds get no tab.
  await expect(bar).toHaveCount(0);
  const tabs = view.getByRole("tab");
  await expect(tabs).toHaveText(["Caches2", "Projects1", "Mobile2"]);
  await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
  await expect(view.getByRole("checkbox", { name: /Xcode build files/ })).toBeChecked();
  await expect(view.getByRole("checkbox", { name: /pnpm store/ })).not.toBeChecked();
  await expect(view.locator(".row", { hasText: "pnpm store" })).toContainText("up to 16.0 GB");
  await expect(view.locator(".row", { hasText: "blobbies" })).toHaveCount(0); // other tab
  await view.getByRole("tab", { name: /Projects/ }).click();
  await expect(view).toContainText("Ticked after 3 months");
  await expect(view.getByRole("checkbox", { name: /blobbies build files/ })).toBeChecked();
  // Arrow keys move along the tabs.
  await view.getByRole("tab", { name: /Projects/ }).press("ArrowRight");
  await expect(view.getByRole("tab", { name: /Mobile/ })).toBeFocused();
  await expect(view.getByRole("checkbox", { name: /Old Xcode archives/ })).not.toBeChecked();
  await expect(view.locator(".row", { hasText: "Simulator: iPhone 15" })).toContainText("Deleted for good");
  // The footer counts ticks from every tab.
  await expect(page.locator(".bar")).toContainText("Frees 25.8 GB");
});

test("confirms with what gets deleted and what goes to the Trash, then cleans only the ticked", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).click();
  await expect(page.locator(".bar .two")).toHaveText(/8\.6 GB deleted\s*17\.2 GB to the (Trash|Recycle Bin)/);
  await expect(view.getByRole("button", { name: "Cancel" })).toBeVisible();
  expect(await closed(page)).toEqual([]); // nothing happens before the confirm
  const confirm = view.locator(".bar-actions").getByRole("button", { name: "Clean 2" });
  await expect(confirm).toBeEnabled();
  await confirm.click();
  // The same big moment as closing apps, in disk terms, with the Trash caveat.
  const osd = page.locator(".osd");
  await expect(osd).toContainText("Cleaned up");
  await expect(osd).toContainText("2 cleaned");
  // Playwright's Chromium reports Windows, where it's the Recycle Bin.
  await expect(osd).toContainText(/Empty the (Trash|Recycle Bin) to free 17\.2 GB/);
  await expect(osd.getByLabel("25.8 GB cleaned up")).toBeVisible();
  await page.locator(".scrim").click();
  await expect(osd).toBeHidden();
  await expect(view.locator(".row", { hasText: "Xcode build files" })).toHaveCount(0);
  await expect(view.locator(".row", { hasText: "blobbies build files" })).toHaveCount(0);
  // Projects is empty now, so its tab goes; the rest stay.
  await expect(view.getByRole("tab")).toHaveText(["Caches1", "Mobile2"]);
  await view.getByRole("tab", { name: /Mobile/ }).click();
  await expect(view.locator(".row", { hasText: "Old Xcode archives" })).toBeVisible();
  expect(await closed(page)).toEqual(["c1-2", "c1-0"]); // biggest first
});

test("cleaning runs in the background: leave the view and come back to its progress and result", async ({ page }) => {
  await open(page, { deepStep: 6 });
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).click();
  const confirm = view.locator(".bar-actions").getByRole("button", { name: "Clean 2" });
  await expect(confirm).toBeEnabled();
  await confirm.click();
  // Progress in the footer, and a promise it carries on without the panel.
  await expect(view.getByRole("progressbar", { name: "Cleaning progress" })).toBeVisible();
  await expect(view.locator(".cleaning")).toContainText("Cleaning 1 of 2 · blobbies build files");
  await expect(view.locator(".cleaning")).toContainText("Keeps going if you close Butters");
  await expect(view.getByRole("button", { name: "Look again" })).toBeDisabled();
  // Leave mid-clean, wait, come back: it's still going (or done), not restarted.
  await view.getByRole("button", { name: "Back" }).click();
  await expect(page.getByRole("heading", { name: "Running smooth" })).toBeVisible();
  await page.getByRole("button", { name: "Deep clean" }).click();
  const again = page.getByRole("region", { name: "Deep clean" });
  await expect(again.getByRole("progressbar", { name: "Deep clean progress" })).toHaveCount(0); // no new look
  await expect(page.locator(".osd")).toContainText("2 cleaned", { timeout: 10_000 });
  // Shown once: the view left mid-clean doesn't add a second pop-up.
  await expect(page.locator(".osd")).toHaveCount(1);
  await page.locator(".scrim").click();
  await page.waitForTimeout(1500);
  await expect(page.locator(".osd")).toHaveCount(0);
  const f = await fake(page);
  expect(f.calls.filter((c) => c.cmd === "deep_clean_run")).toHaveLength(1);
  expect(f.calls.some((c) => c.cmd === "deep_clean" && (c.args as { rescan?: boolean }).rescan)).toBe(false);
  expect(f.calls.some((c) => c.cmd === "deep_clean_seen")).toBe(true);
});

test("a clean that finished while away shows its result once on return", async ({ page }) => {
  await open(page, {
    deep: DEEP.filter((r) => r.id !== "c1-0"),
    deepLast: { cleared: 8.6 * GB, trashed: 0, count: 1, keys: ["xcode:derived-data"], finishedAt: 1 },
  });
  await page.getByRole("button", { name: "Deep clean" }).click();
  await expect(page.locator(".osd")).toContainText("1 cleaned");
  await expect(page.locator(".osd").getByLabel("8.6 GB cleaned up")).toBeVisible();
  await page.locator(".scrim").click();
  await page.getByRole("region", { name: "Deep clean" }).getByRole("button", { name: "Back" }).click();
  await page.getByRole("button", { name: "Deep clean" }).click();
  await expect(page.getByRole("tab").first()).toBeVisible();
  await expect(page.locator(".osd")).toHaveCount(0);
});

test("a quick double-click asks instead of cleaning", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).dblclick();
  await expect(view.getByRole("button", { name: "Cancel" })).toBeVisible();
  expect(await closed(page)).toEqual([]);
});

test("with reduced motion, a double-click still only asks", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).dblclick({ delay: 120 });
  await expect(view.getByRole("button", { name: "Cancel" })).toBeVisible();
  expect(await closed(page)).toEqual([]);
});

test("a blocked item stays with its reason; others still clean", async ({ page }) => {
  await open(page, { closeAs: { "c1-0": "refused" } });
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).click();
  await expect(view.getByRole("button", { name: "Cancel" })).toBeVisible();
  const confirm = view.locator(".bar-actions").getByRole("button", { name: "Clean 2" });
  await expect(confirm).toBeEnabled();
  await confirm.click();
  await expect(view.locator(".row", { hasText: "Xcode build files" })).toContainText("Xcode is running. Quit it first");
  await expect(view.locator(".row", { hasText: "blobbies build files" })).toHaveCount(0);
  // Only what was really cleaned counts in the pop-up.
  await expect(page.locator(".osd")).toContainText("1 cleaned");
  await expect(page.locator(".osd").getByLabel("17.2 GB cleaned up")).toBeVisible();
});

test("Escape cancels the confirm first, then leaves", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Deep clean" }).click();
  const view = page.getByRole("region", { name: "Deep clean" });
  await view.getByRole("button", { name: "Clean 2" }).click();
  await expect(view.getByRole("button", { name: "Cancel" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(view.getByRole("button", { name: "Cancel" })).toHaveCount(0);
  await expect(view.getByRole("button", { name: "Clean 2" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Running smooth" })).toBeVisible();
  expect(await closed(page)).toEqual([]);
});

test("nothing found, and the cutoff is a setting", async ({ page }) => {
  await open(page, { deep: [] });
  await page.getByRole("button", { name: "Deep clean" }).click();
  await expect(page.getByText("Nothing to clean")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("region", { name: "Deep clean" })).toHaveCount(0);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("combobox", { name: /Deep clean/ }).selectOption({ label: "6 months" });
  await expect.poll(async () => (await fake(page)).settings.deep_clean_days).toBe(180);
});
