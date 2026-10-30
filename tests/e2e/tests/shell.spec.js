import { expect, test } from "@playwright/test";
import { GOOD_TOKEN, REPO, connectedDevice, dayFile, mockGitHub, note, utcDay } from "./mock-github.js";

const now = Date.now();
const today = `data/events/${utcDay(now)}.json`;

function sampleFiles() {
  return {
    [today]: dayFile([
      note({ ms: now - 60_000, n: 1, author: "Sankalp", kind: "idea", text: "Clinics will pay for intake automation" }),
      note({ ms: now - 30_000, n: 2, author: "Asha", kind: "question", text: "Who signs the cheque at a clinic?" }),
    ]),
  };
}

test.describe("first run", () => {
  test("goes to settings, connects, then shows notes", async ({ page }) => {
    await mockGitHub(page, { files: sampleFiles() });
    await page.goto("./");
    await expect(page).toHaveURL(/#settings$/);
    await expect(page.getByRole("heading", { name: "Connect this device" })).toBeVisible();
    await expect(page.getByLabel("Repository")).toHaveValue(REPO);

    await page.getByLabel("GitHub token").fill(GOOD_TOKEN);
    await page.getByLabel("Your name").fill("Sankalp");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("status").filter({ hasText: `Connected to ${REPO}.` })).toBeVisible();

    await page.getByRole("link", { name: "Notes" }).click();
    await expect(page.getByText("Clinics will pay for intake automation")).toBeVisible();
    await expect(page.getByText("Who signs the cheque at a clinic?")).toBeVisible();
    // Newest first.
    const texts = await page.locator(".note-text").allTextContents();
    expect(texts).toEqual(["Who signs the cheque at a clinic?", "Clinics will pay for intake automation"]);
    await expect(page.locator(".sync-line")).toContainText("Synced");

    // Names from the notebook become one-tap choices for re-entry.
    await page.getByRole("link", { name: "Settings" }).click();
    await expect(page.getByRole("group", { name: "Names in this notebook" }).getByRole("button", { name: "Asha" })).toBeVisible();
  });

  test("a bad token shows a clear error and isn't saved", async ({ page }) => {
    await mockGitHub(page);
    await page.goto("./");
    await page.getByLabel("GitHub token").fill("github_pat_wrong");
    await page.getByLabel("Your name").fill("Sankalp");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("alert")).toHaveText("GitHub rejected the token. Check it hasn't expired.");
    expect(await page.evaluate(() => localStorage.getItem("aarambh.token"))).toBeNull();
  });

  test("missing fields are caught before calling GitHub", async ({ page }) => {
    const gh = await mockGitHub(page);
    await page.goto("./");
    await page.getByRole("button", { name: "Save and connect" }).click();
    await expect(page.getByRole("alert")).toHaveText("Paste your GitHub token.");
    expect(gh.log.filter((l) => !l.includes("/contents/"))).toEqual([]);
  });
});

test.describe("local-first", () => {
  test("cached notes render when GitHub can't be reached", async ({ page }) => {
    const gh = await mockGitHub(page, { files: sampleFiles() });
    await connectedDevice(page);
    await page.goto("./#notes");
    await expect(page.getByText("Clinics will pay for intake automation")).toBeVisible();

    gh.offline = true;
    await page.reload();
    await expect(page.getByText("Clinics will pay for intake automation")).toBeVisible();
    await expect(page.locator(".sync-line")).toContainText("Offline");
  });

  test("an unchanged listing is a cheap 304 and files aren't refetched", async ({ page, browserName }) => {
    // Playwright's WebKit driver can't fulfill a mocked 304 ("redirect status").
    // A real 304 from api.github.com does reach fetch() in WebKit (checked by hand).
    test.skip(browserName === "webkit", "Playwright can't mock a 304 in WebKit");
    const gh = await mockGitHub(page, { files: sampleFiles() });
    await connectedDevice(page);
    await page.goto("./#notes");
    await expect(page.locator(".sync-line")).toContainText("Synced");
    gh.log.length = 0;
    await page.reload();
    await expect(page.locator(".sync-line")).toContainText("Synced");
    // The events listing is conditional. The taxonomy and judgments are
    // checked too (404s here, as this mock repo has neither).
    expect(gh.log.sort()).toEqual([
      "GET /repos/justsankalp/aarambh/contents/data/events (etag)",
      "GET /repos/justsankalp/aarambh/contents/data/judgments",
      "GET /repos/justsankalp/aarambh/contents/data/taxonomy.toml",
    ]);
  });
});

test("user text is rendered as plain text, never as HTML", async ({ page }) => {
  const evil = `<img src=x onerror="window.__xss=1"><script>window.__xss=2</script>`;
  await mockGitHub(page, {
    files: { [today]: dayFile([note({ ms: now - 1000, author: `<b onmouseover="window.__xss=3">Mallory</b>`, text: evil })]) },
  });
  await connectedDevice(page);
  await page.goto("./#notes");
  const card = page.locator("article.card");
  await expect(card.locator(".note-text")).toHaveText(evil);
  await expect(card).toContainText(`<b onmouseover="window.__xss=3">Mallory</b>`);
  await expect(card.locator("img, script, b")).toHaveCount(0);
  await card.hover();
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
});
