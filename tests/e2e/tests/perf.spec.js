// Load timings on a throttled mobile profile (Lighthouse's: 150 ms RTT,
// 1.6 Mbps down, 750 kbps up, 4x CPU slowdown). Opt-in: PERF=1 npx
// playwright test tests/perf.spec.js --project=chromium
import { expect, test } from "@playwright/test";
import { connectedDevice, dayFile, mockGitHub, note, utcDay } from "./mock-github.js";

test.skip(!process.env.PERF, "set PERF=1 to measure");
test.use({ serviceWorkers: "allow" });

const now = Date.now();
const notes = Array.from({ length: 40 }, (_, i) => note({ ms: now - (40 - i) * 60_000, n: i + 1, author: i % 2 ? "Asha" : "Sankalp", kind: ["idea", "question", "evidence", "note"][i % 4], text: `Note ${i + 1}: a sentence or two about clinics, buyers and what we learned.` }));

async function throttle(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Network.enable");
  await cdp.send("Network.emulateNetworkConditions", { offline: false, latency: 150, downloadThroughput: (1.6 * 1024 * 1024) / 8, uploadThroughput: (750 * 1024) / 8 });
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
}

async function timings(page, start) {
  const fcp = await page.evaluate(() => performance.getEntriesByName("first-contentful-paint")[0]?.startTime ?? null);
  await page.getByRole("navigation", { name: "Sections" }).waitFor();
  const app = Date.now() - start;
  await page.locator("article.card").first().waitFor();
  const content = Date.now() - start;
  return { fcp: Math.round(fcp), app, content };
}

test("first visit and repeat visit", async ({ page }) => {
  test.setTimeout(120_000);
  // GitHub answers instantly here; the throttling is on the app's own files.
  await mockGitHub(page, { files: { [`data/events/${utcDay(now)}.json`]: dayFile(notes) } });
  await connectedDevice(page);
  await throttle(page);

  let start = Date.now();
  await page.goto("./#notes");
  const first = await timings(page, start);
  await page.evaluate(() => navigator.serviceWorker.ready);
  // Let the service worker finish caching the shell.
  await expect.poll(() => page.evaluate(async () => (await caches.open("aarambh-shell-v1").then((c) => c.keys())).length), { timeout: 30_000 }).toBeGreaterThanOrEqual(4);

  // Chrome's throttling doesn't reach the service worker, whose fetch of the
  // page decides repeat-visit paint, so the page's network time is added
  // here instead (context routes do see service worker requests).
  const repeat = {};
  for (const [name, delay] of [["typical (450 ms page fetch)", 450], ["stalled network", 5000]]) {
    await page.context().unrouteAll({ behavior: "ignoreErrors" });
    await page.context().route(/\/aarambh\/(index\.html)?(\?.*)?$/, async (route) => {
      await new Promise((r) => setTimeout(r, delay));
      await route.continue().catch(() => {});
    });
    start = Date.now();
    await page.reload();
    repeat[name] = await timings(page, start);
  }
  await page.context().unrouteAll({ behavior: "ignoreErrors" });
  await page.context().setOffline(true);
  start = Date.now();
  await page.reload();
  repeat.offline = await timings(page, start);
  await page.context().setOffline(false);

  // And posting: tap Post to the note on screen.
  await page.getByRole("button", { name: "New note" }).click();
  await page.keyboard.type("Timing a post");
  const t0 = Date.now();
  await page.getByRole("button", { name: "Post" }).click();
  await page.getByText("Timing a post").waitFor();
  const post = Date.now() - t0;

  console.log(JSON.stringify({ first, repeat, postMs: post }));
  for (const r of Object.values(repeat)) expect(r.fcp).toBeLessThan(1000);
});
