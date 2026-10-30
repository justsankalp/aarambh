//! App frame: header, the three tabs and settings, routed by the URL hash
//! so the back button works and the shell stays a single static page.

use leptos::ev;
use leptos::prelude::*;

use crate::settings::{Settings, SettingsView};
use crate::store::Store;
use crate::util::location_hash;
use crate::views::{DigestView, MapView, NotesView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Notes,
    Map,
    Digest,
    Settings,
}

impl Tab {
    const MAIN: [Tab; 3] = [Tab::Notes, Tab::Map, Tab::Digest];

    fn from_hash(hash: &str) -> Tab {
        match hash.trim_start_matches('#') {
            "map" => Tab::Map,
            "digest" => Tab::Digest,
            "settings" => Tab::Settings,
            _ => Tab::Notes,
        }
    }

    fn href(self) -> &'static str {
        match self {
            Tab::Notes => "#notes",
            Tab::Map => "#map",
            Tab::Digest => "#digest",
            Tab::Settings => "#settings",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Notes => "Notes",
            Tab::Map => "Map",
            Tab::Digest => "Digest",
            Tab::Settings => "Settings",
        }
    }
}

fn current(tab: RwSignal<Tab>, t: Tab) -> impl Fn() -> Option<&'static str> {
    move || (tab.get() == t).then_some("page")
}

#[component]
pub fn App() -> impl IntoView {
    let settings = RwSignal::new(Settings::load());
    let store = Store::new(settings);
    provide_context(store);
    store.start();

    // First run: go straight to settings.
    if !settings.get_untracked().is_complete() && location_hash().is_empty() {
        let _ = window().location().set_hash("settings");
    }
    let tab = RwSignal::new(Tab::from_hash(&location_hash()));
    let listeners = [
        window_event_listener(ev::hashchange, move |_| {
            tab.set(Tab::from_hash(&location_hash()))
        }),
        window_event_listener(ev::online, move |_| store.sync_soon()),
        window_event_listener(ev::focus, move |_| store.sync_soon()),
    ];
    // Poll gently while the app is open; an unchanged listing costs a 304.
    let poll = set_interval_with_handle(
        move || {
            if document().visibility_state() == web_sys::VisibilityState::Visible {
                store.sync_soon();
            }
        },
        std::time::Duration::from_secs(90),
    )
    .ok();
    on_cleanup(move || {
        listeners.into_iter().for_each(|l| l.remove());
        if let Some(p) = poll {
            p.clear();
        }
    });

    view! {
        <header class="top">
            <h1 class="brand">"aarambh"</h1>
            <a class="icon-btn" href=Tab::Settings.href() aria-label="Settings" aria-current=current(tab, Tab::Settings)>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
                    <circle cx="12" cy="12" r="3" />
                    <path d="M12 2.8v2.4M12 18.8v2.4M2.8 12h2.4M18.8 12h2.4M5.5 5.5l1.7 1.7M16.8 16.8l1.7 1.7M5.5 18.5l1.7-1.7M16.8 7.2l1.7-1.7" />
                </svg>
            </a>
        </header>
        <main class="screen" id="main">
            {move || match tab.get() {
                Tab::Notes => view! { <NotesView /> }.into_any(),
                Tab::Map => view! { <MapView /> }.into_any(),
                Tab::Digest => view! { <DigestView /> }.into_any(),
                Tab::Settings => view! {
                    <SettingsView
                        settings=settings
                        known_names=store.known_names()
                        on_connected=move |()| store.sync_soon()
                        on_forget=move |()| leptos::task::spawn_local(store.reset_cache(false))
                    />
                }.into_any(),
            }}
        </main>
        <nav class="tabs" aria-label="Sections">
            {Tab::MAIN.map(|t| view! {
                <a class="tab" href=t.href() aria-current=current(tab, t)>{t.label()}</a>
            })}
        </nav>
    }
}
