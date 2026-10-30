//! The notes feed: newest first, grouped by local day, filterable by person
//! and kind. Each note can be responded to; references jump to the original.

use std::time::Duration;

use aarambh_core::feed::{FeedFilter, excerpt};
use aarambh_core::{EventBody, Id, IdeaStatus, Lesson, Log, NoteKind};
use leptos::prelude::*;

use crate::compose::Composer;
use crate::status::{StatusSheet, status_label};
use crate::store::{Store, SyncState};
use crate::util::{clock, day_label, local_day_number, local_ymd, now_ms, weekday_initial};
use crate::views::SyncLine;

/// What a reference to another note shows.
#[derive(Clone, PartialEq)]
struct RefView {
    to: Id,
    rel: &'static str,
    author: String,
    quote: String,
}

#[derive(Clone, PartialEq)]
enum Body {
    Note {
        kind: NoteKind,
        text: String,
        refs: Vec<RefView>,
        status: Option<IdeaStatus>,
    },
    Status {
        status: IdeaStatus,
        idea: RefView,
        lesson: Option<Lesson>,
    },
}

/// One row in the feed. Rows are re-rendered only when their key changes.
#[derive(Clone, PartialEq)]
struct Item {
    id: Id,
    author: String,
    ts: u64,
    day_head: Option<String>,
    queued: bool,
    /// Changes when anything shown besides the note itself changes: its
    /// idea status, or a referenced note loading.
    fingerprint: u64,
    body: Body,
}

fn fingerprint(body: &Body) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    let refs: &[RefView] = match body {
        Body::Note { refs, status, .. } => {
            status.map(status_label).hash(&mut h);
            refs
        }
        Body::Status { idea, .. } => std::slice::from_ref(idea),
    };
    for r in refs {
        (&r.author, &r.quote).hash(&mut h);
    }
    h.finish()
}

fn ref_view(log: &Log, to: Id, rel: &'static str) -> RefView {
    match log.get(to) {
        Some(e) => RefView {
            to,
            rel,
            author: e.author.clone(),
            quote: e
                .as_note()
                .map(|n| excerpt(&n.text, 80))
                .unwrap_or_default(),
        },
        None => RefView {
            to,
            rel,
            author: String::new(),
            quote: "an older note".into(),
        },
    }
}

fn build_items(store: Store, filter: &FeedFilter) -> Vec<Item> {
    let now = now_ms();
    let queued = store.queued.get();
    store.log.with(|log| {
        let mut items = Vec::new();
        let mut last_day = None;
        for e in filter.apply(log) {
            let body = match &e.body {
                EventBody::Note(n) => Body::Note {
                    kind: n.kind,
                    text: n.text.clone(),
                    refs: n
                        .links
                        .iter()
                        .map(|l| ref_view(log, l.to, l.rel.as_str()))
                        .collect(),
                    status: (n.kind == NoteKind::Idea).then(|| log.status_of(e.id)),
                },
                EventBody::Status(s) => Body::Status {
                    status: s.status,
                    idea: ref_view(log, s.idea, ""),
                    lesson: s.lesson.clone().filter(|l| *l != Lesson::default()),
                },
                EventBody::Verdict(_) => continue,
            };
            let day = local_ymd(e.ts.0);
            let day_head = (last_day != Some(day)).then(|| day_label(e.ts.0, now));
            last_day = Some(day);
            items.push(Item {
                id: e.id,
                author: e.author.clone(),
                ts: e.ts.0,
                day_head,
                queued: queued.contains(&e.id),
                fingerprint: fingerprint(&body),
                body,
            });
        }
        items
    })
}

/// Scrolls to a note and highlights it briefly. Clears filters that hide it.
fn jump_to(
    store: Store,
    filter: RwSignal<FeedFilter>,
    notice: RwSignal<Option<&'static str>>,
    id: Id,
) {
    let visible = store
        .log
        .with_untracked(|l| l.get(id).map(|e| filter.with_untracked(|f| f.matches(e))));
    match visible {
        None => {
            notice.set(Some("That note is older than what's loaded."));
            return;
        }
        Some(false) => filter.set(FeedFilter::default()),
        Some(true) => {}
    }
    notice.set(None);
    request_animation_frame(move || {
        let Some(el) = document().get_element_by_id(&format!("n-{id}")) else {
            return;
        };
        let reduce = window()
            .match_media("(prefers-reduced-motion: reduce)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches());
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Center);
        opts.set_behavior(if reduce {
            web_sys::ScrollBehavior::Instant
        } else {
            web_sys::ScrollBehavior::Smooth
        });
        el.scroll_into_view_with_scroll_into_view_options(&opts);
        let _ = el.class_list().add_1("flash");
        if let Ok(h) = wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el.clone()) {
            let opts = web_sys::FocusOptions::new();
            opts.set_prevent_scroll(true);
            let _ = h.focus_with_options(&opts);
        }
        set_timeout(
            move || drop(el.class_list().remove_1("flash")),
            Duration::from_millis(1600),
        );
    });
}

