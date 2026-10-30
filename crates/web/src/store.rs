//! Local-first sync. The app renders from the IndexedDB cache straight away,
//! then refreshes from the repo in the background. Posts are optimistic:
//! they go into the log and the offline queue at once, and the queue is
//! flushed to the repo whenever we're online.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use aarambh_core::digest::{DIGESTS_DIR, DigestState, STATE_PATH, digest_date};
use aarambh_core::sync::{INITIAL_DAYS, day_from_events_file};
use aarambh_core::{Day, Event, EventBody, Id, Judgment, Log, Taxonomy, Timestamp, parse_day_file};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::cache;
use crate::github::{Fetched, Gh, GhError};
use crate::settings::Settings;
use crate::util::{now_ms, random_u128};

const EVENTS_DIR: &str = "data/events";
const TAXONOMY_PATH: &str = "data/taxonomy.toml";

#[derive(Clone, Debug, PartialEq)]
pub enum SyncState {
    /// Before the first refresh finishes.
    Starting,
    NeedsSetup,
    Syncing,
    Synced {
        at_ms: u64,
    },
    Offline,
    Failed {
        message: String,
        /// The fix is in Settings (token or repo), not waiting.
        settings: bool,
    },
}

#[derive(Clone, Copy)]
pub struct Store {
    pub log: RwSignal<Log>,
    pub state: RwSignal<SyncState>,
    /// Posts not yet in the repo.
    pub queued: RwSignal<BTreeSet<Id>>,
    /// Failure modes for post-mortems, from the repo once fetched.
    pub taxonomy: RwSignal<Taxonomy>,
    /// Digest files as (date, blob sha), newest first.
    pub digests: RwSignal<Vec<(String, String)>>,
    pub digest_state: RwSignal<Option<DigestState>>,
    /// A note another tab asked the feed to scroll to.
    pub focus: RwSignal<Option<Id>>,
    /// Machine judgments for loaded notes, by note id.
    pub judgments: RwSignal<HashMap<Id, Judgment>>,
    /// Entries in day files this version couldn't read.
    pub skipped: RwSignal<usize>,
    pub days_loaded: RwSignal<i64>,
    /// The repo has day files older than the loaded window.
    pub has_older: RwSignal<bool>,
    settings: RwSignal<Settings>,
}

