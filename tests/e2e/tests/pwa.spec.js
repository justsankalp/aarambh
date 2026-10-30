import { expect, test } from "@playwright/test";
import { connectedDevice, mockGitHub } from "./mock-github.js";

test("manifest and icons use relative paths that resolve under /aarambh/", async ({ page, request }) => {
  await page.goto("./");
  const href = await page.locator('link[rel="manifest"]').getAttribute("href");
  expect(href).toBe("manifest.webmanifest");
  const manifest = await (await request.get(href)).json();
  expect(manifest.start_url).toBe("./");
  expect(manifest.scope).toBe("./");
  expect(manifest.display).toBe("standalone");
  for (const icon of manifest.icons) {
    expect(icon.src.startsWith("/")).toBe(false);
    expect((await request.get(icon.src)).status(), icon.src).toBe(200);
  }
  expect(manifest.icons.some((i) => i.purpose === "maskable")).toBe(true);
  const apple = await page.locator('link[rel="apple-touch-icon"]').getAttribute("href");
  expect((await request.get(apple)).status()).toBe(200);
  await expect(page.locator('meta[name="viewport"]')).toHaveAttribute("content", /viewport-fit=cover/);
  await expect(page.locator('meta[name="apple-mobile-web-app-capable"]')).toHaveAttribute("content", "yes");
});

test.describe("service worker", () => {
  test.use({ serviceWorkers: "allow" });

  test("the app opens offline after one visit and GitHub calls bypass it", async ({ page, context, browserName }) => {
    test.skip(browserName !== "chromium", "Playwright's offline mode with service workers is Chromium-only");
    const gh = await mockGitHub(page);
    await connectedDevice(page);
    await page.goto("./#notes");
    await page.evaluate(() => navigator.serviceWorker.ready);
    // Reload so the page is controlled and the shell is cached.
    await page.reload();
    await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);
    await expect.poll(() => page.evaluate(async () => (await caches.keys()).length)).toBeGreaterThan(0);

    const swUrls = [];
    context.on("request", (r) => {
      if (r.serviceWorker()) swUrls.push(r.url());
    });
    await context.setOffline(true);
    gh.offline = true; // page routes answer even when the context is offline
    await page.reload();
    await expect(page.getByRole("navigation", { name: "Sections" })).toBeVisible();
    await expect(page.locator(".sync-line")).toContainText("Offline");
    expect(swUrls.filter((u) => u.includes("api.github.com"))).toEqual([]);
    await context.setOffline(false);
  });
});
