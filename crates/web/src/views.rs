//! Bits shared by several tabs.

use leptos::prelude::*;

use crate::store::{Store, SyncState};
use crate::util::{ago, now_ms};

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
        let pending = store.pending();
        let waiting = (pending > 0).then(|| format!(" · {pending} waiting to send"));
        let base = match store.state.get() {
            SyncState::Starting | SyncState::Syncing => "Syncing…".to_owned(),
            SyncState::NeedsSetup => String::new(),
            SyncState::Synced { at_ms } => format!("Synced {}", ago(at_ms, now.max(at_ms))),
            SyncState::Offline => "Offline. Notes will send when you're back.".to_owned(),
            SyncState::Failed { message, .. } => message,
        };
        format!("{base}{}", waiting.unwrap_or_default())
    };
    let is_error = move || matches!(store.state.get(), SyncState::Failed { .. });
    let fix_in_settings =
        move || matches!(store.state.get(), SyncState::Failed { settings: true, .. });
    let skipped = move || store.skipped.get();
    let has_skipped = move || skipped() > 0;
    view! {
        <p class="sync-line" class:error=is_error role="status" aria-live="polite">
            {text}
            <Show when=fix_in_settings>
                " " <a href="#settings">"Open settings"</a>
            </Show>
        </p>
        <Show when=has_skipped>
            <p class="notice" role="status">
                {move || match skipped() {
                    1 => "1 entry was written by a newer version of aarambh and isn't shown. Reload to update.".to_owned(),
                    n => format!("{n} entries were written by a newer version of aarambh and aren't shown. Reload to update."),
                }}
            </p>
        </Show>
    }
}
