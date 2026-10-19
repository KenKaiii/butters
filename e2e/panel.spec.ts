// End-to-end tests of the Butters panel. The real built panel runs in
// WebKit (the engine the macOS app uses) and Chromium (close to Windows'
// WebView2), against a fake backend that records every command.
import { expect, test, type Page } from "@playwright/test";
import { installFakeBackend, type FakeState } from "./backend";

async function open(page: Page, init: Partial<FakeState> & { suggestions?: unknown[] } = {}) {
  await page.addInitScript(installFakeBackend, init);
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Running smooth");
}

const fake = (page: Page) => page.evaluate(() => (window as any).__fake as FakeState);
const settingsFile = async (page: Page) => (await fake(page)).settings;

test.describe("settings toggles", () => {
  for (const [label, key] of [
    ["Alerts", "alerts"],
    ["Sounds", "sounds"],
    ["Start at login", "launch_at_login"],
  ] as const) {
    test(`${label} switch saves when clicked anywhere on it`, async ({ page }) => {
      await open(page);
      await page.getByRole("button", { name: "Settings" }).click();
      const sw = page.getByRole("switch", { name: label });
      const before = (await settingsFile(page))[key];
      await expect(sw).toBeChecked({ checked: before });

      // Click the drawn knob/track exactly where a person would, not the hidden input.
      const box = (await sw.locator("xpath=..").boundingBox())!;
      await page.mouse.click(box.x + box.width - 6, box.y + box.height / 2);
      await expect.poll(async () => (await settingsFile(page))[key]).toBe(!before);
      await expect(sw).toBeChecked({ checked: !before });

      // And back again via the label text.
      await page.getByText(label, { exact: true }).click();
      await expect.poll(async () => (await settingsFile(page))[key]).toBe(before);
    });
  }

  test("sounds off means the panel stops asking for sounds to be heard", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("switch", { name: "Sounds" }).click({ force: true });
    await expect.poll(async () => (await settingsFile(page)).sounds).toBe(false);
  });
});

test.describe("closing things", () => {
  test("recommended items are pre-ticked and the footer totals them", async ({ page }) => {
    await open(page);
    await expect(page.getByRole("checkbox", { name: /Vite dev server/ })).toBeChecked();
    await expect(page.getByRole("checkbox", { name: /Google Chrome/ })).not.toBeChecked();
    await expect(page.locator(".bar")).toContainText("Frees");
    await expect(page.getByRole("button", { name: "Close 2" })).toBeVisible();
  });

  test("ticking a row updates the count and plays a tick", async ({ page }) => {
    await open(page);
    await page.getByText("Google Chrome").click();
    await expect(page.getByRole("button", { name: "Close 3" })).toBeVisible();
    await expect.poll(async () => (await fake(page)).sounds).toContain("tick");
  });

  test("close melts the rows away and shows the savings", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Close 2" }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();

    // Rows melt (class applied) in sync with the melt sound.
    await expect(page.locator(".pat.melting").first()).toBeVisible();
    await expect.poll(async () => (await fake(page)).sounds).toContain("melt");

    // Big centred savings: 400 MB + 600 MB = 1000 MB.
    const osd = page.locator(".osd");
    await expect(osd).toBeVisible();
    await expect(osd).toContainText("Freed");
    await expect(osd.locator(".num")).toHaveText("1000", { timeout: 4000 });
    await expect(osd).toContainText("2 closed");
    await expect.poll(async () => (await fake(page)).sounds).toContain("freed");

    // Centred in the panel.
    const p = (await page.locator(".panel").boundingBox())!;
    const o = (await osd.boundingBox())!;
    expect(Math.abs(o.x + o.width / 2 - (p.x + p.width / 2))).toBeLessThan(4);

    // Closed items are gone; it dismisses itself.
    await expect(osd).toBeHidden({ timeout: 6000 });
    const titles = page.locator(".row .title");
    await expect(titles).toHaveText(["Google Chrome"]);

    const calls = (await fake(page)).calls.filter((c) => c.cmd === "close_item");
    expect(calls).toHaveLength(2);
    expect(calls.every((c) => (c.args as any).force === false)).toBe(true);
  });

  test("an app asking to save offers force close, which needs a second confirm", async ({ page }) => {
    await open(page, { closeAs: { a: "still_running" } });
    await page.getByRole("checkbox", { name: /Claude Code session/ }).uncheck();
    await page.getByRole("button", { name: "Close 1" }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await expect(page.getByText("Still open, may be asking to save")).toBeVisible();

    await page.getByRole("button", { name: "Force close…" }).click();
    await expect(page.getByText("Unsaved work is lost")).toBeVisible();
    await page.getByRole("button", { name: "Force close", exact: true }).click();
    await expect(page.locator(".osd")).toBeVisible();
    const force = (await fake(page)).calls.filter((c) => c.cmd === "close_item" && (c.args as any).force);
    expect(force).toHaveLength(1);
  });

  test("ignore removes a row and remembers it", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Ignore Google Chrome" }).click();
    await expect(page.locator(".row .title", { hasText: "Google Chrome" })).toHaveCount(0);
    await expect(page.getByRole("status").filter({ hasText: "Ignoring Google Chrome" })).toBeVisible();
    expect((await settingsFile(page)).ignored).toContain("app:chrome");
  });

  test("Escape cancels the confirm step", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Close 2" }).click();
    await expect(page.getByText("Close 2?")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("button", { name: "Close 2" })).toBeVisible();
    expect((await fake(page)).calls.some((c) => c.cmd === "close_item")).toBe(false);
  });
});

