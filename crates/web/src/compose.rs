//! The compose bottom sheet. Tapping + opens it with the keyboard up (one
//! tap); the draft is saved on every keystroke so nothing is lost if the app
//! is closed. "Respond" on a note opens it with a typed link to that note.

use aarambh_core::feed::{MAX_NOTE_CHARS, NoteTextError, clean_note_text, default_rel, excerpt};
use aarambh_core::{EventBody, Id, Link, Note, NoteKind, Rel};
use leptos::html;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::util::{local_get, local_remove, local_set};

const DRAFT_KEY: &str = "aarambh.draft";

#[derive(Serialize, Deserialize, Default)]
struct Draft {
    kind: Option<NoteKind>,
    #[serde(default)]
    text: String,
    reply_to: Option<Id>,
    rel: Option<Rel>,
}

#[derive(Clone, Copy)]
pub struct Composer {
    pub open: RwSignal<bool>,
    pub kind: RwSignal<NoteKind>,
    pub text: RwSignal<String>,
    pub reply_to: RwSignal<Option<Id>>,
    pub rel: RwSignal<Rel>,
    sheet: NodeRef<html::Div>,
    input: NodeRef<html::Textarea>,
}

impl Composer {
    pub fn new() -> Self {
        let d: Draft = local_get(DRAFT_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let c = Composer {
            open: RwSignal::new(false),
            kind: RwSignal::new(d.kind.unwrap_or(NoteKind::Note)),
            text: RwSignal::new(d.text),
            reply_to: RwSignal::new(d.reply_to),
            rel: RwSignal::new(d.rel.unwrap_or(Rel::Related)),
            sheet: NodeRef::new(),
            input: NodeRef::new(),
        };
        // Autosave the draft.
        Effect::new(move |_| {
            let d = Draft {
                kind: Some(c.kind.get()),
                text: c.text.get(),
                reply_to: c.reply_to.get(),
                rel: Some(c.rel.get()),
            };
            if d.text.trim().is_empty() && d.reply_to.is_none() {
                local_remove(DRAFT_KEY);
            } else if let Ok(json) = serde_json::to_string(&d) {
                local_set(DRAFT_KEY, &json);
            }
        });
        c
    }

    /// Call from a tap handler: iOS only raises the keyboard for a focus
    /// that happens synchronously inside the user's gesture, so the sheet is
    /// made visible and focused directly rather than on the next render.
    pub fn open_new(self) {
        self.show();
    }

    pub fn respond(self, target: Id, target_kind: NoteKind) {
        if self.reply_to.get_untracked() != Some(target) {
            self.reply_to.set(Some(target));
            self.rel.set(default_rel(target_kind));
        }
        self.show();
    }

    fn show(self) {
        self.open.set(true);
        if let Some(sheet) = self.sheet.get_untracked() {
            let _ = sheet.class_list().add_1("open");
        }
        if let Some(input) = self.input.get_untracked() {
            let _ = input.focus();
        }
    }

    pub fn close(self) {
        self.open.set(false);
        if let Some(input) = self.input.get_untracked() {
            let _ = input.blur();
        }
    }

    fn clear(self) {
        self.text.set(String::new());
        self.reply_to.set(None);
        self.kind.set(NoteKind::Note);
    }
}

/// Tapping a chip shouldn't take focus from the text box: on iOS that would
/// drop the keyboard mid-thought. Keyboard users can still tab to the chips.
fn keep_focus(ev: leptos::ev::MouseEvent) {
    ev.prevent_default();
}

#[component]
pub fn ComposeSheet() -> impl IntoView {
    let c = expect_context::<Composer>();
    let store = expect_context::<Store>();
    let error = RwSignal::new(None::<String>);

    let send = move || match clean_note_text(&c.text.get_untracked()) {
        Ok(text) => {
            let links = c.reply_to.get_untracked().map(|to| {
                vec![Link {
                    to,
                    rel: c.rel.get_untracked(),
                }]
            });
            let note = Note {
                kind: c.kind.get_untracked(),
                text,
                links: links.unwrap_or_default(),
            };
            store.post(store.new_event(EventBody::Note(note)));
            error.set(None);
            c.clear();
            c.close();
        }
        Err(NoteTextError::Empty) => {}
        Err(NoteTextError::TooLong { chars }) => error.set(Some(format!(
            "That's {chars} characters; notes can be up to {MAX_NOTE_CHARS}."
        ))),
    };
    let target = move || {
        let id = c.reply_to.get()?;
        Some(store.log.with(|l| {
            l.get(id)
                .and_then(|e| Some((e.author.clone(), excerpt(&e.as_note()?.text, 90))))
                .unwrap_or_else(|| (String::new(), "an older note".to_owned()))
        }))
    };
    let empty = move || c.text.with(|t| t.trim().is_empty());

    view! {
        <div class="backdrop" class:open=move || c.open.get() on:click=move |_| c.close()></div>
        <div
            class="sheet"
            class:open=move || c.open.get()
            node_ref=c.sheet
            role="dialog"
            aria-modal="true"
            aria-label=move || if c.reply_to.get().is_some() { "Respond" } else { "New note" }
            on:keydown=move |ev| {
                if ev.key() == "Escape" {
                    c.close();
                } else if ev.key() == "Enter" && (ev.meta_key() || ev.ctrl_key()) {
                    ev.prevent_default();
                    send();
                }
            }
        >
            <div class="sheet-head">
                <span class="sheet-title">
                    {move || if c.reply_to.get().is_some() { "Respond" } else { "New note" }}
                </span>
                <button type="button" class="btn quiet" on:click=move |_| c.close()>
                    "Close"
                </button>
            </div>
            {move || target().map(|(author, quote)| view! {
                <div class="replying">
                    <p>
                        <span class="muted">"Responding to "{author}</span>
                        <q>{quote}</q>
                    </p>
                    <button type="button" class="btn quiet small" on:mousedown=keep_focus on:click=move |_| c.reply_to.set(None)>
                        "Unlink"
                    </button>
                </div>
                <p class="group-label" aria-hidden="true">"How it relates"</p>
                <div class="chips" role="group" aria-label="How it relates">
                    {Rel::ALL.map(|r| view! {
                        <button
                            type="button"
                            class="chip"
                            on:mousedown=keep_focus
                            aria-pressed=move || (c.rel.get() == r).to_string()
                            on:click=move |_| c.rel.set(r)
                        >
                            {r.as_str()}
                        </button>
                    })}
                </div>
            })}
            <p class="group-label" aria-hidden="true">"Kind"</p>
            <div class="chips" role="group" aria-label="Kind">
                {NoteKind::ALL.map(|k| view! {
                    <button
                        type="button"
                        class="chip"
                        on:mousedown=keep_focus
                        aria-pressed=move || (c.kind.get() == k).to_string()
                        on:click=move |_| c.kind.set(k)
                    >
                        {k.as_str()}
                    </button>
                })}
            </div>
            <textarea
                class="compose-text"
                node_ref=c.input
                aria-label="Note"
                placeholder="What's on your mind?"
                rows="4"
                prop:value=move || c.text.get()
                on:input:target=move |ev| c.text.set(ev.target().value())
            ></textarea>
            {move || error.get().map(|e| view! { <p class="status-msg error" role="alert">{e}</p> })}
            <div class="sheet-actions">
                <button type="button" class="btn primary" disabled=empty on:click=move |_| send()>
                    "Post"
                </button>
            </div>
        </div>
    }
}

/// Keeps `--kb` (the on-screen keyboard's height) up to date, so the sheet
/// can sit above the keyboard on iOS, where the layout viewport doesn't shrink.
pub fn track_keyboard_inset() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let Some(vv) = window().visual_viewport() else {
        return;
    };
    let update = {
        let vv = vv.clone();
        move || {
            let inner = window()
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let kb = (inner - vv.height() - vv.offset_top()).max(0.0);
            if let Some(root) = document().document_element() {
                let style = root.unchecked_into::<web_sys::HtmlElement>().style();
                let _ = style.set_property("--kb", &format!("{kb:.0}px"));
            }
        }
    };
    let cb = Closure::<dyn Fn()>::new(update);
    for ev in ["resize", "scroll"] {
        let _ = vv.add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref());
    }
    // Lives as long as the page.
    cb.forget();
}
