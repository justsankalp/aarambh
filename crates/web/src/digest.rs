//! The Digest tab: pick a day, read its summary, see how many notes are new
//! since the last one, and ask for a fresh summary.

use std::time::Duration;

use aarambh_core::Timestamp;
use aarambh_core::markdown::{Block, Inline, parse};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::github::{GhError, sleep_ms};
use crate::store::{Store, SyncState};
use crate::util::now_ms;

/// How long to keep checking for the new digest after "Summarise now".
const WAIT_FOR: Duration = Duration::from_secs(6 * 60);
const POLL_EVERY_MS: u32 = 15_000;

#[derive(Clone, PartialEq)]
enum Summarise {
    Idle,
    Starting,
    Waiting,
    Done,
    Failed(String),
}

fn inlines(v: Vec<Inline>) -> AnyView {
    v.into_iter()
        .map(|i| match i {
            Inline::Text(s) => s.into_any(),
            Inline::Code(s) => view! { <code>{s}</code> }.into_any(),
            Inline::Strong(c) => view! { <strong>{inlines(c)}</strong> }.into_any(),
            Inline::Emph(c) => view! { <em>{inlines(c)}</em> }.into_any(),
        })
        .collect_view()
        .into_any()
}

/// Markdown blocks as DOM nodes. Headings move down two levels to sit under
/// the page's own heading.
fn blocks(v: Vec<Block>) -> AnyView {
    v.into_iter()
        .map(|b| match b {
            Block::Heading(1, i) => view! { <h3>{inlines(i)}</h3> }.into_any(),
            Block::Heading(2, i) => view! { <h4>{inlines(i)}</h4> }.into_any(),
            Block::Heading(3, i) => view! { <h5>{inlines(i)}</h5> }.into_any(),
            Block::Heading(_, i) => view! { <h6>{inlines(i)}</h6> }.into_any(),
            Block::Paragraph(i) => view! { <p>{inlines(i)}</p> }.into_any(),
            Block::List { ordered: true, start, items } => view! {
                <ol start=start.to_string()>
                    {items.into_iter().map(|it| view! { <li>{inlines(it)}</li> }).collect_view()}
                </ol>
            }
            .into_any(),
            Block::List { ordered: false, items, .. } => view! {
                <ul>{items.into_iter().map(|it| view! { <li>{inlines(it)}</li> }).collect_view()}</ul>
            }
            .into_any(),
            Block::Rule => view! { <hr /> }.into_any(),
        })
        .collect_view()
        .into_any()
}

fn pretty_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match date.parse::<aarambh_core::Day>() {
        Ok(d) => format!("{} {} {}", d.day, MONTHS[usize::from(d.month - 1)], d.year),
        Err(_) => date.to_owned(),
    }
}

