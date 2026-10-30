mod app;
mod cache;
mod compose;
mod feed;
mod github;
mod settings;
mod status;
mod store;
mod theme;
mod util;
mod views;

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    if let Some(boot) = document().get_element_by_id("boot") {
        boot.remove();
    }
    mount_to_body(app::App);
}
