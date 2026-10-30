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

fn local_date(ms: u64) -> js_sys::Date {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64))
}

/// Local calendar day as (year, month 1-12, day), for grouping the feed.
pub fn local_ymd(ms: u64) -> (u32, u32, u32) {
    let d = local_date(ms);
    (d.get_full_year(), d.get_month() + 1, d.get_date())
}

/// "Today", "Yesterday", or e.g. "Thu 29 Oct" (with the year if not this year).
pub fn day_label(ms: u64, now: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let day = local_ymd(ms);
    if day == local_ymd(now) {
        return "Today".into();
    }
    if day == local_ymd(now.saturating_sub(86_400_000)) {
        return "Yesterday".into();
    }
    let d = local_date(ms);
    let mut s = format!(
        "{} {} {}",
        DAYS[d.get_day() as usize % 7],
        d.get_date(),
        MONTHS[d.get_month() as usize % 12]
    );
    if day.0 != local_ymd(now).0 {
        s.push_str(&format!(" {}", day.0));
    }
    s
}

/// Local time, "09:41".
pub fn clock(ms: u64) -> String {
    let d = local_date(ms);
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

/// "just now", "5 min ago", "2 h ago".
pub fn ago(ms: u64, now: u64) -> String {
    let mins = now.saturating_sub(ms) / 60_000;
    match mins {
        0 => "just now".into(),
        1..=59 => format!("{mins} min ago"),
        _ => format!("{} h ago", mins / 60),
    }
}
