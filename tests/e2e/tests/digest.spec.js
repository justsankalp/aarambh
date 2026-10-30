import { expect, test } from "@playwright/test";
import { connectedDevice, dayFile, mockGitHub, note, utcDay } from "./mock-github.js";

const now = Date.now();
const day = (offset) => utcDay(now + offset * 86_400_000);
const n1 = note({ ms: now - 3 * 3600_000, n: 1, author: "Asha", kind: "idea", text: "Clinics will pay for intake automation" });
const n2 = note({ ms: now - 2 * 3600_000, n: 2, author: "Sankalp", kind: "question", text: "Who signs the cheque?" });
const n3 = note({ ms: now - 1 * 3600_000, n: 3, author: "Asha", kind: "evidence", text: "Two owners said they approve software" });

const latestMd = `# Today

## Summary · 02:30 UTC

### Today in brief

Asha proposed **clinic intake**; Sankalp asked who pays.

### Where you two diverge

**Who buys**

- **Asha:** Clinic managers
- **Sankalp:** Clinic owners
- *Underneath:* Managers control software budgets

### Suggested next moves

1. Ask three owners who approves software spend.
`;
const olderMd = "# Yesterday\n\n## Summary · 02:30 UTC\n\n### Today in brief\n\nAn older, quieter day.\n";

function state(throughMs, generatedMs) {
  return JSON.stringify({ v: 1, through: new Date(throughMs).toISOString(), generated_at: new Date(generatedMs).toISOString(), model: "gemini-3.8-flash" });
}

function files(extra = {}) {
  return {
    [`data/events/${utcDay(now)}.json`]: dayFile([n1, n2, n3]),
    [`data/digests/${day(0)}.md`]: latestMd,
    [`data/digests/${day(-1)}.md`]: olderMd,
    "data/digests/state.json": state(Date.parse(n1.ts), now - 1800_000),
    ...extra,
  };
}

async function openDigest(page, opts = {}) {
  const gh = await mockGitHub(page, { files: files(), ...opts });
  await connectedDevice(page);
  await page.goto("./#digest");
  return gh;
}

test("shows the newest digest rendered as structured text, and older days on request", async ({ page }) => {
  await openDigest(page);
  const digest = page.locator("article.digest");
  await expect(digest.getByRole("heading", { name: "Today in brief" })).toBeVisible();
  await expect(digest.locator("strong", { hasText: "clinic intake" })).toBeVisible();
  await expect(digest.locator("li", { hasText: "Asha: Clinic managers" })).toBeVisible();
  await expect(digest.locator("ol li")).toHaveText(["Ask three owners who approves software spend."]);
  await expect(page.getByRole("combobox", { name: "Digest date" })).toHaveValue(day(0));
  await expect(page.getByRole("button", { name: "Newer digest" })).toBeDisabled();

  await page.getByRole("button", { name: "Older digest" }).click();
  await expect(digest).toContainText("An older, quieter day.");
  await expect(page.getByRole("combobox", { name: "Digest date" })).toHaveValue(day(-1));
  await page.getByRole("combobox", { name: "Digest date" }).selectOption(day(0));
  await expect(digest).toContainText("Who buys");
  await expect(page.locator(".digest-foot")).toContainText("gemini-3.8-flash");
});

test("counts notes written since the last summary", async ({ page }) => {
  await openDigest(page);
  await expect(page.locator(".digest-bar")).toContainText("2 new notes since this summary.");
});

test("HTML and links in a digest are shown as text, never run", async ({ page }) => {
  const evil = `# Day\n\nHi <img src=x onerror="window.__xss=1"> and [click](javascript:window.__xss=2) and [ok](https://example.org)\n\n<script>window.__xss=3</script>\n`;
  await openDigest(page, { files: files({ [`data/digests/${day(0)}.md`]: evil }) });
  const digest = page.locator("article.digest");
  await expect(digest).toContainText(`<img src=x onerror="window.__xss=1">`);
  await expect(digest).toContainText("<script>window.__xss=3</script>");
  await expect(digest).toContainText("[click](javascript:window.__xss=2)");
  await expect(digest.locator("img, script, a")).toHaveCount(0);
  await digest.getByText("click").click();
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
});

test("Summarise now starts the workflow and shows the new digest when it lands", async ({ page }) => {
  await page.clock.install();
  const gh = await openDigest(page, {
    // The workflow commits a new summary and state.
    onDispatch: (repo) => {
      repo.setFile(`data/digests/${day(0)}.md`, latestMd + "\n---\n\n## Summary · 15:00 UTC\n\n### Today in brief\n\nFresh summary.\n");
      repo.setFile("data/digests/state.json", state(Date.parse(n3.ts), now));
    },
  });
  const button = page.getByRole("button", { name: "Summarise now" });
  await expect(page.locator(".digest-bar")).toContainText("2 new notes");
  await button.click();
  await expect(page.locator(".notice")).toContainText("Summarising");
  await expect(button).toBeDisabled();
  expect(gh.dispatches).toEqual([{ workflow: "digest.yml", body: { ref: "main" } }]);

  await page.clock.fastForward(16_000);
  await expect(page.locator(".notice")).toHaveText("New summary ready.");
  await expect(page.locator("article.digest")).toContainText("Fresh summary.");
  await expect(page.locator(".digest-bar")).toContainText("Up to date");
  await expect(button).toBeDisabled();
});

test("a token without Actions permission gets a clear message", async ({ page }) => {
  await openDigest(page, { actionsForbidden: true });
  await page.getByRole("button", { name: "Summarise now" }).click();
  await expect(page.getByRole("alert")).toHaveText("The token can't start the digest workflow. It needs Actions: Read and write.");
});

test("before the first digest", async ({ page }) => {
  await mockGitHub(page, { files: { [`data/events/${utcDay(now)}.json`]: dayFile([n1]) } });
  await connectedDevice(page);
  await page.goto("./#digest");
  await expect(page.getByText("No digest yet.")).toBeVisible();
  await expect(page.locator(".digest-bar")).toContainText("1 note, not summarised yet.");
  await expect(page.getByRole("button", { name: "Summarise now" })).toBeEnabled();
});
