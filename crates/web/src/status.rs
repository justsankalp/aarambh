//! Changing an idea's status. Dropping or deciding an idea offers a quick,
//! skippable post-mortem: which failure modes applied, why, what would
//! change your mind, and what you could have checked earlier.

use aarambh_core::feed::excerpt;
use aarambh_core::{EventBody, Id, IdeaStatus, Lesson, StatusChange};
use leptos::prelude::*;

use crate::store::Store;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Choose,
    Lesson(IdeaStatus),
}

#[derive(Clone, Copy)]
pub struct StatusSheet {
    idea: RwSignal<Option<Id>>,
    step: RwSignal<Step>,
}

impl StatusSheet {
    pub fn new() -> Self {
        StatusSheet {
            idea: RwSignal::new(None),
            step: RwSignal::new(Step::Choose),
        }
    }

    pub fn open(self, idea: Id) {
        self.step.set(Step::Choose);
        self.idea.set(Some(idea));
    }

    fn close(self) {
        self.idea.set(None);
    }
}

pub fn status_label(s: IdeaStatus) -> &'static str {
    match s {
        IdeaStatus::Active => "active",
        IdeaStatus::Parked => "parked",
        IdeaStatus::Dropped => "dropped",
        IdeaStatus::Decided => "decided",
    }
}

const CHOICES: [(IdeaStatus, &str); 4] = [
    (IdeaStatus::Active, "Still exploring it"),
    (IdeaStatus::Parked, "Not now, maybe later"),
    (IdeaStatus::Dropped, "Not pursuing it"),
    (IdeaStatus::Decided, "Committed to it"),
];

#[component]
pub fn StatusSheetView() -> impl IntoView {
    let sheet = expect_context::<StatusSheet>();
    let store = expect_context::<Store>();
    let open = move || sheet.idea.get().is_some();

    let post = move |status: IdeaStatus, lesson: Option<Lesson>| {
        if let Some(idea) = sheet.idea.get_untracked() {
            store.post(store.new_event(EventBody::Status(StatusChange {
                idea,
                status,
                lesson,
            })));
        }
        sheet.close();
    };
    let choose = move |status: IdeaStatus| {
        let current = sheet
            .idea
            .get_untracked()
            .map(|id| store.log.with_untracked(|l| l.status_of(id)));
        if current == Some(status) {
            sheet.close();
        } else if status.prompts_lesson() {
            sheet.step.set(Step::Lesson(status));
        } else {
            post(status, None);
        }
    };
    let idea_text = move || {
        let id = sheet.idea.get()?;
        store
            .log
            .with(|l| Some(excerpt(&l.get(id)?.as_note()?.text, 90)))
    };

    view! {
        <div class="backdrop" class:open=open on:click=move |_| sheet.close()></div>
        <div
            class="sheet"
            class:open=open
            role="dialog"
            aria-modal="true"
            aria-label="Idea status"
            on:keydown=move |ev| {
                if ev.key() == "Escape" {
                    sheet.close();
                }
            }
        >
            <div class="sheet-head">
                <span class="sheet-title">
                    {move || match sheet.step.get() {
                        Step::Choose => "Where does this idea stand?",
                        Step::Lesson(_) => "Quick post-mortem",
                    }}
                </span>
                <button type="button" class="btn quiet" on:click=move |_| sheet.close()>
                    "Close"
                </button>
            </div>
            {move || idea_text().map(|t| view! { <p class="replying"><q>{t}</q></p> })}
            {move || match sheet.step.get() {
                Step::Choose => {
                    let current = sheet.idea.get().map(|id| store.log.with(|l| l.status_of(id)));
                    view! {
                        <div class="choices" role="group" aria-label="Status">
                            {CHOICES.map(|(s, hint)| view! {
                                <button
                                    type="button"
                                    class="choice"
                                    aria-pressed=(current == Some(s)).to_string()
                                    on:click=move |_| choose(s)
                                >
                                    <span class="choice-label">{status_label(s)}</span>
                                    <span class="muted">{hint}</span>
                                </button>
                            })}
                        </div>
                    }
                    .into_any()
                }
                Step::Lesson(status) => view! { <LessonForm status=status on_done=post /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn LessonForm(
    status: IdeaStatus,
    on_done: impl Fn(IdeaStatus, Option<Lesson>) + Copy + 'static,
) -> impl IntoView {
    let store = expect_context::<Store>();
    let modes = RwSignal::new(Vec::<String>::new());
    let why = RwSignal::new(String::new());
    let change = RwSignal::new(String::new());
    let earlier = RwSignal::new(String::new());
    let save = move |_| {
        let lesson = Lesson {
            modes: modes.get_untracked(),
            why: why.get_untracked().trim().to_owned(),
            would_change_mind: change.get_untracked().trim().to_owned(),
            check_earlier: earlier.get_untracked().trim().to_owned(),
        };
        let empty = lesson == Lesson::default();
        on_done(status, (!empty).then_some(lesson));
    };
    let toggle = move |id: String| {
        modes.update(|m| {
            if let Some(i) = m.iter().position(|x| *x == id) {
                m.remove(i);
            } else {
                m.push(id);
            }
        })
    };
    view! {
        <p class="muted small-print">"Marking it " {status_label(status)} ". All of this is optional."</p>
        <div class="field">
            <span>"Which of these applied?"</span>
            <div class="chips" role="group" aria-label="Failure modes">
                {move || store.taxonomy.with(|t| {
                    t.modes
                        .iter()
                        .map(|m| {
                            let id = m.id.clone();
                            let pressed = {
                                let id = id.clone();
                                move || modes.with(|ms| ms.contains(&id)).to_string()
                            };
                            view! {
                                <button
                                    type="button"
                                    class="chip"
                                    title=m.hint.clone()
                                    aria-pressed=pressed
                                    on:click=move |_| toggle(id.clone())
                                >
                                    {m.label.clone()}
                                </button>
                            }
                        })
                        .collect_view()
                })}
            </div>
        </div>
        <label class="field">
            <span>"Why?"</span>
            <textarea rows="2" prop:value=move || why.get() on:input:target=move |ev| why.set(ev.target().value())></textarea>
        </label>
        <label class="field">
            <span>"What would change your mind?"</span>
            <textarea rows="2" prop:value=move || change.get() on:input:target=move |ev| change.set(ev.target().value())></textarea>
        </label>
        <label class="field">
            <span>"What could you have checked earlier?"</span>
            <textarea rows="2" prop:value=move || earlier.get() on:input:target=move |ev| earlier.set(ev.target().value())></textarea>
        </label>
        <div class="sheet-actions">
            <button type="button" class="btn quiet" on:click=move |_| on_done(status, None)>
                "Skip"
            </button>
            <button type="button" class="btn primary" on:click=save>
                "Save"
            </button>
        </div>
    }
}
