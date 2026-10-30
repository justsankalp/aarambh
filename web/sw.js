// aarambh service worker.
// - The page (index.html) is network-first so new versions ship at once, but
//   falls back to the cached copy after a short wait or when offline.
// - Content-hashed assets (name-<16 hex>.js/.wasm/.css) never change, so they
//   are served from cache. A new deploy references new names.
// - Cross-origin requests (api.github.com) are never intercepted.

const CACHE = "aarambh-shell-v1";
const PAGE_TIMEOUT_MS = 700;
const HASHED = /-[0-9a-f]{16}(_bg)?\.(js|wasm|css)$/;
const ASSET_REF = /["'](\.\/[^"']+\.(?:js|wasm|css))["']/g;
const scope = self.registration.scope; // e.g. https://sankalp.life/aarambh/

self.addEventListener("install", (event) => {
  event.waitUntil(refreshShell().catch(() => {}).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin || !req.url.startsWith(scope)) return;
  if (req.mode === "navigate") {
    event.respondWith(page(event));
  } else if (HASHED.test(url.pathname)) {
    event.respondWith(cacheFirst(req));
  } else {
    event.respondWith(networkFirst(req));
  }
});

async function page(event) {
  const network = fetch(scope, { cache: "no-cache" }).then((res) => {
    if (res.ok) event.waitUntil(storePage(res.clone()));
    return res;
  });
  const cached = await caches.match(scope);
  if (!cached) return network;
  const timeout = new Promise((resolve) => setTimeout(resolve, PAGE_TIMEOUT_MS));
  try {
    const res = await Promise.race([network, timeout]);
    if (res && res.ok) return res;
  } catch (e) {
    // Offline: use the cached page.
  }
  event.waitUntil(network.catch(() => {}));
  return cached;
}

async function cacheFirst(req) {
  const cached = await caches.match(req);
  if (cached) return cached;
  const res = await fetch(req);
  if (res.ok) {
    const cache = await caches.open(CACHE);
    await cache.put(req, res.clone());
  }
  return res;
}

async function networkFirst(req) {
  try {
    const res = await fetch(req, { cache: "no-cache" });
    if (res.ok) {
      const cache = await caches.open(CACHE);
      await cache.put(req, res.clone());
    }
    return res;
  } catch (e) {
    const cached = await caches.match(req);
    if (cached) return cached;
    throw e;
  }
}

async function refreshShell() {
  const res = await fetch(scope, { cache: "no-cache" });
  if (res.ok) await storePage(res);
}

// Caches the page and the hashed assets it references, and drops hashed
// assets from older deploys.
async function storePage(res) {
  const html = await res.text();
  const cache = await caches.open(CACHE);
  await cache.put(scope, new Response(html, { headers: { "Content-Type": "text/html; charset=utf-8" } }));
  const wanted = new Set([...html.matchAll(ASSET_REF)].map((m) => new URL(m[1], scope).href));
  for (const url of wanted) {
    if (!(await cache.match(url))) {
      try {
        await cache.add(url);
      } catch (e) {
        // Retried on the next page load.
      }
    }
  }
  for (const req of await cache.keys()) {
    if (HASHED.test(new URL(req.url).pathname) && !wanted.has(req.url)) await cache.delete(req);
  }
}
