import { expect, test } from "@playwright/test";
import { REPO, connectedDevice, dayFile, mockGitHub, note, utcDay } from "./mock-github.js";

const now = Date.now();
const DAY = 86_400_000;

test("the week row shows who wrote on each of the last seven days", async ({ page }) => {
  const events = [
    note({ ms: now - 60_000, n: 1, author: "Asha", kind: "idea", text: "Today, Asha" }),
    note({ ms: now - 30_000, n: 2, author: "Sankalp", kind: "note", text: "Today, Sankalp" }),
    note({ ms: now - DAY, n: 3, author: "Asha", kind: "idea", text: "Yesterday, Asha only" }),
    note({ ms: now - 10 * DAY, n: 4, author: "Asha", kind: "note", text: "Too old to count" }),
  ];
  const files = {};
  for (const e of events) {
    const path = `data/events/${utcDay(Date.parse(e.ts))}.json`;
    files[path] = dayFile([...(files[path] ? JSON.parse(files[path]) : []), e]);
  }
  await mockGitHub(page, { files });
  await connectedDevice(page);
  await page.goto("./#notes");
  await expect(page.getByText("Today, Sankalp")).toBeVisible();

  const dots = page.locator(".week-dots .dot");
  await expect(dots).toHaveCount(7);
  await expect(dots.nth(6)).toHaveClass(/both/);
  await expect(dots.nth(5)).toHaveClass(/one/);
  for (let i = 0; i < 5; i++) await expect(dots.nth(i)).toHaveClass(/none/);
  await expect(page.getByRole("img", { name: "Last 7 days: both of you wrote on 1, one of you on 1." })).toBeVisible();
  await expect(page.locator(".idea-count")).toHaveText("2 open · 0 decided");
});

test("a revoked token says so and points to Settings", async ({ page }) => {
  await mockGitHub(page);
  await page.addInitScript((repo) => {
    localStorage.setItem("aarambh.repo", repo);
    localStorage.setItem("aarambh.token", "github_pat_revoked");
    localStorage.setItem("aarambh.name", "Sankalp");
  }, REPO);
  await page.goto("./#notes");
  const line = page.locator(".sync-line");
  await expect(line).toContainText("GitHub rejected the token");
  await line.getByRole("link", { name: "Open settings" }).click();
  await expect(page).toHaveURL(/#settings$/);
});

test("entries from a newer version are kept out of view with a notice", async ({ page }) => {
  const ok = note({ ms: now - 1000, n: 1, text: "Readable note" });
  const text = `[\n  ${JSON.stringify(ok)},\n  {"v":2,"id":"01JZZZZZZZZZZZZZZZZZZZZZZZ","type":"poll","question":"?"}\n]\n`;
  await mockGitHub(page, { files: { [`data/events/${utcDay(now)}.json`]: text } });
  await connectedDevice(page);
  await page.goto("./#notes");
  await expect(page.getByText("Readable note")).toBeVisible();
  await expect(page.locator(".notice")).toHaveText("1 entry was written by a newer version of aarambh and isn't shown. Reload to update.");
});
