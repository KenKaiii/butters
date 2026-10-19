import { expect, test } from "@playwright/test";
import { installFakeBackend } from "./backend";
const MB = 1024 ** 2;
test.use({ deviceScaleFactor: 2 });
test("four close frames", async ({ page, browserName }) => {
  test.skip(browserName !== "webkit");
  const mk = (id: string, title: string, mem: number) => ({ id, key: `k:${id}`, kind: "dev_server", title, detail: "~/x", reason: "Terminal closed", note: null, memory: mem * MB, cpu: 0, ports: [], recommended: true, polite: true, verb: "Stop", disk: 0 });
  await page.addInitScript(installFakeBackend, { suggestions: [mk("a","Vite dev server",400), mk("b","Next.js dev server",700), mk("c","Storybook dev server",300), mk("d","Expo dev server",500), { ...mk("e","Google Chrome",2048), recommended: false, kind: "browser", reason: "Unused 2 h", verb: "Quit" }], sweep: [] } as any);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Close 4" })).toBeVisible();
  await page.waitForTimeout(1200);
  await page.getByRole("button", { name: "Close 4" }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  for (let i = 0; i < 14; i++) {
    await page.screenshot({ path: `/tmp/line-${String(i).padStart(2, "0")}.png` });
    await page.waitForTimeout(140);
  }
});
