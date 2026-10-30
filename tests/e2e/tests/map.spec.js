import { expect, test } from "@playwright/test";
import { connectedDevice, dayFile, mockGitHub, note, ulid, utcDay } from "./mock-github.js";

const now = Date.now();
const today = `data/events/${utcDay(now)}.json`;
const idea = note({ ms: now - 9e6, n: 1, author: "Asha", kind: "idea", text: "Clinics will pay for intake automation" });
const vets = note({ ms: now - 8e6, n: 2, author: "Sankalp", kind: "idea", text: "Sell scheduling to vets" });
const dropped = {
  v: 1, id: ulid(now - 7e6, 3), ts: new Date(now - 7e6).toISOString(), author: "Sankalp", type: "status",
  idea: vets.id, status: "dropped", lesson: { modes: ["no-paying-customer"], why: "No budget", would_change_mind: "", check_earlier: "" },
};
const dentists = note({ ms: now - 6e6, n: 4, author: "Asha", kind: "idea", text: "Sell intake to dentists" });
const q = note({ ms: now - 5e6, n: 5, author: "Sankalp", kind: "question", text: "Who signs the cheque?", links: [{ to: idea.id, rel: "challenges" }] });

const possibleLink = { id: ulid(now, 102), to: idea.id, rel: "extends", strength: "possible", why: "Same buyer type, different specialty" };
const possibleRisk = { id: ulid(now, 104), lesson_event: dropped.id, mode: "slow-to-validate", level: "possibly", why: "Long sales cycles" };
const judgment = (noteId, edges, risks) => JSON.stringify({ note_id: noteId, model: "m", ts: new Date(now).toISOString(), edges, risks });

function files() {
  return {
    [today]: dayFile([idea, vets, dropped, dentists, q]),
    [`data/judgments/${dentists.id}.json`]: judgment(dentists.id, [possibleLink], [possibleRisk]),
    "data/digests/state.json": JSON.stringify({
      v: 1, through: new Date(now).toISOString(), generated_at: new Date(now).toISOString(), model: "m",
      disagreements: [{ topic: "Who buys", positions: [{ who: "Asha", says: "Managers" }, { who: "Sankalp", says: "Owners" }], assumption: "Managers hold budgets", note_ids: [] }],
      assumptions: [{ text: "Clinics have budget", status: "challenged", note_ids: [] }],
      open_questions: [{ text: "Who signs the cheque?", note_id: q.id }],
    }),
  };
}

async function openMap(page, opts = {}) {
  const gh = await mockGitHub(page, { files: files(), ...opts });
  await connectedDevice(page);
  await page.goto("./#map");
  return gh;
}

/** The canvas image once the layout has stopped moving. */
async function settled(page) {
  const canvas = page.locator("canvas.graph");
  let last = "";
  await expect
    .poll(async () => {
      const now = await canvas.evaluate((c) => c.toDataURL());
      const same = now === last;
      last = now;
      return same;
    }, { intervals: [300], timeout: 15_000 })
    .toBe(true);
  return last;
}

function eventsIn(gh) {
  return JSON.parse(gh.files.get(`data/events/${utcDay(Date.now())}.json`).text);
}

test("draws the notebook as a graph with a text alternative", async ({ page }) => {
  await openMap(page);
  const canvas = page.getByRole("img", { name: /Map of 4 notes and \d+ links/ });
  await expect(canvas).toBeVisible();
  await expect(page.getByRole("list", { name: "Legend" })).toContainText("suggested");
  await settled(page);
});

test("tapping a note opens its card, and Open in Notes jumps to it in the feed", async ({ page }) => {
  // One note: after fitting, it sits in the middle of the canvas.
  await mockGitHub(page, { files: { [today]: dayFile([idea]) } });
  await connectedDevice(page);
  await page.goto("./#map");
  await settled(page);
  const box = await page.locator("canvas.graph").boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  const card = page.locator(".map-card");
  await expect(card).toContainText("Clinics will pay for intake automation");
  await expect(card.locator(".badge")).toHaveText("active");
  await card.getByRole("button", { name: "Open in Notes" }).click();
  await expect(page).toHaveURL(/#notes$/);
  await expect(page.locator(`#n-${idea.id}`)).toBeFocused();

  // Tapping empty space clears the card.
  await page.goto("./#map");
  await settled(page);
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.locator(".map-card")).toBeVisible();
  await page.mouse.click(box.x + 20, box.y + box.height - 20);
  await expect(page.locator(".map-card")).toHaveCount(0);
});

