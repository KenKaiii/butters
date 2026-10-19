import { defineConfig, devices } from "@playwright/test";

// Panel end-to-end tests: the built panel, served by Vite preview, in WebKit
// (what the macOS app uses) and Chromium (close to Windows' WebView2).
export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.ts",
  fullyParallel: true,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    viewport: { width: 380, height: 580 },
    trace: "retain-on-failure",
  },
  projects: [
    { name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 380, height: 580 } } },
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 380, height: 580 } } },
  ],
  webServer: {
    command: "pnpm build && pnpm exec vite preview --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
