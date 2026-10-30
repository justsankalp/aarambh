//! Per-device settings: repo, token and the person's name, kept in
//! localStorage. iOS may clear storage, so re-entering them must be quick:
//! the repo is prefilled, the token has a paste button, and names already
//! seen in the notebook are offered as one-tap choices.

use aarambh_core::sync::RepoRef;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen_futures::JsFuture;

use crate::github::Gh;
use crate::theme::ThemePicker;
use crate::util::{local_get, local_remove, local_set};

pub const DEFAULT_REPO: &str = "justsankalp/aarambh";
const TOKEN_HELP: &str = "https://github.com/settings/personal-access-tokens/new";

const K_REPO: &str = "aarambh.repo";
const K_TOKEN: &str = "aarambh.token";
const K_NAME: &str = "aarambh.name";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub repo: String,
    pub token: String,
    pub name: String,
}

impl Settings {
    pub fn load() -> Self {
        Settings {
            repo: local_get(K_REPO).unwrap_or_else(|| DEFAULT_REPO.into()),
            token: local_get(K_TOKEN).unwrap_or_default(),
            name: local_get(K_NAME).unwrap_or_default(),
        }
    }

    fn save(&self) {
        local_set(K_REPO, self.repo.trim());
        local_set(K_TOKEN, self.token.trim());
        local_set(K_NAME, self.name.trim());
    }

    pub fn repo_ref(&self) -> Option<RepoRef> {
        RepoRef::parse(&self.repo)
    }

    pub fn is_complete(&self) -> bool {
        self.repo_ref().is_some() && !self.token.trim().is_empty() && !self.name.trim().is_empty()
    }

    pub fn client(&self) -> Option<Gh> {
        if self.token.trim().is_empty() {
            return None;
        }
        Some(Gh::new(self.repo_ref()?, self.token.trim().to_owned()))
    }
}

#[derive(Clone, PartialEq)]
enum Check {
    Idle,
    Checking,
    Ok(String),
    Failed(String),
}

#[component]
pub fn SettingsView(
    settings: RwSignal<Settings>,
    /// Names seen in the notebook, newest first.
    #[prop(into)]
    known_names: Signal<Vec<String>>,
    #[prop(into)] on_connected: Callback<()>,
    #[prop(into)] on_forget: Callback<()>,
) -> impl IntoView {
    let initial = settings.get_untracked();
    let first_run = !initial.is_complete();
    let repo = RwSignal::new(initial.repo);
    let token = RwSignal::new(initial.token);
    let name = RwSignal::new(initial.name);
    let check = RwSignal::new(Check::Idle);

    let paste_token = move |_| {
        spawn_local(async move {
            let clip = window().navigator().clipboard();
            if let Ok(v) = JsFuture::from(clip.read_text()).await
                && let Some(s) = v.as_string()
            {
                token.set(s.trim().to_owned());
            }
        });
    };

    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let draft = Settings {
            repo: repo.get(),
            token: token.get(),
            name: name.get(),
        };
        let Some(repo_ref) = draft.repo_ref() else {
            check.set(Check::Failed("Repo should look like owner/name.".into()));
            return;
        };
        if draft.token.trim().is_empty() {
            check.set(Check::Failed("Paste your GitHub token.".into()));
            return;
        }
        if draft.name.trim().is_empty() {
            check.set(Check::Failed(
                "Add your name so notes show who wrote them.".into(),
            ));
            return;
        }
        let draft = Settings {
            repo: repo_ref.to_string(),
            ..draft
        };
        check.set(Check::Checking);
        spawn_local(async move {
            let Some(gh) = draft.client() else { return };
            match gh.check_repo().await {
                Ok(info) => {
                    draft.save();
                    repo.set(draft.repo.clone());
                    settings.set(draft);
                    check.set(Check::Ok(format!("Connected to {}.", info.full_name)));
                    on_connected.run(());
                }
                Err(e) => check.set(Check::Failed(e.message())),
            }
        });
    };

    let forget = move |_| {
        local_remove(K_TOKEN);
        token.set(String::new());
        settings.update(|s| s.token.clear());
        check.set(Check::Idle);
        on_forget.run(());
    };

    let token_hint = move || {
        let t = token.get();
        let t = t.trim();
        (!t.is_empty() && !t.starts_with("github_pat_"))
            .then_some("This doesn't look like a fine-grained token (they start with github_pat_).")
    };

    view! {
        <h2>{if first_run { "Connect this device" } else { "Settings" }}</h2>
        <p class="lede">
            {if first_run {
                "Notes sync through your GitHub repo. This device needs the repo, a token and your name."
            } else {
                "Stored on this device only."
            }}
        </p>
        <form class="card" on:submit=save novalidate>
            <label class="field">
                <span>"Repository"</span>
                <input
                    type="text"
                    name="repo"
                    bind:value=repo
                    autocapitalize="off"
                    spellcheck="false"
                    placeholder="owner/name"
                />
            </label>
            <label class="field">
                <span>"GitHub token"</span>
                <div class="input-row">
                    <input
                        type="password"
                        name="token"
                        bind:value=token
                        autocomplete="off"
                        autocapitalize="off"
                        spellcheck="false"
                        placeholder="github_pat_…"
                    />
                    <button type="button" class="btn" on:click=paste_token>
                        "Paste"
                    </button>
                </div>
                <small>
                    "Fine-grained, this repo only, with Contents and Actions read/write. "
                    <a href=TOKEN_HELP target="_blank" rel="noopener noreferrer">
                        "Create one"
                    </a>
                </small>
                <Show when=move || token_hint().is_some()>
                    <small>{token_hint}</small>
                </Show>
            </label>
            <label class="field">
                <span>"Your name"</span>
                <input type="text" name="name" bind:value=name autocomplete="nickname" maxlength="40" />
                <Show when=move || !known_names.get().is_empty()>
                    <div class="chips" role="group" aria-label="Names in this notebook">
                        {move || known_names.get().into_iter().map(|n| {
                            let pick = n.clone();
                            view! {
                                <button type="button" class="chip" on:click=move |_| name.set(pick.clone())>
                                    {n}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </Show>
            </label>
            <div class="actions">
                <button type="submit" class="btn primary" disabled=move || check.get() == Check::Checking>
                    {move || if check.get() == Check::Checking { "Checking…" } else { "Save and connect" }}
                </button>
            </div>
            {move || match check.get() {
                Check::Ok(msg) => view! { <p class="status-msg ok" role="status">{msg}</p> }.into_any(),
                Check::Failed(msg) => view! { <p class="status-msg error" role="alert">{msg}</p> }.into_any(),
                _ => ().into_any(),
            }}
        </form>

        <hr class="soft" />
        <div class="field">
            <span>"Theme"</span>
            <ThemePicker />
        </div>

        <Show when=move || !first_run>
            <hr class="soft" />
            <div class="field">
                <span>"This device"</span>
                <small>"Removes the token and cached notes from this device. Your notes stay in the repo."</small>
                <div class="actions">
                    <button type="button" class="btn danger" on:click=forget>
                        "Forget this device"
                    </button>
                </div>
            </div>
        </Show>
    }
}
