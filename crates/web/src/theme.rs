//! Theme preference. The inline script in index.html does the actual work
//! (before first paint, and when the system theme changes); this module
//! stores the choice and asks the script to re-apply it.

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

const KEY: &str = "aarambh.theme";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref {
    Auto,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::Auto, ThemePref::Light, ThemePref::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::Auto => "Auto",
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
        }
    }

    pub fn load() -> Self {
        match crate::util::local_get(KEY).as_deref() {
            Some("light") => ThemePref::Light,
            Some("dark") => ThemePref::Dark,
            _ => ThemePref::Auto,
        }
    }

    pub fn save(self) {
        match self {
            ThemePref::Auto => crate::util::local_remove(KEY),
            ThemePref::Light => crate::util::local_set(KEY, "light"),
            ThemePref::Dark => crate::util::local_set(KEY, "dark"),
        }
        let apply = js_sys::Reflect::get(&window(), &JsValue::from_str("aarambhApplyTheme"));
        if let Ok(f) = apply.and_then(|f| f.dyn_into::<js_sys::Function>()) {
            let _ = f.call0(&JsValue::NULL);
        }
    }
}

#[component]
pub fn ThemePicker() -> impl IntoView {
    let pref = RwSignal::new(ThemePref::load());
    view! {
        <div class="segmented" role="group" aria-label="Theme">
            {ThemePref::ALL
                .map(|p| {
                    view! {
                        <button
                            type="button"
                            aria-pressed=move || (pref.get() == p).to_string()
                            on:click=move |_| {
                                p.save();
                                pref.set(p);
                            }
                        >
                            {p.label()}
                        </button>
                    }
                })}
        </div>
    }
}