#[component]
fn FilterBar(filter: RwSignal<FeedFilter>) -> impl IntoView {
    let store = expect_context::<Store>();
    let names = store.known_names();
    let person = move |who: Option<String>| {
        let label = who.clone().unwrap_or_else(|| "Everyone".into());
        let pressed = {
            let who = who.clone();
            move || filter.with(|f| f.author == who).to_string()
        };
        view! {
            <button
                type="button"
                class="chip"
                aria-pressed=pressed
                on:click=move |_| filter.update(|f| f.author = who.clone())
            >
                {label}
            </button>
        }
    };
    let kind = move |k: Option<NoteKind>| {
        view! {
            <button
                type="button"
                class="chip"
                aria-pressed=move || filter.with(|f| f.kind == k).to_string()
                on:click=move |_| filter.update(|f| f.kind = k)
            >
                {k.map_or("All kinds", NoteKind::as_str)}
            </button>
        }
    };
    view! {
        <div class="filters">
            <div class="chips scroll" role="group" aria-label="Person">
                {person(None)}
                {move || names.get().into_iter().map(|n| person(Some(n))).collect_view()}
            </div>
            <div class="chips scroll" role="group" aria-label="Kind of note">
                {kind(None)}
                {NoteKind::ALL.map(|k| kind(Some(k)))}
            </div>
        </div>
    }
}

fn ref_button(
    r: RefView,
    store: Store,
    filter: RwSignal<FeedFilter>,
    notice: RwSignal<Option<&'static str>>,
) -> impl IntoView {
    let to = r.to;
    let who = (!r.author.is_empty()).then(|| format!("{}: ", r.author));
    view! {
        <button type="button" class="ref" on:click=move |_| jump_to(store, filter, notice, to)>
            {(!r.rel.is_empty()).then(|| view! { <span class="rel">{r.rel}</span> })}
            <span class="ref-text">{who}"“"{r.quote}"”"</span>
        </button>
    }
}

fn render_item(
    item: Item,
    filter: RwSignal<FeedFilter>,
    notice: RwSignal<Option<&'static str>>,
) -> impl IntoView {
    let store = expect_context::<Store>();
    let composer = expect_context::<Composer>();
    let status_sheet = expect_context::<StatusSheet>();
    let Item {
        id,
        author,
        ts,
        day_head,
        queued,
        body,
        ..
    } = item;
    let head = day_head.map(|d| view! { <h3 class="day-head">{d}</h3> });
    let pending = queued.then(|| view! { <span class="pending">"sending…"</span> });
    let row = match body {
        Body::Note { kind, text, refs, status } => view! {
            <article class="card note" id=format!("n-{id}") tabindex="-1" data-kind=kind.as_str()>
                <div class="meta">
                    <span class="kind">{kind.as_str()}</span>
                    <span>{author}</span>
                    <span>{clock(ts)}</span>
                    {pending}
                    {status.map(|s| view! { <span class="badge" data-status=status_label(s)>{status_label(s)}</span> })}
                </div>
                {refs.into_iter().map(|r| ref_button(r, store, filter, notice)).collect_view()}
                <p class="note-text">{text}</p>
                <div class="card-actions">
                    <button type="button" class="link-btn" on:click=move |_| composer.respond(id, kind)>
                        "Respond"
                    </button>
                    {status.is_some().then(|| view! {
                        <button type="button" class="link-btn" on:click=move |_| status_sheet.open(id)>
                            "Status"
                        </button>
                    })}
                </div>
            </article>
        }
        .into_any(),
        Body::Status { status, idea, lesson } => view! {
            <div class="status-row" id=format!("n-{id}") tabindex="-1">
                <p class="meta">
                    <span>{author}" marked an idea "</span>
                    <span class="badge" data-status=status_label(status)>{status_label(status)}</span>
                    <span>{clock(ts)}</span>
                    {pending}
                </p>
                {ref_button(idea, store, filter, notice)}
                {lesson.map(|l| view! {
                    <dl class="lesson">
                        {(!l.why.is_empty()).then(|| view! { <dt>"Why"</dt><dd>{l.why}</dd> })}
                        {(!l.would_change_mind.is_empty()).then(|| view! { <dt>"Would change our mind"</dt><dd>{l.would_change_mind}</dd> })}
                        {(!l.check_earlier.is_empty()).then(|| view! { <dt>"Check earlier next time"</dt><dd>{l.check_earlier}</dd> })}
                        {(!l.modes.is_empty()).then(|| {
                            let labels = store.taxonomy.with_untracked(|t| l.modes.iter().map(|m| t.label(m).to_owned()).collect::<Vec<_>>().join(", "));
                            view! { <dt>"Failure modes"</dt><dd>{labels}</dd> }
                        })}
                    </dl>
                })}
            </div>
        }
        .into_any(),
    };
    view! { {head} {row} }
}