test.describe("live and status", () => {
  test("memory and CPU update on their own", async ({ page }) => {
    await open(page);
    const mem = page.locator(".gauge").first().locator(".num");
    const first = await mem.textContent();
    await expect.poll(async () => mem.textContent(), { timeout: 6000 }).not.toBe(first);
  });

  test("protected list explains what's left alone", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: /Protected · 1/ }).click();
    await expect(page.getByText("Ghostty")).toBeVisible();
    await expect(page.getByText("In use")).toBeVisible();
  });

  test("open sound plays once on open", async ({ page }) => {
    await open(page);
    await expect.poll(async () => (await fake(page)).sounds.filter((s) => s === "open").length).toBe(1);
  });
});

test.describe("permissions and updates", () => {
  test("alerts permission: one click turns it on and the row goes away", async ({ page }) => {
    await open(page, { notify: "ask" });
    await expect(page.getByText("Turn on alerts")).toBeVisible();
    await page.getByRole("button", { name: "Turn on" }).click();
    await expect(page.getByText("Turn on alerts")).toHaveCount(0);
  });

  test("update ready: banner and settings both install it", async ({ page }) => {
    await open(page, { update: { current: "0.1.0", available: "0.2.0", checking: false, installing: false, error: null } });
    await expect(page.getByText("Update 0.2.0 ready")).toBeVisible();
    await page.getByRole("button", { name: "Update", exact: true }).click();
    await expect.poll(async () => (await fake(page)).calls.some((c) => c.cmd === "install_update")).toBe(true);
  });

  test("no update: settings offers a check", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByText("Version 0.1.0")).toBeVisible();
    await page.getByRole("button", { name: "Check" }).click();
    await expect.poll(async () => (await fake(page)).calls.some((c) => c.cmd === "check_update")).toBe(true);
  });
});

test("nothing wraps: every row stays one line", async ({ page }) => {
  await open(page);
  await expect(page.locator(".row .title").first()).toBeVisible();
  // One snapshot of every line, so a live refresh can't swap elements mid-check.
  const lines = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".row .title, .row .meta, .bar-text")].map((n) => ({
      text: n.textContent?.trim() ?? "",
      lines: Math.round(n.offsetHeight / parseFloat(getComputedStyle(n).lineHeight || "16")),
    })),
  );
  expect(lines.length).toBeGreaterThan(3);
  for (const l of lines) expect(l.lines, l.text).toBeLessThanOrEqual(1);
});