test("zooming changes the view and Fit brings it back", async ({ page, browserName }) => {
  test.skip(browserName === "webkit", "the mobile WebKit profile has no mouse wheel");
  await openMap(page);
  const fitted = await settled(page);
  const box = await page.locator("canvas.graph").boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -400);
  await expect.poll(() => page.locator("canvas.graph").evaluate((c) => c.toDataURL())).not.toBe(fitted);
  await page.getByRole("button", { name: "Fit" }).click();
  await expect.poll(() => page.locator("canvas.graph").evaluate((c) => c.toDataURL())).toBe(fitted);
});

test("Maybe: confirming or rejecting records a verdict and clears the item", async ({ page }) => {
  const gh = await openMap(page);
  const tray = page.locator(".maybe");
  const items = tray.locator(".maybe-item");
  await expect(items).toHaveCount(2);
  // (The quote marks come from <q>, so they aren't part of the text.)
  await expect(items.nth(0)).toContainText("Sell intake to dentists may repeat Slow to validate from Sell scheduling to vets");
  await expect(items.nth(1)).toContainText("Sell intake to dentists extends Clinics will pay for intake automation");

  await items.filter({ hasText: "extends" }).getByRole("button", { name: "Confirm" }).click();
  await expect(items).toHaveCount(1);
  await items.getByRole("button", { name: "Reject" }).click();
  await expect(tray).toHaveCount(0);

  await expect.poll(() => eventsIn(gh).filter((e) => e.type === "verdict").length).toBe(2);
  const verdicts = eventsIn(gh).filter((e) => e.type === "verdict");
  expect(verdicts.map((v) => [v.target, v.decision])).toEqual([[possibleLink.id, "confirm"], [possibleRisk.id, "reject"]]);
  expect(verdicts[0].author).toBe("Sankalp");

  // The confirmed link joins your own link on the map; the rejected risk never appears.
  await expect(page.getByRole("img", { name: /Map of 4 notes and 2 links/ })).toBeVisible();
});

test("list view: ideas by status with timelines, and the digest's running state", async ({ page }) => {
  await openMap(page);
  await page.getByRole("button", { name: "List" }).click();
  const list = page.locator(".map-list");
  await expect(list.locator("h4")).toHaveText(["Open", "Dropped"]);
  const vetsCard = list.locator("li.card", { hasText: "Sell scheduling to vets" });
  await expect(vetsCard.locator(".timeline li")).toHaveText([/Proposed by Sankalp/, /Marked dropped by Sankalp/]);
  await expect(list).toContainText("Who buys");
  await expect(list).toContainText("Underneath: Managers hold budgets");
  await expect(list.locator(".badge[data-status=challenged]")).toHaveText("challenged");

  // The choice of view is remembered.
  await page.reload();
  await expect(page.getByRole("button", { name: "List" })).toHaveAttribute("aria-pressed", "true");
  // An open question links back to its note.
  await list.getByRole("button", { name: "Who signs the cheque?" }).click();
  await expect(page).toHaveURL(/#notes$/);
  await expect(page.locator(`#n-${q.id}`)).toBeFocused();
});

test("an empty notebook shows a gentle placeholder", async ({ page }) => {
  await mockGitHub(page);
  await connectedDevice(page);
  await page.goto("./#map");
  await expect(page.getByText("The map fills in as you write notes and link them.")).toBeVisible();
});

test("pinch with two fingers zooms, one finger pans (real touch events)", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "multi-touch is sent through Chromium's DevTools protocol");
  await openMap(page);
  const fitted = await settled(page);
  const box = await page.locator("canvas.graph").boundingBox();
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  const cdp = await page.context().newCDPSession(page);
  const touch = (type, points) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points.map(([x, y], id) => ({ x, y, id })) });
  const image = () => page.locator("canvas.graph").evaluate((c) => c.toDataURL());

  await touch("touchStart", [[cx - 30, cy], [cx + 30, cy]]);
  for (let d = 40; d <= 120; d += 20) await touch("touchMove", [[cx - d, cy], [cx + d, cy]]);
  await touch("touchEnd", []);
  const pinched = await image();
  expect(pinched).not.toBe(fitted);

  await touch("touchStart", [[cx, cy]]);
  for (let d = 10; d <= 60; d += 10) await touch("touchMove", [[cx + d, cy + d]]);
  await touch("touchEnd", []);
  expect(await image()).not.toBe(pinched);
  await expect(page.locator(".map-card")).toHaveCount(0, { timeout: 500 });

  await page.getByRole("button", { name: "Fit" }).click();
  await expect.poll(image).toBe(fitted);
});
