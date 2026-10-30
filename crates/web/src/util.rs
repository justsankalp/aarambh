//! Small browser helpers.

use leptos::prelude::window;

fn local() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

/// Reads localStorage. Storage can be unavailable (private mode, cleared by
/// iOS), so every access is fallible and treated as "not set".
pub fn local_get(key: &str) -> Option<String> {
    local()?.get_item(key).ok().flatten()
}

pub fn local_set(key: &str, value: &str) {
    if let Some(s) = local() {
        let _ = s.set_item(key, value);
    }
}

pub fn local_remove(key: &str) {
    if let Some(s) = local() {
        let _ = s.remove_item(key);
    }
}

pub fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

/// 80+ random bits for IDs, from `crypto.getRandomValues`.
pub fn random_u128() -> u128 {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("crypto.getRandomValues is available");
    u128::from_le_bytes(buf)
}

pub fn location_hash() -> String {
    window().location().hash().unwrap_or_default()
}