/// Seven dots for the last seven days (amber: both of you wrote; ring: one
/// of you did), and how many ideas are open and decided. No streaks.
#[component]
fn WeekRow() -> impl IntoView {
    use aarambh_core::momentum::{Dot, week};
    let store = expect_context::<Store>();
    let today = local_day_number(now_ms());
    let dots = Memo::new(move |_| {
        store.log.with(|l| {
            let entries: Vec<(i64, String)> = l
                .newest_first()
                .take_while(|e| local_day_number(e.ts.0) > today - 7)
                .filter(|e| e.as_note().is_some())
                .map(|e| (local_day_number(e.ts.0), e.author.clone()))
                .collect();
            week(entries.iter().map(|(d, a)| (*d, a.as_str())), today)
        })
    });
    let ideas = Memo::new(move |_| {
        store.log.with(|l| {
            let mut open = 0;
            let mut decided = 0;
            for e in l
                .iter()
                .filter(|e| e.as_note().is_some_and(|n| n.kind == NoteKind::Idea))
            {
                match l.status_of(e.id) {
                    IdeaStatus::Active => open += 1,
                    IdeaStatus::Decided => decided += 1,
                    _ => {}
                }
            }
            (open, decided)
        })
    });
    let summary = move || {
        let d = dots.get();
        let both = d.iter().filter(|x| **x == Dot::Both).count();
        let one = d.iter().filter(|x| **x == Dot::One).count();
        format!("Last 7 days: both of you wrote on {both}, one of you on {one}.")
    };
    view! {
        <div class="week">
            <div class="week-dots" role="img" aria-label=summary>
                {move || {
                    dots.get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, d)| {
                            let day = today - 6 + i as i64;
                            let class = match d {
                                Dot::Both => "dot both",
                                Dot::One => "dot one",
                                Dot::Nobody => "dot none",
                            };
                            view! {
                                <span class="day" class:today=i == 6>
                                    <span class=class></span>
                                    <span class="initial" aria-hidden="true">{weekday_initial(day)}</span>
                                </span>
                            }
                        })
                        .collect_view()
                }}
            </div>
            {move || {
                let (open, decided) = ideas.get();
                (open + decided > 0).then(|| view! {
                    <p class="idea-count">
                        <a href="#map">{format!("{open} open")}</a>" · "{format!("{decided} decided")}
                    </p>
                })
            }}
        </div>
    }
}

#[component]
pub fn NotesView() -> impl IntoView {
    let store = expect_context::<Store>();
    let filter = RwSignal::new(FeedFilter::default());
    let notice = RwSignal::new(None::<&'static str>);
    let items = Memo::new(move |_| filter.with(|f| build_items(store, f)));
    let has_notes = move || store.log.with(|l| l.iter().any(|e| e.as_note().is_some()));

    // Another tab (the Map) asked to show a note.
    Effect::new(move |_| {
        if let Some(id) = store.focus.get() {
            store.focus.set(None);
            set_timeout(
                move || jump_to(store, filter, notice, id),
                std::time::Duration::from_millis(60),
            );
        }
    });

    view! {
        <SyncLine />
        <Show when=has_notes>
            <WeekRow />
            <FilterBar filter=filter />
        </Show>
        {move || notice.get().map(|n| view! {
            <p class="notice" role="status">
                {n}" "
                <button type="button" class="link-btn" on:click=move |_| { notice.set(None); store.load_older(); }>
                    "Load older notes"
                </button>
            </p>
        })}
        <div class="feed">
            <For
                each=move || items.get()
                key=|it| (it.id, it.day_head.clone(), it.queued, it.fingerprint)
                children=move |it| render_item(it, filter, notice)
            />
        </div>
        {move || {
            if !items.with(Vec::is_empty) {
                return ().into_any();
            }
            let msg = match store.state.get() {
                SyncState::NeedsSetup => view! {
                    <p>"This device isn't connected yet."</p>
                    <p><a href="#settings">"Open settings"</a></p>
                }
                .into_any(),
                SyncState::Starting | SyncState::Syncing if !has_notes() => view! { <p>"Loading notes…"</p> }.into_any(),
                _ if filter.with(FeedFilter::is_active) => view! {
                    <p>"No notes match these filters."</p>
                    <p>
                        <button type="button" class="link-btn" on:click=move |_| filter.set(FeedFilter::default())>
                            "Show everything"
                        </button>
                    </p>
                }
                .into_any(),
                _ => view! {
                    <p>"Nothing here yet."</p>
                    <p>"Tap + to write the first note."</p>
                }
                .into_any(),
            };
            view! { <div class="empty">{msg}</div> }.into_any()
        }}
        <Show when=move || store.has_older.get()>
            <div class="actions center">
                <button type="button" class="btn quiet" on:click=move |_| store.load_older()>
                    "Load older"
                </button>
            </div>
        </Show>
    }
}
