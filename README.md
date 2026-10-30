# aarambh

A shared notebook for two co-founders: jot down ideas, beliefs, questions,
evidence and disagreements, link them to each other, and get a short daily
digest of how your thinking has moved.

Live at **https://sankalp.life/aarambh/**. It works on phones and laptops,
and offline: notes written offline are sent when you're back online.

## What's in it

- **Notes.** Tap **+** to write. Pick a kind (idea, belief, question,
  evidence, disagreement, note). **Respond** on any note links your reply to
  it (extends, supports, challenges, answers, related); tap the reference to
  jump back. Ideas have a **Status**: active, parked, dropped or decided.
  Dropping or deciding one offers a short, skippable post-mortem. The dots
  at the top are the last seven days: amber when you both wrote, a ring when
  one of you did.
- **Map.** Your notes as a graph: ideas are the larger dots, your links are
  solid lines, links the judge suggests are dashed, and risk flags (a note
  that looks like a past post-mortem) are dashed in clay. Pinch or scroll to
  zoom, drag to move, tap a note to open it. **List** shows ideas by status
  with their timelines, plus where you diverge, your assumptions and open
  questions. **Maybe** holds suggestions the judge wasn't sure about:
  confirm or reject them.
- **Digest.** A short summary each morning (08:00 IST) when something is
  new: today in brief, what moved, where you two diverge and the assumption
  underneath, patterns, untested assumptions, and next moves. **Summarise
  now** asks for one straight away.

## Where your notes go

- **This repository is public, and so are the notes.** Each note is a line of
  JSON in `data/events/YYYY-MM-DD.json`, committed by the app with your token.
- **Note text is sent to Google's Gemini API** by two GitHub Actions jobs:
  - *Judge* (on every push of new notes): sends the new note, up to 8 related
    earlier notes and recent post-mortem lessons, to suggest links and flag
    risks. It never sends the whole notebook.
  - *Digest* (nightly, or "Summarise now"): sends what's new since the last
    digest plus the previous digest's running state.
- These jobs use Gemini's **free tier**, where Google may use what's sent to
  improve its products. Use a paid Gemini key if you don't want that.
- Your GitHub token stays in the browser (localStorage) and is only ever sent
  to `api.github.com`. The Gemini key is an Actions secret and never reaches
  the browser.

## Setup

1. **The repository.** Create a **public** repository and push this code to
   its `main` branch. (Pages from a private repo needs a paid plan.)
2. **Pages.** Repository Settings → Pages → Source: *GitHub Actions*. The
   *Deploy* workflow publishes the app on every push that changes more than
   `data/`. A custom domain works too: this one is served under
   `sankalp.life` because that is the owner's user-site domain.
3. **Gemini key.** Create a key at <https://aistudio.google.com/apikey> (a
   Google account is enough; no billing needed). Add it under Settings →
   Secrets and variables → Actions → *New repository secret*, named
   `GEMINI_API_KEY`. Until it's set, the judge and digest jobs skip with a
   warning.
4. **Models (optional).** The defaults are `gemini-3.5-flash-lite` for judging
   and `gemini-3.8-flash` for digests. To change them, add repository
   *variables* (not secrets) `JUDGE_MODEL` and/or `DIGEST_MODEL`.
5. **Outside contributors.** Settings → Actions → General → *Fork pull request
   workflows from outside collaborators* → **Require approval for all outside
   collaborators**. The workflows here only run on pushes to `main`, on the
   schedule and on demand, never for pull requests, but this keeps it so.
6. **A token on each device.** GitHub → Settings → Developer settings →
   Fine-grained tokens → *Generate new token*:
   - Expiration: up to a year (put a reminder in your calendar).
   - Repository access: *Only select repositories* → this repository.
   - Permissions: **Contents: Read and write**, **Actions: Read and write**.
7. **Open the app** and go to Settings: paste the token, enter your name and
   tap *Save and connect*.
   - **iPhone:** open the link in Safari → Share → *Add to Home Screen*.
   - **Android:** Chrome menu → *Add to Home screen* / *Install app*.
   - **Laptop:** Chrome or Edge offer *Install* in the address bar; any
     browser works in a tab.

## Troubleshooting

- **"GitHub rejected the token."** It has expired or been revoked. Make a new
  one (step 6) and paste it in Settings.
- **The app forgot my settings.** iOS can clear a site's storage. Your notes
  are safe in the repository: open Settings, tap *Paste* for the token and
  pick your name. Anything still waiting to send stays queued on the device
  and goes out once you're connected.
- **No suggested links on the Map.** Check the repository's *Actions* tab: the
  *Judge* run for your push should be green. A yellow warning about
  `GEMINI_API_KEY` means the secret is missing. A "rate limited" warning means
  the free tier's limit was hit; the next push picks up where it stopped.
- **"Summarise now" did nothing.** The digest skips when nothing is new since
  the last one. If the app says the token can't start workflows, the token
  needs *Actions: Read and write*.
- **Notes from a newer version.** If the app says some entries were written
  by a newer version, reload it to get the latest app.

## The data

All plain files under `data/`, readable on GitHub:

| Path | Written by | What |
|---|---|---|
| `events/YYYY-MM-DD.json` | the app | notes, status changes and verdicts, one per line, append-only (UTC days) |
| `judgments/<note-id>.json` | the judge | suggested links and risk flags for one note |
| `digests/YYYY-MM-DD.md` | the digest | that day's summary |
| `digests/state.json` | the digest | running lists: ideas, divergences, assumptions, open questions, patterns |
| `taxonomy.toml` | you | the failure modes used by post-mortems and risk flags |

To change the failure modes, edit `data/taxonomy.toml` on GitHub: each
`[[mode]]` has an `id`, a `label` and an optional `hint`. Only rename an `id`
that no post-mortem has used yet.

## Development

Needs Rust (the version is pinned in `rust-toolchain.toml`), Trunk 0.22.1 and,
for browser tests, Node.

```sh
cargo test --workspace                    # core logic and the judge (mock Gemini)
trunk serve                               # the app at http://127.0.0.1:8080/
cd tests/e2e && npm install && npm test   # Playwright: Chromium, then WebKit
npm run perf                              # load timings on a throttled phone profile
```

WebKit tests run in Playwright's container image (`npm run test:webkit`,
needs podman) because its WebKit build needs Ubuntu libraries.

Layout: `crates/core` (types, merging, search, graph and layout, digest state;
native and wasm), `crates/web` (the Leptos app), `crates/judge` (the CLI the
workflows run), `web/` (HTML shell, styles, service worker, icons), `data/`
(notes, judgments, digests, `taxonomy.toml`).
