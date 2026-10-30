import { expect, test } from "@playwright/test";
import { connectedDevice, dayFile, mockGitHub, note, utcDay } from "./mock-github.js";

const now = Date.now();
const todayPath = () => `data/events/${utcDay(Date.now())}.json`;

const IDEA = note({ ms: now - 120_000, n: 1, author: "Asha", kind: "idea", text: "Clinics will pay for intake automation" });
const QUESTION = note({ ms: now - 60_000, n: 2, author: "Sankalp", kind: "question", text: "Who signs the cheque at a clinic?" });
const BELIEF = note({ ms: now - 30_000, n: 3, author: "Asha", kind: "belief", text: "Front desks hate their software" });

function files(events = [IDEA, QUESTION, BELIEF]) {
  return { [`data/events/${utcDay(now)}.json`]: dayFile(events) };
}

async function open(page, opts = {}) {
  const gh = await mockGitHub(page, { files: files(), ...opts });
  await connectedDevice(page);
  await page.goto("./#notes");
  await expect(page.locator(".sync-line")).toContainText("Synced");
  return gh;
}

const composer = (page) => page.getByRole("dialog", { name: /New note|Respond/ });
const card = (page, text) => page.locator("article.card", { hasText: text });

function eventsIn(gh, path = todayPath()) {
  const f = gh.files.get(path);
  return f ? JSON.parse(f.text) : [];
}

