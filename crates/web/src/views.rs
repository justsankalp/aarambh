//! Tab screens. Map and Digest are filled in by later phases; Notes shows a
//! plain list until the full feed lands in phase 2.

use aarambh_core::Event;
use leptos::prelude::*;

use crate::store::{Store, SyncState};
use crate::util::{ago, clock, day_label, local_ymd, now_ms};

#[component]
pub fn SyncLine() -> impl IntoView {
    let store = expect_context::<Store>();
    // Re-render "n min ago" once a minute.
    let tick = RwSignal::new(now_ms());
    let handle = set_interval_with_handle(
        move || tick.set(now_ms()),
        std::time::Duration::from_secs(60),
    )
    .ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });
    let text = move || {
        let now = tick.get();
        let pending = store.queued.with(|q| q.len());
        let waiting = (pending > 0).then(|| format!(" · {pending} waiting to send"));
        let base = match store.state.get() {
            SyncState::Starting | SyncState::Syncing => "Syncing…".to_owned(),
            SyncState::NeedsSetup => String::new(),
            SyncState::Synced { at_ms } => format!("Synced {}", ago(at_ms, now.max(at_ms))),
            SyncState::Offline => "Offline. Notes will send when you're back.".to_owned(),
            SyncState::Failed(msg) => msg,
        };
        format!("{base}{}", waiting.unwrap_or_default())
    };
    let is_error = move || matches!(store.state.get(), SyncState::Failed(_));
    view! { <p class="sync-line" class:error=is_error role="status" aria-live="polite">{text}</p> }
}

fn note_card(e: Event) -> impl IntoView {
    let note = e.as_note().cloned();
    let (kind, text) = note
        .map(|n| (n.kind.as_str(), n.text))
        .unwrap_or(("note", String::new()));
    view! {
        <article class="card" data-id=e.id.to_string()>
            <div class="meta">
                <span class="kind">{kind}</span>
                <span>{e.author}</span>
                <span>{clock(e.ts.0)}</span>
            </div>
            <p class="note-text">{text}</p>
        </article>
    }
}

#[component]
pub fn NotesView() -> impl IntoView {
    let store = expect_context::<Store>();
    let groups = move || {
        let now = now_ms();
        let mut out: Vec<(String, Vec<Event>)> = Vec::new();
        let mut last = None;
        store.log.with(|l| {
            for e in l.newest_first().filter(|e| e.as_note().is_some()) {
                let day = local_ymd(e.ts.0);
                if last != Some(day) {
                    out.push((day_label(e.ts.0, now), Vec::new()));
                    last = Some(day);
                }
                out.last_mut().expect("pushed above").1.push(e.clone());
            }
        });
        out
    };
    let is_empty = move || store.log.with(|l| !l.iter().any(|e| e.as_note().is_some()));
    view! {
        <SyncLine />
        <Show
            when=move || !is_empty()
            fallback=move || {
                view! {
                    <div class="empty">
                        {move || match store.state.get() {
                            SyncState::NeedsSetup => view! {
                                <p>"This device isn't connected yet."</p>
                                <p><a href="#settings">"Open settings"</a></p>
                            }.into_any(),
                            SyncState::Starting | SyncState::Syncing => view! { <p>"Loading notes…"</p> }.into_any(),
                            _ => view! { <p>"No notes yet."</p> }.into_any(),
                        }}
                    </div>
                }
            }
        >
            <For
                each=groups
                key=|(label, events)| (label.clone(), events.len(), events.first().map(|e| e.id))
                children=|(label, events)| {
                    view! {
                        <section>
                            <h3 class="day-head">{label}</h3>
                            <div class="stack">{events.into_iter().map(note_card).collect_view()}</div>
                        </section>
                    }
                }
            />
        </Show>
        <Show when=move || store.has_older.get()>
            <div class="actions">
                <button type="button" class="btn quiet" on:click=move |_| store.load_older()>
                    "Load older"
                </button>
            </div>
        </Show>
    }
}

#[component]
pub fn MapView() -> impl IntoView {
    view! {
        <h2>"Map"</h2>
        <p class="lede">"How your ideas connect and where they stand. Coming soon."</p>
    }
}

#[component]
pub fn DigestView() -> impl IntoView {
    view! {
        <h2>"Digest"</h2>
        <p class="lede">"A daily summary of what moved. Coming soon."</p>
    }
}
