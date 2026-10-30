import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:4173/aarambh/",
    // Most tests mock api.github.com with page routes; a service worker would
    // sit in front of those. The service worker has its own test.
    serviceWorkers: "block",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node serve.mjs",
    url: "http://127.0.0.1:4173/aarambh/",
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: "chromium", use: { ...devices["Pixel 7"] } },
    { name: "webkit", use: { ...devices["iPhone 15"] } },
  ],
});
