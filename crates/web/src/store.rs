//! Local-first sync. The app renders from the IndexedDB cache straight away,
//! then refreshes from the repo in the background. Posts are optimistic:
//! they go into the log and the offline queue at once, and the queue is
//! flushed to the repo whenever we're online.

use std::cell::Cell;
use std::collections::BTreeMap;

use aarambh_core::sync::{INITIAL_DAYS, day_from_events_file};
use aarambh_core::{Day, Event, Log, Timestamp, parse_day_file};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::cache;
use crate::github::{Fetched, GhError};
use crate::settings::Settings;
use crate::util::now_ms;

const EVENTS_DIR: &str = "data/events";

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
    Failed(String),
}

#[derive(Clone, Copy)]
pub struct Store {
    pub log: RwSignal<Log>,
    pub state: RwSignal<SyncState>,
    /// Posts not yet in the repo.
    pub pending: RwSignal<usize>,
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
            pending: RwSignal::new(0),
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
        self.pending.set(queued.len());
        events.extend(queued);
        self.log.update(|l| {
            l.extend(events);
        });
        self.skipped.set(skipped);
    }

    /// Refresh, then flush anything queued.
    pub async fn sync(self) {
        self.refresh().await;
        self.flush().await;
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
        self.pending.update(|n| *n += 1);
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
            &[cache::FILES, cache::META]
        } else {
            &[cache::FILES, cache::META, cache::QUEUE]
        };
        let _ = cache::clear(stores).await;
        let queued = if keep_queue {
            queued_events().await
        } else {
            Vec::new()
        };
        self.pending.set(queued.len());
        self.log.set({
            let mut l = Log::new();
            l.extend(queued);
            l
        });
        self.skipped.set(0);
        self.days_loaded.set(INITIAL_DAYS);
        self.has_older.set(false);
    }

    fn fail(self, e: &GhError) {
        self.state.set(match e {
            GhError::Network => SyncState::Offline,
            e => SyncState::Failed(e.message()),
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
                self.pending.set(queued.len());
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
                            self.pending.update(|n| *n = n.saturating_sub(ids.len()));
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
