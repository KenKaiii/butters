import { expect, test } from "@playwright/test";
import { installFakeBackend } from "./backend";

// The status pat sits on the heading's centre line in every state, and is
// the animated butter pat (not a plain dot).
for (const level of ["smooth", "busy", "strained"] as const) {
  test(`status pat is centred on the heading when ${level}`, async ({ page }) => {
    await page.addInitScript(installFakeBackend, {});
    await page.addInitScript((lvl) => {
      const t = setInterval(() => {
        const i = (window as any).__TAURI_INTERNALS__;
        if (!i || i.__patched) return;
        const orig = i.invoke;
        i.invoke = async (cmd: string, args: unknown) => {
          const r = await orig(cmd, args);
          if (cmd === "scan" || cmd === "live_stats") (r as any).level = lvl;
          return r;
        };
        i.__patched = true;
        clearInterval(t);
      }, 0);
    }, level);
    await page.goto("/");
    const h1 = page.getByRole("heading", { level: 1 });
    await expect(h1).not.toHaveText("Checking…");
    await page.waitForTimeout(1100); // let the shape settle

    const pat = page.locator(".status-pat");
    await expect(pat).toHaveClass(new RegExp(`level-${level}`));
    expect(await pat.locator(".body > path").count()).toBe(3); // side, front, top

    const icon = (await pat.boundingBox())!;
    const text = (await h1.boundingBox())!;
    expect(Math.abs(icon.y + icon.height / 2 - (text.y + text.height / 2))).toBeLessThanOrEqual(1.5);
    await page.locator(".status").screenshot({ path: test.info().outputPath(`status-${level}.png`) });
  });
}
