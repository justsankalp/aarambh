// WCAG AA contrast for all visible text, measured on the rendered screens in
// both themes: 4.5:1 for body text, 3:1 for large text (24px, or 18.66px
// bold). Backgrounds are composited through translucent layers. Disabled
// controls are exempt, as WCAG allows.
import { expect, test } from "@playwright/test";
import { connectedDevice, dayFile, mockGitHub, note, ulid, utcDay } from "./mock-github.js";

const now = Date.now();
const idea = note({ ms: now - 9e6, n: 1, author: "Asha", kind: "idea", text: "Clinics will pay for intake automation" });
const decided = note({ ms: now - 8e6, n: 2, author: "Sankalp", kind: "idea", text: "Start with dental clinics" });
const parkedIdea = note({ ms: now - 7e6, n: 3, author: "Asha", kind: "idea", text: "Sell scheduling to vets" });
const status = (n, ideaId, s, lesson) => ({ v: 1, id: ulid(now - 6e6 + n, 50 + n), ts: new Date(now - 6e6 + n).toISOString(), author: "Sankalp", type: "status", idea: ideaId, status: s, ...(lesson ? { lesson } : {}) });
const q = note({ ms: now - 5e6, n: 5, author: "Sankalp", kind: "question", text: "Who signs the cheque?", links: [{ to: idea.id, rel: "challenges" }] });
const dentists = note({ ms: now - 4e6, n: 6, author: "Asha", kind: "evidence", text: "Two owners said they approve software" });
const events = [idea, decided, parkedIdea, status(1, decided.id, "decided"), status(2, parkedIdea.id, "dropped", { modes: ["no-paying-customer"], why: "No budget", would_change_mind: "A paid pilot", check_earlier: "" }), q, dentists];
const judgment = JSON.stringify({ note_id: dentists.id, model: "m", ts: new Date(now).toISOString(), edges: [{ id: ulid(now, 9), to: idea.id, rel: "supports", strength: "possible", why: "Owners approving supports willingness to pay" }], risks: [] });
const md = "# Friday\n\n## Summary · 02:30 UTC\n\n### Today in brief\n\nAsha proposed **intake**; *Sankalp* asked who pays.\n\n### What moved\n\n- One\n\n### Suggested next moves\n\n1. Ask owners.\n";
const files = {
  [`data/events/${utcDay(now)}.json`]: dayFile(events),
  [`data/judgments/${dentists.id}.json`]: judgment,
  [`data/digests/${utcDay(now)}.md`]: md,
  "data/digests/state.json": JSON.stringify({ v: 1, through: new Date(now - 4.5e6).toISOString(), generated_at: new Date(now).toISOString(), model: "m",
    disagreements: [{ topic: "Who buys", positions: [{ who: "Asha", says: "Managers" }, { who: "Sankalp", says: "Owners" }], assumption: "Budgets", note_ids: [] }],
    assumptions: [{ text: "A", status: "untested", note_ids: [] }, { text: "B", status: "supported", note_ids: [] }, { text: "C", status: "challenged", note_ids: [] }],
    open_questions: [{ text: "Who signs?", note_id: null }] }),
};

/** Runs in the page: every visible text element below the AA threshold. */
function audit() {
  const parse = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
    return [p[0], p[1], p[2], p[3] ?? 1];
  };
  const over = (top, under) => top.slice(0, 3).map((v, i) => v * top[3] + under[i] * (1 - top[3])).concat(1);
  const lum = ([r, g, b]) => {
    const f = (v) => ((v /= 255) <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
  };
  const background = (el) => {
    const layers = [];
    for (let e = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c && c[3] > 0) {
        layers.push(c);
        if (c[3] >= 1) break;
      }
    }
    return layers.reverse().reduce((acc, l) => over(l, acc), [255, 255, 255, 1]);
  };
  const visible = (el) => {
    if (el.closest("[aria-hidden=true], .sheet:not(.open), .backdrop:not(.open)")) return false;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
  };
  const failures = [];
  const check = (el, text, colour, bg, size, weight) => {
    const large = size >= 24 || (size >= 18.66 && weight >= 700);
    const need = large ? 3 : 4.5;
    const r = ratio(colour, bg);
    if (r < need) failures.push(`${r.toFixed(2)} < ${need}: "${text.slice(0, 40)}" (${el.tagName.toLowerCase()}.${el.className})`);
  };
  for (const el of document.querySelectorAll("body *")) {
    if (!visible(el) || el.closest(":disabled")) continue;
    const s = getComputedStyle(el);
    const own = [...el.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim()).map((n) => n.textContent.trim()).join(" ");
    const bg = background(el);
    if (own) {
      const c = parse(s.color);
      check(el, own, over(c, bg), bg, parseFloat(s.fontSize), Number(s.fontWeight));
    }
    if ((el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el.placeholder && !el.value) {
      const p = parse(getComputedStyle(el, "::placeholder").color);
      if (p) check(el, `placeholder: ${el.placeholder}`, over(p, bg), bg, parseFloat(s.fontSize), Number(s.fontWeight));
    }
  }
  return failures;
}

for (const scheme of ["light", "dark"]) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });

    test("connected screens", async ({ page }) => {
      const gh = await mockGitHub(page, { files });
      await connectedDevice(page);
      const failures = [];
      const record = async (name) => {
        await page.waitForTimeout(250); // let 150ms transitions finish
        for (const f of await page.evaluate(audit)) failures.push(`${name}: ${f}`);
      };

      await page.goto("./#notes");
      await expect(page.getByText("Who signs the cheque?").first()).toBeVisible();
      gh.offline = true; // leaves a post pending, so "sending…" is checked
      await page.getByRole("button", { name: "New note" }).click();
      await page.keyboard.type("Pending note");
      await page.getByRole("button", { name: "Post" }).click();
      await page.getByRole("group", { name: "Kind of note" }).getByRole("button", { name: "idea" }).click();
      await page.getByRole("group", { name: "Kind of note" }).getByRole("button", { name: "All kinds" }).click();
      await record("feed");

      await page.locator(`#n-${q.id}`).getByRole("button", { name: "Respond" }).click();
      await record("compose");
      await page.getByRole("dialog", { name: "Respond" }).getByRole("button", { name: "Close" }).click();

      await page.locator(`#n-${idea.id}`).getByRole("button", { name: "Status" }).click();
      await page.getByRole("button", { name: /dropped/i }).click();
      await record("post-mortem");
      await page.getByRole("dialog", { name: "Idea status" }).getByRole("button", { name: "Close" }).click();

      gh.offline = false;
      await page.goto("./#digest");
      await expect(page.getByRole("heading", { name: "Today in brief" })).toBeVisible();
      await record("digest");

      await page.goto("./#map");
      await page.getByRole("button", { name: "List" }).click();
      await expect(page.locator(".maybe-item")).toHaveCount(1);
      await record("map list");

      await page.goto("./#settings");
      await record("settings");

      expect(failures).toEqual([]);
    });

    test("first run and errors", async ({ page }) => {
      await mockGitHub(page);
      await page.goto("./");
      await page.getByLabel("GitHub token").fill("ghp_classic_token");
      await page.getByRole("button", { name: "Save and connect" }).click();
      await expect(page.getByRole("alert")).toBeVisible();
      await page.getByLabel("GitHub token").fill("");
      await page.waitForTimeout(250);
      const failures = await page.evaluate(audit);
      expect(failures).toEqual([]);
    });
  });
}
