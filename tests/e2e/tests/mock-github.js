// A small in-memory stand-in for the GitHub REST Contents API.
import { createHash } from "node:crypto";

export const REPO = "justsankalp/aarambh";
export const GOOD_TOKEN = "github_pat_good_token_for_tests";

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** A valid ULID for a given time; `n` varies the random part. */
export function ulid(ms, n = 0) {
  let t = "";
  for (let i = 0, v = ms; i < 10; i++, v = Math.floor(v / 32)) t = CROCKFORD[v % 32] + t;
  let r = "";
  for (let i = 0, v = n; i < 16; i++, v = Math.floor(v / 32)) r = CROCKFORD[v % 32] + r;
  return t + r;
}

export function note({ ms, n = 0, author = "Sankalp", kind = "idea", text, links = [] }) {
  return { v: 1, id: ulid(ms, n), ts: new Date(ms).toISOString(), author, type: "note", kind, text, links };
}

export function dayFile(events) {
  return "[\n  " + events.map((e) => JSON.stringify(e)).join(",\n  ") + "\n]\n";
}

export function utcDay(ms) {
  return new Date(ms).toISOString().slice(0, 10);
}

const sha1 = (s) => createHash("sha1").update(s).digest("hex");
const b64 = (s) => Buffer.from(s, "utf8").toString("base64").replace(/.{60}/g, "$&\n");

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-expose-headers": "ETag, X-RateLimit-Remaining",
};

/**
 * Routes api.github.com for a page. Options:
 *   files:     { "data/events/2026-10-30.json": "<text>" }
 *   conflicts: how many PUTs to answer 409 (another device "wins" each time)
 */
export async function mockGitHub(page, { files = {}, conflicts = 0 } = {}) {
  const isWebKit = page.context().browser()?.browserType().name() === "webkit";
  const state = {
    files: new Map(Object.entries(files).map(([p, text]) => [p, { text, sha: sha1(text) }])),
    conflicts,
    offline: false,
    puts: [],
    log: [],
  };

  await page.route("https://api.github.com/**", async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const method = req.method();
    if (state.offline) return route.abort("internetdisconnected");
    if (method === "OPTIONS") {
      return route.fulfill({
        status: 204,
        headers: { ...CORS, "access-control-allow-methods": "GET, PUT", "access-control-allow-headers": "*" },
      });
    }
    const headers = await req.allHeaders();
    state.log.push(`${method} ${url.pathname}${headers["if-none-match"] ? " (etag)" : ""}`);
    const json = (status, body, extra = {}) =>
      route.fulfill({ status, headers: { ...CORS, "content-type": "application/json", ...extra }, body: JSON.stringify(body) });

    if (headers.authorization !== `Bearer ${GOOD_TOKEN}`) return json(401, { message: "Bad credentials" });

    const base = `/repos/${REPO}`;
    if (url.pathname === base) return json(200, { full_name: REPO, private: false });
    if (!url.pathname.startsWith(`${base}/contents/`)) return json(404, { message: "Not Found" });
    const path = url.pathname.slice(`${base}/contents/`.length);

    if (method === "GET") {
      const file = state.files.get(path);
      if (file) return json(200, { type: "file", encoding: "base64", path, sha: file.sha, content: b64(file.text) }, { etag: `"${file.sha}"` });
      const entries = [...state.files.entries()]
        .filter(([p]) => p.startsWith(path + "/") && !p.slice(path.length + 1).includes("/"))
        .map(([p, f]) => ({ type: "file", name: p.split("/").pop(), path: p, sha: f.sha }));
      if (!entries.length) return json(404, { message: "Not Found" });
      const etag = `"${sha1(entries.map((e) => e.sha).join())}"`;
      // Playwright can't fulfill a 304 in WebKit; a 200 is what a browser
      // without the ETag would get, so the app must cope with it anyway.
      if (headers["if-none-match"] === etag && !isWebKit) return route.fulfill({ status: 304, headers: { ...CORS, etag } });
      return json(200, entries, { etag });
    }

    if (method === "PUT") {
      const body = JSON.parse(req.postData());
      const text = Buffer.from(body.content, "base64").toString("utf8");
      state.puts.push({ path, sha: body.sha, text, message: body.message });
      const current = state.files.get(path);
      if (state.conflicts > 0) {
        // Another device writes first: its event lands, our sha is now stale.
        state.conflicts--;
        const theirs = note({ ms: Date.now(), n: 999 + state.conflicts, author: "Co-founder", text: "Written on another device" });
        const before = current ? JSON.parse(current.text) : [];
        const newText = dayFile([...before, theirs]);
        state.files.set(path, { text: newText, sha: sha1(newText) });
        return json(409, { message: "is at a different sha" });
      }
      if (current && body.sha !== current.sha) return json(409, { message: "sha mismatch" });
      if (!current && body.sha) return json(422, { message: "sha given for a new file" });
      const sha = sha1(text);
      state.files.set(path, { text, sha });
      return json(current ? 200 : 201, { content: { path, sha }, commit: { sha: sha1(sha) } });
    }
    return json(405, { message: "Method not allowed" });
  });
  return state;
}

/** Pre-fills settings as if this device was set up before. */
export async function connectedDevice(page, name = "Sankalp") {
  await page.addInitScript(
    ([repo, token, who]) => {
      localStorage.setItem("aarambh.repo", repo);
      localStorage.setItem("aarambh.token", token);
      localStorage.setItem("aarambh.name", who);
    },
    [REPO, GOOD_TOKEN, name],
  );
}