test.describe("protected and leaving", () => {
  test("protected app: memory turns into Close on hover, confirm closes it", async ({ page }) => {
    await open(page);
    await page.getByRole("button", { name: /Protected/ }).click();
    const row = page.locator(".protected li", { hasText: "Ghostty" });
    const close = row.getByRole("button", { name: "Close Ghostty" });
    await expect(close).toHaveCSS("opacity", "0");
    await row.hover();
    await expect(close).toHaveCSS("opacity", "1");
    await close.click();
    await row.getByRole("button", { name: "Quit?" }).click();
    await expect(page.locator(".osd")).toBeVisible();
    const calls = (await fake(page)).calls.filter((c) => c.cmd === "close_item");
    expect(calls.map((c) => (c.args as any).id)).toEqual(["x1-0"]);
  });

  test("locked protected items have no Close button", async ({ page }) => {
    await open(page, {
      protectedItems: [{ id: null, key: "self", verb: "Quit", title: "GG Coder", kind: "app", memory: 2 * 1024 ** 3, cpu: 3, reason: "Launched Butters" }],
    } as any);
    await page.getByRole("button", { name: /Protected/ }).click();
    const row = page.locator(".protected li", { hasText: "GG Coder" });
    await row.hover();
    await expect(row.getByRole("button")).toHaveCount(0);
  });

  test("clicking away melts the panel instead of cutting it", async ({ page }) => {
    await open(page);
    await page.evaluate(() => (window as any).__buttersLeave());
    const panel = page.locator(".panel");
    await expect(panel).toHaveCSS("animation-name", /panel-melt$/);
    await page.waitForTimeout(90);
    const mid = await panel.evaluate((el) => getComputedStyle(el).opacity);
    expect(Number(mid)).toBeGreaterThan(0.05); // still visible mid-melt, not blinked out
    await page.waitForTimeout(300);
    expect(Number(await panel.evaluate((el) => getComputedStyle(el).opacity))).toBeLessThan(0.05);
  });
});

test.describe("closing several at once", () => {
  const MB = 1024 ** 2;
  const mk = (id: string, title: string, mem: number) => ({
    id, key: `k:${id}`, kind: "dev_server", title, detail: "~/x", reason: "Terminal closed", note: null,
    memory: mem * MB, cpu: 0, ports: [], recommended: true, polite: true, verb: "Stop", disk: 0,
  });
  const four = [mk("a", "Vite dev server", 400), mk("b", "Next.js dev server", 700), mk("c", "Storybook dev server", 300), mk("d", "Expo dev server", 500)];

  test("the panel is held open for the whole close, then let go", async ({ page }) => {
    await open(page, { suggestions: four, sweep: [] } as any);
    await page.getByRole("button", { name: "Close 4" }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await expect(page.locator(".osd")).toBeVisible();
    await expect.poll(async () => (await fake(page)).calls.filter((c) => c.cmd === "hold_panel").length, { timeout: 5000 }).toBe(2);
    const calls = (await fake(page)).calls.map((c) => (c.cmd === "hold_panel" ? `hold:${(c.args as any).on}` : c.cmd));
    const first = calls.indexOf("hold:true");
    const last = calls.lastIndexOf("hold:false");
    const closes = calls.map((c, i) => (c === "close_item" ? i : -1)).filter((i) => i >= 0);
    expect(closes).toHaveLength(4);
    for (const i of closes) expect(i > first && i < last).toBe(true);
  });

  test("nothing is left behind after the melt: no slab, no line, no drops", async ({ page }) => {
    await open(page, { suggestions: four, sweep: [] } as any);
    await page.getByRole("button", { name: "Close 4" }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    // During: the footer says Closing… rather than counting down.
    await expect(page.locator(".bar")).toContainText("Closing");
    await page.locator(".osd").waitFor({ state: "detached", timeout: 8000 });
    await expect(page.locator(".pat.melting")).toHaveCount(0);
    await expect(page.locator(".drop-layer .drop")).toHaveCount(0);
    await expect(page.locator(".session-puddle")).toHaveCount(0);
    await expect(page.getByText("Nothing to close")).toBeVisible();
    // No butter-coloured element is lying around between the list and the footer.
    const leftovers = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>(".scroll *, .bar > *")].filter((el) => {
        const cs = getComputedStyle(el);
        const r = el.getBoundingClientRect();
        return r.height > 0 && r.height < 10 && r.width > 60 && Number(cs.opacity) > 0.05 && /rgb\(2[34]\d, 2[01]\d, 1[0-4]\d\)/.test(cs.backgroundColor + cs.backgroundImage);
      }).length,
    );
    expect(leftovers).toBe(0);
  });
});
