# aarambh

A shared notebook for two co-founders: jot down ideas, beliefs, questions,
evidence and disagreements, link them to each other, and get a short daily
digest of how your thinking has moved.

Live at **https://sankalp.life/aarambh/**. It works on phones (on iPhone,
open it in Safari and use Share → Add to Home Screen) and laptops, and it
works offline: notes you write offline are sent when you're back online.

## Where your notes go

- **This repository is public, and so are the notes.** Each note is a line of
  JSON in `data/events/YYYY-MM-DD.json`, committed by the app with your token.
- **Note text is sent to Google's Gemini API** by two GitHub Actions jobs:
  - *Judge* (on every push of new notes): sends the new note, up to 8 related
    earlier notes and recent post-mortem lessons, to suggest links and flag
    risks. It never sends the whole notebook.
  - *Digest* (nightly at 08:00 IST, or "Summarise now"): sends what's new
    since the last digest plus the previous digest's running state.
- These jobs use Gemini's **free tier**, where Google may use what's sent to
  improve its products. Use a paid Gemini key if you don't want that.
- Your GitHub token stays in the browser (localStorage) and is only ever sent
  to `api.github.com`. The Gemini key is an Actions secret and never reaches
  the browser.

## Setup

1. **Pages.** Repository Settings → Pages → Source: *GitHub Actions*. The
   deploy workflow publishes the app on every push that changes more than
   `data/`.
2. **Gemini key.** Create a key at <https://aistudio.google.com/apikey> (a
   Google account is enough; no billing needed). Add it under Settings →
   Secrets and variables → Actions → *New repository secret*, named
   `GEMINI_API_KEY`. Until it's set, the judge and digest jobs skip with a
   warning.
3. **Models (optional).** The defaults are `gemini-3.5-flash-lite` for judging
   and `gemini-3.8-flash` for digests. To change them, add repository
   *variables* `JUDGE_MODEL` and/or `DIGEST_MODEL`.
4. **Outside contributors.** Settings → Actions → General → *Require approval
   for all outside collaborators*. Workflows here only run on pushes to
   `main`, on the schedule and on demand, never for pull requests.
5. **A token on each device.** GitHub → Settings → Developer settings →
   Fine-grained tokens → *Generate new token*:
   - Repository access: *Only select repositories* → this repository.
   - Permissions: **Contents: Read and write**, **Actions: Read and write**.

   In the app, open Settings, paste the token, enter your name and tap
   *Save and connect*. If your phone clears the app's storage, do the same
   again; your notes are safe in the repository.

## Development

Needs Rust (the version is pinned in `rust-toolchain.toml`), Trunk 0.22.1 and,
for browser tests, Node.

```sh
cargo test --workspace                 # core logic and the judge (mock Gemini)
trunk serve                            # the app at http://127.0.0.1:8080/
cd tests/e2e && npm install && npm test   # Playwright: Chromium, then WebKit
```

WebKit tests run in Playwright's container image (`npm run test:webkit`,
needs podman) because its WebKit build needs Ubuntu libraries.

Layout: `crates/core` (types, merging, search, digest state; native and
wasm), `crates/web` (the Leptos app), `crates/judge` (the CLI the workflows
run), `web/` (HTML shell, styles, service worker, icons), `data/` (notes,
judgments, digests, `taxonomy.toml`).
