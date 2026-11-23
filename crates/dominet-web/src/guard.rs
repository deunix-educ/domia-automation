// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page protection: an anonymous visitor is redirected to the login page.
//!
//! On the server side the redirect is a real HTTP 302 response (`Async` rendering), so the protected page is never
//! sent; in the browser (internal navigation) it is a route change. The server re-checks anyway
//! every server function call: this guard is for display, not for security.

use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::{A, Redirect};
use leptos_router::hooks::use_location;

use crate::layout::{AuthStatus, LayoutState, status_of};
use crate::nav::login_url;

/// Only displays `children` for a logged-in account.
#[component]
pub fn RequireAuth(children: ChildrenFn) -> impl IntoView {
    view! {
        <Transition fallback=|| ()>
            <Gate admin=false children=children.clone()/>
        </Transition>
    }
}

/// Only displays `children` for an administrator: an anonymous visitor is sent to the login, an ordinary account
/// receives the "access denied" page (403 status on the server side).
#[component]
pub fn RequireAdmin(children: ChildrenFn) -> impl IntoView {
    view! {
        <Transition fallback=|| ()>
            <Gate admin=true children=children.clone()/>
        </Transition>
    }
}

/// Decides what to display according to the visitor's *status*.
///
/// Two constraints. (1) The resource must be read **under** the `Transition`: otherwise the server render does not wait for it.
/// (2) A reload of the navigation (menu modification for example) must not recreate the displayed
/// page: the component is created only once per page and only reacts to a change of *status*, thanks to the
/// memo (which only notifies if the value changes).
#[component]
fn Gate(admin: bool, children: ChildrenFn) -> impl IntoView {
    let state = expect_context::<LayoutState>();
    let location = use_location();
    // During a reload the resource keeps its last value; with no value at all we keep the last status.
    let status = Memo::new(
        move |prev: Option<&Option<AuthStatus>>| match state.data.get() {
            Some(d) => Some(status_of(&d)),
            None => prev.copied().flatten(),
        },
    );
    move || match status.get() {
        // No response from the server yet.
        None => ().into_any(),
        Some(AuthStatus::Anonymous) => {
            if state.leaving.get_untracked() {
                // Voluntary departure (logout...): the navigation is already under way.
                return ().into_any();
            }
            let search = location.search.get_untracked();
            let target = if search.is_empty() {
                location.pathname.get_untracked()
            } else {
                format!("{}?{search}", location.pathname.get_untracked())
            };
            view! { <Redirect path=login_url(&target)/> }.into_any()
        }
        Some(AuthStatus::User) if admin => view! { <Forbidden/> }.into_any(),
        Some(AuthStatus::User | AuthStatus::Admin) => children().into_any(),
    }
}

#[component]
pub fn Forbidden() -> impl IntoView {
    // The HTTP status only exists on the server side.
    #[cfg(feature = "ssr")]
    {
        let response = expect_context::<leptos_axum::ResponseOptions>();
        response.set_status(axum::http::StatusCode::FORBIDDEN);
    }
    view! {
        <main class="vc-auth-wrap">
            <div class="vc-auth-card" id="forbidden">
                <h1>{move_tr!("forbidden-title")}</h1>
                <p>{move_tr!("forbidden-text")}</p>
                <A href="/">{move_tr!("not-found-back")}</A>
            </div>
        </main>
    }
}