test("+ opens the composer with the cursor in the text box, and posting shows at once", async ({ page }) => {
  const gh = await open(page);
  await page.getByRole("button", { name: "New note" }).click();
  // One tap and you can type: the text box already has focus.
  await expect(composer(page).getByRole("textbox", { name: "Note" })).toBeFocused();
  await page.keyboard.type("Ask three clinics about intake this week");
  await composer(page).getByRole("button", { name: "evidence" }).click();
  await composer(page).getByRole("button", { name: "Post" }).click();

  await expect(composer(page)).toBeHidden();
  const posted = card(page, "Ask three clinics about intake this week");
  await expect(posted).toBeVisible();
  await expect(posted.locator(".kind")).toHaveText("evidence");
  await expect(page.locator(".sync-line")).not.toContainText("waiting");

  const ours = eventsIn(gh).find((e) => e.text === "Ask three clinics about intake this week");
  expect(ours).toMatchObject({ v: 1, type: "note", kind: "evidence", author: "Sankalp", links: [] });
  expect(ours.id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
  expect(gh.puts.at(-1).message).toBe(`notes: 1 event from Sankalp (${utcDay(Date.now())})`);
});

test("a write that loses a race re-reads, retries and keeps everyone's events", async ({ page }) => {
  const gh = await open(page, { conflicts: 2 });
  const before = gh.files.get(todayPath()).text;
  await page.getByRole("button", { name: "New note" }).click();
  await page.keyboard.type("Posted during a race");
  await composer(page).getByRole("button", { name: "Post" }).click();
  await expect(card(page, "Posted during a race")).toBeVisible();

  await expect.poll(() => gh.puts.length, { timeout: 10_000 }).toBe(3);
  await expect(page.locator(".sync-line")).toContainText("Synced");
  const texts = eventsIn(gh).map((e) => e.text);
  expect(texts.filter((t) => t === "Posted during a race")).toHaveLength(1);
  expect(texts.filter((t) => t === "Written on another device")).toHaveLength(2);
  // Existing lines are untouched, byte for byte.
  const finalText = gh.files.get(todayPath()).text;
  for (const line of before.split("\n").slice(1, -2)) expect(finalText).toContain(line);
  // The other device's events show up here too.
  await expect(card(page, "Written on another device")).toHaveCount(2);
});

test("posts made offline wait in a queue, survive a reload, and send when back online", async ({ page }) => {
  const gh = await open(page);
  gh.offline = true;
  await page.getByRole("button", { name: "New note" }).click();
  await page.keyboard.type("Written on the train");
  await composer(page).getByRole("button", { name: "Post" }).click();

  const posted = card(page, "Written on the train");
  await expect(posted).toBeVisible();
  await expect(posted.locator(".pending")).toHaveText("sending…");
  await expect(page.locator(".sync-line")).toContainText("1 waiting to send");

  await page.reload();
  await expect(card(page, "Written on the train")).toBeVisible();
  await expect(page.locator(".sync-line")).toContainText("1 waiting to send");
  expect(eventsIn(gh).some((e) => e.text === "Written on the train")).toBe(false);

  gh.offline = false;
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect(page.locator(".sync-line")).not.toContainText("waiting", { timeout: 10_000 });
  await expect(card(page, "Written on the train").locator(".pending")).toHaveCount(0);
  expect(eventsIn(gh).filter((e) => e.text === "Written on the train")).toHaveLength(1);
});

test("respond creates a typed link, and tapping the reference jumps to the original", async ({ page }) => {
  const gh = await open(page);
  await card(page, "Who signs the cheque").getByRole("button", { name: "Respond" }).click();
  const sheet = composer(page);
  await expect(sheet).toContainText("Responding to Sankalp");
  await expect(sheet).toContainText("Who signs the cheque at a clinic?");
  // A question defaults to "answers".
  await expect(sheet.getByRole("group", { name: "How it relates" }).getByRole("button", { name: "answers" })).toHaveAttribute("aria-pressed", "true");
  await sheet.getByRole("button", { name: "challenges" }).click();
  await page.keyboard.type("Usually the owner, not the manager");
  await sheet.getByRole("button", { name: "Post" }).click();

  const reply = card(page, "Usually the owner");
  await expect(reply.locator(".ref")).toContainText("challenges");
  await expect(reply.locator(".ref")).toContainText("Sankalp: “Who signs the cheque at a clinic?”");
  // Shown at once; written to the repo a moment later.
  await expect
    .poll(() => eventsIn(gh).find((e) => e.text === "Usually the owner, not the manager")?.links)
    .toEqual([{ to: QUESTION.id, rel: "challenges" }]);

  // Filter the original out of view; jumping brings it back and focuses it.
  await page.getByRole("group", { name: "Person" }).getByRole("button", { name: "Asha" }).click();
  await expect(card(page, "Who signs the cheque")).toHaveCount(0);
  await page.getByRole("group", { name: "Person" }).getByRole("button", { name: "Sankalp" }).click();
  await card(page, "Usually the owner").locator(".ref").click();
  const original = page.locator(`#n-${QUESTION.id}`);
  await expect(original).toBeFocused();
  await expect(original).toBeInViewport();
  await expect(original).toHaveClass(/flash/);
});

test("the draft is kept when the sheet is closed or the app reloads", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "New note" }).click();
  await page.keyboard.type("Half a thought");
  await composer(page).getByRole("button", { name: "question" }).click();
  await composer(page).getByRole("button", { name: "Close" }).click();
  await expect(composer(page)).toBeHidden();

  await page.reload();
  await page.getByRole("button", { name: "New note" }).click();
  await expect(composer(page).getByRole("textbox", { name: "Note" })).toHaveValue("Half a thought");
  await expect(composer(page).getByRole("button", { name: "question" })).toHaveAttribute("aria-pressed", "true");
  // Posting clears it.
  await composer(page).getByRole("button", { name: "Post" }).click();
  expect(await page.evaluate(() => localStorage.getItem("aarambh.draft"))).toBeNull();
});

test("filters by person and by kind", async ({ page }) => {
  await open(page);
  const texts = () => page.locator(".note-text").allTextContents();
  expect(await texts()).toEqual([BELIEF.text, QUESTION.text, IDEA.text]);

  const people = page.getByRole("group", { name: "Person" });
  const kinds = page.getByRole("group", { name: "Kind of note" });
  await people.getByRole("button", { name: "Asha" }).click();
  await expect.poll(texts).toEqual([BELIEF.text, IDEA.text]);
  await kinds.getByRole("button", { name: "idea" }).click();
  await expect.poll(texts).toEqual([IDEA.text]);
  await kinds.getByRole("button", { name: "question" }).click();
  await expect(page.getByText("No notes match these filters.")).toBeVisible();
  await page.getByRole("button", { name: "Show everything" }).click();
  await expect.poll(texts).toEqual([BELIEF.text, QUESTION.text, IDEA.text]);
});