#[component]
pub fn DigestView() -> impl IntoView {
    let store = expect_context::<Store>();
    let selected = RwSignal::new(None::<String>);
    let text = RwSignal::new(None::<Result<String, String>>);
    let summarise = RwSignal::new(Summarise::Idle);
    let loading_list = RwSignal::new(true);

    spawn_local(async move {
        let _ = store.refresh_digests().await;
        loading_list.set(false);
    });

    // Follow the newest digest unless the person picked another day.
    Effect::new(move |_| {
        let list = store.digests.get();
        let keep = selected
            .get_untracked()
            .filter(|d| list.iter().any(|(x, _)| x == d));
        if keep.is_none() {
            selected.set(list.first().map(|(d, _)| d.clone()));
        }
    });

    // Load the chosen day's Markdown.
    Effect::new(move |_| {
        let Some(date) = selected.get() else {
            text.set(None);
            return;
        };
        let sha = store
            .digests
            .with(|l| l.iter().find(|(d, _)| *d == date).map(|(_, s)| s.clone()));
        let Some(sha) = sha else { return };
        text.set(None);
        spawn_local(async move {
            let r = store
                .digest_text(date.clone(), sha)
                .await
                .map_err(|e| e.message());
            if selected.get_untracked().as_deref() == Some(date.as_str()) {
                text.set(Some(r));
            }
        });
    });

    let new_notes = move || {
        store.digest_state.with(|s| {
            store
                .log
                .with(|l| s.clone().unwrap_or_default().new_note_count(l))
        })
    };
    let step = move |delta: isize| {
        store.digests.with_untracked(|l| {
            let Some(i) = selected
                .get_untracked()
                .and_then(|d| l.iter().position(|(x, _)| *x == d))
            else {
                return;
            };
            let j = i as isize + delta;
            if (0..l.len() as isize).contains(&j) {
                selected.set(Some(l[j as usize].0.clone()));
            }
        })
    };
    let at = move |delta: isize| {
        move || {
            store.digests.with(|l| {
                let i = selected
                    .get()
                    .and_then(|d| l.iter().position(|(x, _)| *x == d));
                i.is_none_or(|i| !(0..l.len() as isize).contains(&(i as isize + delta)))
            })
        }
    };

    let start = move |_| {
        let before = store
            .digest_state
            .with_untracked(|s| s.as_ref().and_then(|s| s.generated_at));
        summarise.set(Summarise::Starting);
        spawn_local(async move {
            if let Err(e) = store.summarise_now().await {
                let msg = match e {
                    GhError::Forbidden | GhError::NotFound => {
                        "The token can't start the digest workflow. It needs Actions: Read and write.".to_owned()
                    }
                    other => other.message(),
                };
                summarise.set(Summarise::Failed(msg));
                return;
            }
            summarise.set(Summarise::Waiting);
            let deadline = now_ms() + WAIT_FOR.as_millis() as u64;
            while now_ms() < deadline {
                sleep_ms(POLL_EVERY_MS).await;
                let _ = store.refresh_digests().await;
                let after = store
                    .digest_state
                    .with_untracked(|s| s.as_ref().and_then(|s| s.generated_at));
                if after != before {
                    selected.set(
                        store
                            .digests
                            .with_untracked(|l| l.first().map(|(d, _)| d.clone())),
                    );
                    summarise.set(Summarise::Done);
                    return;
                }
            }
            summarise.set(Summarise::Failed(
                "Still working, or nothing new to summarise. Check back in a few minutes.".into(),
            ));
        });
    };

    let status_line = move || {
        let n = new_notes();
        let since = store
            .digest_state
            .with(|s| s.as_ref().and_then(|s| s.generated_at).is_some());
        match (n, since) {
            (0, true) => "Up to date: no new notes since this summary.".to_owned(),
            (1, true) => "1 new note since this summary.".to_owned(),
            (n, true) => format!("{n} new notes since this summary."),
            (0, false) => "No notes to summarise yet.".to_owned(),
            (1, false) => "1 note, not summarised yet.".to_owned(),
            (n, false) => format!("{n} notes, not summarised yet."),
        }
    };

    view! {
        <div class="digest-head">
            <h2>"Digest"</h2>
            <Show when=move || store.digests.with(|l| !l.is_empty())>
                <div class="date-pick">
                    <button type="button" class="icon-btn" aria-label="Older digest" disabled=at(1) on:click=move |_| step(1)>
                        "‹"
                    </button>
                    <select
                        aria-label="Digest date"
                        prop:value=move || selected.get().unwrap_or_default()
                        on:change:target=move |ev| selected.set(Some(ev.target().value()))
                    >
                        {move || store.digests.get().into_iter().map(|(d, _)| {
                            let label = pretty_date(&d);
                            view! { <option value=d>{label}</option> }
                        }).collect_view()}
                    </select>
                    <button type="button" class="icon-btn" aria-label="Newer digest" disabled=at(-1) on:click=move |_| step(-1)>
                        "›"
                    </button>
                </div>
            </Show>
        </div>
        <div class="digest-bar">
            <p class="muted" role="status">{status_line}</p>
            <button
                type="button"
                class="btn"
                disabled=move || new_notes() == 0 || matches!(summarise.get(), Summarise::Starting | Summarise::Waiting)
                on:click=start
            >
                "Summarise now"
            </button>
        </div>
        {move || match summarise.get() {
            Summarise::Starting => view! { <p class="notice" role="status">"Starting…"</p> }.into_any(),
            Summarise::Waiting => view! {
                <p class="notice" role="status">"Summarising. This usually takes a minute or two; you can leave this tab."</p>
            }
            .into_any(),
            Summarise::Done => view! { <p class="notice" role="status">"New summary ready."</p> }.into_any(),
            Summarise::Failed(m) => view! { <p class="status-msg error" role="alert">{m}</p> }.into_any(),
            Summarise::Idle => ().into_any(),
        }}
        {move || match (selected.get(), text.get()) {
            (None, _) if loading_list.get() && store.digests.with(Vec::is_empty) => {
                view! { <p class="empty">"Loading…"</p> }.into_any()
            }
            (None, _) => {
                let offline = store.state.with(|s| matches!(s, SyncState::Offline | SyncState::NeedsSetup));
                view! {
                    <div class="empty">
                        <p>{if offline { "Digests appear here once this device is online and connected." } else { "No digest yet." }}</p>
                        <p>"A summary is written every morning at 08:00 IST when there's something new, or tap Summarise now."</p>
                    </div>
                }
                .into_any()
            }
            (Some(_), None) => view! { <p class="empty">"Loading…"</p> }.into_any(),
            (Some(_), Some(Err(e))) => view! { <p class="status-msg error" role="alert">{e}</p> }.into_any(),
            (Some(_), Some(Ok(md))) => view! { <article class="digest card">{blocks(parse(&md))}</article> }.into_any(),
        }}
        {move || store.digest_state.with(|s| {
            s.as_ref().and_then(|s| Some((s.generated_at?, s.model.clone()))).map(|(at, model)| {
                let when = Timestamp(at.0).to_string();
                view! { <p class="small-print muted digest-foot">"Last summary " {when[..16].replace('T', " ")} " UTC · " {model}</p> }
            })
        })}
    }
}