thread_local! {
    static REFRESHING: Cell<bool> = const { Cell::new(false) };
    static FLUSHING: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` unless a run with the same flag is already in progress.
async fn exclusive<F: Future<Output = ()>>(flag: &'static std::thread::LocalKey<Cell<bool>>, f: F) {
    if flag.replace(true) {
        return;
    }
    f.await;
    flag.set(false);
}

impl Store {
    pub fn new(settings: RwSignal<Settings>) -> Self {
        Store {
            log: RwSignal::new(Log::new()),
            state: RwSignal::new(SyncState::Starting),
            queued: RwSignal::new(BTreeSet::new()),
            taxonomy: RwSignal::new(Taxonomy::default()),
            digests: RwSignal::new(Vec::new()),
            digest_state: RwSignal::new(None),
            judgments: RwSignal::new(HashMap::new()),
            focus: RwSignal::new(None),
            skipped: RwSignal::new(0),
            days_loaded: RwSignal::new(INITIAL_DAYS),
            has_older: RwSignal::new(false),
            settings,
        }
    }

    /// Loads the cache, then syncs. Call once on startup.
    pub fn start(self) {
        spawn_local(async move {
            self.load_cache().await;
            self.sync().await;
        });
    }

    async fn load_cache(self) {
        let mut events = Vec::new();
        let mut skipped = 0;
        for f in cache::all_files().await.unwrap_or_default() {
            if let Ok(p) = parse_day_file(&f.text) {
                events.extend(p.events);
                skipped += p.skipped;
            }
        }
        let queued = queued_events().await;
        self.queued.set(queued.iter().map(|e| e.id).collect());
        events.extend(queued);
        self.log.update(|l| {
            l.extend(events);
        });
        self.skipped.set(skipped);
        if let Some(t) = cache::get(cache::META, cache::META_TAXONOMY)
            .await
            .ok()
            .flatten()
            && let Ok(t) = Taxonomy::parse(&t)
        {
            self.taxonomy.set(t);
        }
        if let Some(list) = cache::get(cache::META, cache::META_DIGESTS)
            .await
            .ok()
            .flatten()
            && let Ok(list) = serde_json::from_str(&list)
        {
            self.digests.set(list);
        }
        if let Some(s) = cache::get(cache::META, cache::META_DIGEST_STATE)
            .await
            .ok()
            .flatten()
        {
            self.digest_state.set(DigestState::parse(&s).ok());
        }
        let judgments: HashMap<Id, Judgment> = cache::all(cache::JUDGMENTS)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|s| serde_json::from_str::<cache::CachedFile>(s).ok())
            .filter_map(|c| serde_json::from_str::<Judgment>(&c.text).ok())
            .map(|j| (j.note_id, j))
            .collect();
        self.judgments.set(judgments);
    }

    /// Fetches judgments for loaded notes that are new or changed. The
    /// listing is conditional; files are fetched only when their sha moved.
    pub async fn refresh_judgments(self) -> Result<(), GhError> {
        let Some(gh) = self.settings.get_untracked().client() else {
            return Ok(());
        };
        let etag = cache::get(cache::META, cache::META_JUDGMENTS_ETAG)
            .await
            .ok()
            .flatten()
            .filter(|e| !e.is_empty());
        let listing: Vec<(String, String)> =
            match gh.list_dir("data/judgments", etag.as_deref()).await? {
                Fetched::Found((entries, etag)) => {
                    let list: Vec<(String, String)> = entries
                        .into_iter()
                        .filter_map(|e| Some((e.name.strip_suffix(".json")?.to_owned(), e.sha)))
                        .collect();
                    if let Ok(json) = serde_json::to_string(&list) {
                        let _ = cache::put(cache::META, cache::META_JUDGMENTS, &json).await;
                    }
                    let _ = cache::put(
                        cache::META,
                        cache::META_JUDGMENTS_ETAG,
                        etag.as_deref().unwrap_or(""),
                    )
                    .await;
                    list
                }
                Fetched::Missing => Vec::new(),
                // Unchanged listing: still fetch judgments for notes loaded since.
                Fetched::NotModified => cache::get(cache::META, cache::META_JUDGMENTS)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
            };
        for (name, sha) in listing {
            let Ok(id) = name.parse::<Id>() else { continue };
            if !self.log.with_untracked(|l| l.contains(id)) {
                continue;
            }
            let cached = cache::get(cache::JUDGMENTS, &name).await.ok().flatten();
            if cached
                .and_then(|c| serde_json::from_str::<cache::CachedFile>(&c).ok())
                .is_some_and(|c| c.sha == sha)
            {
                continue;
            }
            let Fetched::Found(f) = gh
                .get_file(&format!("data/judgments/{name}.json"), None)
                .await?
            else {
                continue;
            };
            let Ok(j) = serde_json::from_str::<Judgment>(&f.text) else {
                continue;
            };
            if let Ok(json) = serde_json::to_string(&cache::CachedFile {
                sha: f.sha,
                text: f.text,
            }) {
                let _ = cache::put(cache::JUDGMENTS, &name, &json).await;
            }
            self.judgments.update(|m| {
                m.insert(id, j);
            });
        }
        Ok(())
    }

    /// Re-reads the list of digests and state.json (both conditional).
    pub async fn refresh_digests(self) -> Result<(), GhError> {
        let settings = self.settings.get_untracked();
        let Some(gh) = settings.client() else {
            return Ok(());
        };
        let etag = cache::get(cache::META, cache::META_DIGESTS_ETAG)
            .await
            .ok()
            .flatten()
            .filter(|e| !e.is_empty());
        match gh.list_dir(DIGESTS_DIR, etag.as_deref()).await? {
            Fetched::Found((entries, etag)) => {
                let mut list: Vec<(String, String)> = entries
                    .into_iter()
                    .filter_map(|e| Some((digest_date(&e.name)?.to_owned(), e.sha)))
                    .collect();
                list.sort_by(|a, b| b.0.cmp(&a.0));
                if let Ok(json) = serde_json::to_string(&list) {
                    let _ = cache::put(cache::META, cache::META_DIGESTS, &json).await;
                }
                let _ = cache::put(
                    cache::META,
                    cache::META_DIGESTS_ETAG,
                    etag.as_deref().unwrap_or(""),
                )
                .await;
                self.digests.set(list);
            }
            Fetched::Missing => self.digests.set(Vec::new()),
            Fetched::NotModified => {}
        }
        let etag = cache::get(cache::META, cache::META_DIGEST_STATE_ETAG)
            .await
            .ok()
            .flatten()
            .filter(|e| !e.is_empty());
        if let Fetched::Found(f) = gh.get_file(STATE_PATH, etag.as_deref()).await? {
            let _ = cache::put(cache::META, cache::META_DIGEST_STATE, &f.text).await;
            let _ = cache::put(
                cache::META,
                cache::META_DIGEST_STATE_ETAG,
                f.etag.as_deref().unwrap_or(""),
            )
            .await;
            self.digest_state.set(DigestState::parse(&f.text).ok());
        }
        Ok(())
    }

    /// A digest's Markdown, from the cache when its sha hasn't changed.
    pub async fn digest_text(self, date: String, sha: String) -> Result<String, GhError> {
        let key = format!("digest:{date}");
        if let Some(c) = cache::get(cache::META, &key).await.ok().flatten()
            && let Ok(c) = serde_json::from_str::<cache::CachedFile>(&c)
            && c.sha == sha
        {
            return Ok(c.text);
        }
        let gh = self
            .settings
            .get_untracked()
            .client()
            .ok_or(GhError::Unauthorized)?;
        match gh
            .get_file(&format!("{DIGESTS_DIR}/{date}.md"), None)
            .await?
        {
            Fetched::Found(f) => {
                let cached = cache::CachedFile {
                    sha: f.sha,
                    text: f.text.clone(),
                };
                if let Ok(json) = serde_json::to_string(&cached) {
                    let _ = cache::put(cache::META, &key, &json).await;
                }
                Ok(f.text)
            }
            _ => Err(GhError::NotFound),
        }
    }

    /// Starts the digest workflow.
    pub async fn summarise_now(self) -> Result<(), GhError> {
        let gh = self
            .settings
            .get_untracked()
            .client()
            .ok_or(GhError::Unauthorized)?;
        gh.dispatch_workflow("digest.yml").await
    }

    pub fn pending(self) -> usize {
        self.queued.with(BTreeSet::len)
    }

    /// A new event by this device's person, timestamped now.
    pub fn new_event(self, body: EventBody) -> Event {
        let name = self.settings.with_untracked(|s| s.name.trim().to_owned());
        Event::new(Id::new(now_ms(), random_u128()), name, body)
    }

    async fn refresh_taxonomy(self, gh: &Gh) {
        let etag = cache::get(cache::META, cache::META_TAXONOMY_ETAG)
            .await
            .ok()
            .flatten();
        if let Ok(Fetched::Found(f)) = gh.get_file(TAXONOMY_PATH, etag.as_deref()).await {
            match Taxonomy::parse(&f.text) {
                Ok(t) => {
                    self.taxonomy.set(t);
                    let _ = cache::put(cache::META, cache::META_TAXONOMY, &f.text).await;
                    let _ = cache::put(
                        cache::META,
                        cache::META_TAXONOMY_ETAG,
                        f.etag.as_deref().unwrap_or(""),
                    )
                    .await;
                }
                Err(e) => leptos::logging::warn!("{e}; keeping the previous failure modes"),
            }
        }
    }

    /// Sends queued posts first (what the person is waiting on), then
    /// refreshes.
    pub async fn sync(self) {
        self.flush().await;
        self.refresh().await;
        // After notes load, so judgments for newly loaded notes are fetched.
        let _ = self.refresh_judgments().await;
    }

    pub fn sync_soon(self) {
        spawn_local(self.sync());
    }

    pub fn load_older(self) {
        self.days_loaded.update(|d| *d += INITIAL_DAYS);
        spawn_local(async move {
            // The listing may be unchanged, but we now want more of it.
            let _ = cache::put(cache::META, cache::META_EVENTS_ETAG, "").await;
            self.refresh().await;
        });
    }

    /// Adds a new event: shown at once, saved to the queue, then sent.
    pub fn post(self, event: Event) {
        self.log.update(|l| {
            l.insert(event.clone());
        });
        self.queued.update(|q| {
            q.insert(event.id);
        });
        spawn_local(async move {
            if let Ok(json) = serde_json::to_string(&event) {
                let _ = cache::put(cache::QUEUE, &event.id.to_string(), &json).await;
            }
            self.flush().await;
        });
    }

    /// Called when the repo changes or the device is forgotten.
    pub async fn reset_cache(self, keep_queue: bool) {
        let stores: &[&str] = if keep_queue {
            &[cache::FILES, cache::META, cache::JUDGMENTS]
        } else {
            &[cache::FILES, cache::META, cache::QUEUE, cache::JUDGMENTS]
        };
        let _ = cache::clear(stores).await;
        let queued = if keep_queue {
            queued_events().await
        } else {
            Vec::new()
        };
        self.queued.set(queued.iter().map(|e| e.id).collect());
        self.log.set({
            let mut l = Log::new();
            l.extend(queued);
            l
        });
        self.skipped.set(0);
        self.judgments.set(HashMap::new());
        self.days_loaded.set(INITIAL_DAYS);
        self.has_older.set(false);
    }

    fn fail(self, e: &GhError) {
        self.state.set(match e {
            GhError::Network => SyncState::Offline,
            e => SyncState::Failed {
                message: e.message(),
                settings: matches!(
                    e,
                    GhError::Unauthorized | GhError::Forbidden | GhError::NotFound
                ),
            },
        });
    }

    async fn refresh(self) {
        exclusive(&REFRESHING, async move {
            let settings = self.settings.get_untracked();
            let Some(gh) = settings.client().filter(|_| settings.is_complete()) else {
                self.state.set(SyncState::NeedsSetup);
                return;
            };
            // The cache belongs to one repo; start over if it changed.
            let repo = settings
                .repo_ref()
                .map(|r| r.to_string())
                .unwrap_or_default();
            if cache::get(cache::META, cache::META_REPO)
                .await
                .ok()
                .flatten()
                .as_deref()
                != Some(repo.as_str())
            {
                self.reset_cache(true).await;
                let _ = cache::put(cache::META, cache::META_REPO, &repo).await;
            }
            self.state.set(SyncState::Syncing);
            self.refresh_taxonomy(&gh).await;

            let etag = cache::get(cache::META, cache::META_EVENTS_ETAG)
                .await
                .ok()
                .flatten()
                .filter(|e| !e.is_empty());
            let (entries, new_etag) = match gh.list_dir(EVENTS_DIR, etag.as_deref()).await {
                Ok(Fetched::NotModified) | Ok(Fetched::Missing) => {
                    self.state.set(SyncState::Synced { at_ms: now_ms() });
                    return;
                }
                Ok(Fetched::Found(x)) => x,
                Err(e) => return self.fail(&e),
            };

            let today = Timestamp(now_ms()).day();
            let oldest = today.offset(1 - self.days_loaded.get_untracked());
            let mut days: Vec<(Day, String, String)> = entries
                .into_iter()
                .filter(|e| e.kind == "file")
                .filter_map(|e| Some((day_from_events_file(&e.name)?, e.path, e.sha)))
                .collect();
            days.sort_by_key(|d| std::cmp::Reverse(d.0));
            self.has_older.set(days.iter().any(|(d, ..)| *d < oldest));

            for (day, path, sha) in days.into_iter().filter(|(d, ..)| *d >= oldest) {
                let cached = cache::get_file(&path).await.ok().flatten();
                if cached.is_some_and(|c| c.sha == sha) {
                    continue;
                }
                let file = match gh.get_file(&path, None).await {
                    Ok(Fetched::Found(f)) => f,
                    Ok(_) => continue,
                    Err(e) => return self.fail(&e),
                };
                let Ok(parsed) = parse_day_file(&file.text) else {
                    leptos::logging::warn!("{day}: day file is not a JSON array; skipped");
                    continue;
                };
                let _ = cache::put_file(&path, &file.sha, &file.text).await;
                self.skipped.update(|n| *n += parsed.skipped);
                self.log.update(|l| {
                    l.extend(parsed.events);
                });
            }
            // Saved only once every changed file was fetched, so a failure
            // part-way through is retried in full next time.
            if let Some(e) = new_etag {
                let _ = cache::put(cache::META, cache::META_EVENTS_ETAG, &e).await;
            }
            self.state.set(SyncState::Synced { at_ms: now_ms() });
        })
        .await;
    }

    async fn flush(self) {
        exclusive(&FLUSHING, async move {
            let settings = self.settings.get_untracked();
            let Some(gh) = settings.client().filter(|_| settings.is_complete()) else {
                return;
            };
            // A few rounds pick up posts made while we were writing; the cap
            // stops a broken queue store from looping forever.
            for _ in 0..3 {
                let queued = queued_events().await;
                self.queued.set(queued.iter().map(|e| e.id).collect());
                if queued.is_empty() {
                    return;
                }
                let mut by_day: BTreeMap<Day, Vec<Event>> = BTreeMap::new();
                for e in queued {
                    by_day.entry(e.ts.day()).or_default().push(e);
                }
                let mut wrote_any = false;
                for (day, events) in by_day {
                    match gh.append_events(day, settings.name.trim(), &events).await {
                        Ok(file) => {
                            let path = day.events_path();
                            if !file.sha.is_empty() {
                                let _ = cache::put_file(&path, &file.sha, &file.text).await;
                            }
                            if let Ok(parsed) = parse_day_file(&file.text) {
                                self.log.update(|l| {
                                    l.extend(parsed.events);
                                });
                            }
                            let ids: Vec<String> =
                                events.iter().map(|e| e.id.to_string()).collect();
                            let _ = cache::delete(cache::QUEUE, &ids).await;
                            self.queued.update(|q| {
                                for e in &events {
                                    q.remove(&e.id);
                                }
                            });
                            wrote_any = true;
                        }
                        Err(e) => {
                            self.fail(&e);
                            return;
                        }
                    }
                }
                if wrote_any {
                    self.state.set(SyncState::Synced { at_ms: now_ms() });
                }
            }
        })
        .await;
    }

    /// Authors seen in the notebook, most recent first.
    pub fn known_names(self) -> Signal<Vec<String>> {
        Signal::derive(move || {
            self.log.with(|l| {
                let mut seen = Vec::<String>::new();
                for e in l.newest_first() {
                    if !e.author.is_empty() && !seen.contains(&e.author) {
                        seen.push(e.author.clone());
                    }
                }
                seen
            })
        })
    }
}

async fn queued_events() -> Vec<Event> {
    cache::all(cache::QUEUE)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}
