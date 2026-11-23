// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser access (local storage, full screen). No effect on the server side: the server render has no browser.

/// Storage key of the sidebar's collapsed state (the same as the reference interface).
const COLLAPSED_KEY: &str = "vc-sidebar-collapsed";

#[cfg(feature = "hydrate")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Remembered collapsed state, if it exists.
#[must_use]
pub fn stored_collapsed() -> Option<bool> {
    #[cfg(feature = "hydrate")]
    {
        storage()?.get_item(COLLAPSED_KEY).ok()?.map(|v| v == "1")
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = COLLAPSED_KEY;
        None
    }
}

/// Remembers the collapsed state (ignores errors: blocked storage, private browsing...).
pub fn store_collapsed(collapsed: bool) {
    #[cfg(feature = "hydrate")]
    if let Some(s) = storage() {
        let _ = s.set_item(COLLAPSED_KEY, if collapsed { "1" } else { "0" });
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = collapsed;
}

/// Scrolls back to the top of the page (opening an edit form from a long list: the form is at the top).
pub fn scroll_to_top() {
    #[cfg(feature = "hydrate")]
    if let Some(w) = web_sys::window() {
        w.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

/// Loads `path` with a real request to the server (page outside the interface: documentation, Grafana).
pub fn load_page(path: &str) {
    #[cfg(feature = "hydrate")]
    if let Some(w) = web_sys::window() {
        let _ = w.location().assign(path);
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = path;
}

/// Toggles full screen.
pub fn toggle_fullscreen() {
    #[cfg(feature = "hydrate")]
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else if let Some(root) = doc.document_element() {
            let _ = root.request_fullscreen();
        }
    }
}

/// REAL content of a field (empty if not yet mounted). To be preferred to a signal fed by `input` for the
/// login forms: autofill and the browser's restoration of a form set
/// a value without an `input` event.
#[must_use]
pub fn input_value(field: leptos::prelude::NodeRef<leptos::html::Input>) -> String {
    use leptos::prelude::GetUntracked;
    field.get_untracked().map(|e| e.value()).unwrap_or_default()
}

/// Clears a field (after a failure, so as not to leave a password on screen).
pub fn clear_input(field: leptos::prelude::NodeRef<leptos::html::Input>) {
    use leptos::prelude::GetUntracked;
    if let Some(e) = field.get_untracked() {
        e.set_value("");
    }
}
