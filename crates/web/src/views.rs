//! Tab screens. Notes, Map and Digest are filled in by later phases.

use leptos::prelude::*;

#[component]
pub fn NotesView() -> impl IntoView {
    view! {
        <div class="empty">
            <p>"No notes yet."</p>
        </div>
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