test("dropping an idea offers a skippable post-mortem with the repo's failure modes", async ({ page }) => {
  const taxonomy = `[[mode]]\nid = "no-paying-customer"\nlabel = "No paying customer"\n\n[[mode]]\nid = "slow-to-validate"\nlabel = "Slow to validate"\n\n[[mode]]\nid = "our-own-mode"\nlabel = "Our own mode"\n`;
  const gh = await open(page, { files: { ...files(), "data/taxonomy.toml": taxonomy } });
  const idea = card(page, "Clinics will pay");
  await expect(idea.locator(".badge")).toHaveText("active");

  // Parking needs no post-mortem.
  await idea.getByRole("button", { name: "Status" }).click();
  const sheet = page.getByRole("dialog", { name: "Idea status" });
  await sheet.getByRole("button", { name: /parked/i }).click();
  await expect(sheet).toBeHidden();
  await expect(idea.locator(".badge")).toHaveText("parked");

  // Dropping offers one.
  await idea.getByRole("button", { name: "Status" }).click();
  await sheet.getByRole("button", { name: /dropped/i }).click();
  await expect(sheet).toContainText("Quick post-mortem");
  const modes = sheet.getByRole("group", { name: "Failure modes" });
  await expect(modes.getByRole("button")).toHaveText(["No paying customer", "Slow to validate", "Our own mode"]);
  await modes.getByRole("button", { name: "No paying customer" }).click();
  await modes.getByRole("button", { name: "Our own mode" }).click();
  await sheet.getByLabel("Why?").fill("Nobody would sign");
  await sheet.getByLabel("What would change your mind?").fill("A signed letter of intent");
  await sheet.getByRole("button", { name: "Save" }).click();

  await expect(idea.locator(".badge")).toHaveText("dropped");
  const row = page.locator(".status-row").first();
  await expect(row).toContainText("Nobody would sign");
  await expect(row).toContainText("No paying customer, Our own mode");
  await expect(page.locator(".sync-line")).not.toContainText("waiting");

  const statuses = eventsIn(gh).filter((e) => e.type === "status");
  expect(statuses.map((s) => s.status)).toEqual(["parked", "dropped"]);
  expect(statuses[0].lesson).toBeUndefined();
  expect(statuses[1]).toMatchObject({
    idea: IDEA.id,
    lesson: { modes: ["no-paying-customer", "our-own-mode"], why: "Nobody would sign", would_change_mind: "A signed letter of intent", check_earlier: "" },
  });

  // Skip records the status without a lesson.
  await idea.getByRole("button", { name: "Status" }).click();
  await sheet.getByRole("button", { name: /decided/i }).click();
  await sheet.getByRole("button", { name: "Skip" }).click();
  await expect(idea.locator(".badge")).toHaveText("decided");
  await expect.poll(() => eventsIn(gh).filter((e) => e.type === "status").at(-1)?.status).toBe("decided");
  expect(eventsIn(gh).filter((e) => e.type === "status").at(-1).lesson).toBeUndefined();
});

test("text typed into the composer is posted and shown as plain text", async ({ page }) => {
  const gh = await open(page);
  const evil = `<img src=x onerror="window.__xss=1"> **not markdown**`;
  await page.getByRole("button", { name: "New note" }).click();
  await page.keyboard.type(evil);
  await composer(page).getByRole("button", { name: "Post" }).click();
  await expect(card(page, "not markdown").locator(".note-text")).toHaveText(evil);
  await expect(page.locator("article img")).toHaveCount(0);
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
  await expect.poll(() => eventsIn(gh).some((e) => e.text === evil)).toBe(true);
});
