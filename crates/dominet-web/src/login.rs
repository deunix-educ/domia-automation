// SPDX-License-Identifier: AGPL-3.0-or-later
//! Login page.

use dominet_proto::LoginOutcome;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{move_tr, tr};
use leptos_router::NavigateOptions;
use leptos_router::components::Redirect;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::layout::{LanguageSwitcher, LayoutRefresh, LayoutState, use_hydrated};
use crate::nav::{leaves_app, safe_next};
use crate::server_fns::login;
use crate::{browser, chrome};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoginError {
    Invalid,
    Locked(u64),
    Generic,
}

/// Login failure message, translated live.
#[component]
fn ErrorAlert(error: RwSignal<Option<LoginError>>) -> impl IntoView {
    move || {
        error.get().map(|e| {
            view! {
                <div class="vc-alert vc-alert--error" role="alert" id="login-error">
                    {move || match e {
                        LoginError::Invalid => tr!("login-error-invalid"),
                        LoginError::Locked(s) => {
                            tr!("login-error-locked", {"minutes" => s.div_ceil(60).max(1)})
                        }
                        LoginError::Generic => tr!("error-generic"),
                    }}
                </div>
            }
        })
    }
}

/// Target served outside the interface (documentation, Grafana): a real page load, and `true`. The browser's router
/// does not know these addresses and would display "Page not found".
fn left_the_app(target: &str) -> bool {
    let leaves = leaves_app(target);
    if leaves {
        browser::load_page(target);
    }
    leaves
}

/// After a successful login: to the target, through the router or a real load as the case may be.
fn go_to(target: &str, navigate: &impl Fn(&str, NavigateOptions)) {
    if !left_the_app(target) {
        navigate(target, NavigateOptions::default());
    }
}

/// Visitor already logged in on the login page: sent on to the target.
fn already_signed_in(signed_in: bool, target: String) -> Option<impl IntoView> {
    if !signed_in || left_the_app(&target) {
        None
    } else {
        Some(view! { <Redirect path=target/> })
    }
}

#[component]
pub fn LoginPage() -> impl IntoView {
    let state = expect_context::<LayoutState>();
    let refresh = expect_context::<LayoutRefresh>();
    let navigate = use_navigate();
    let query = use_query_map();
    let next = move || safe_next(&query.read().get("next").unwrap_or_default());
    let notice = move || query.read().get("notice").as_deref() == Some("password-changed");

    // "Uncontrolled" fields: we read their REAL content on submission. Autofill and the browser's restoration
    // of a form set a value without an `input` event; a signal fed by `input` would then be
    // empty while the field is visibly filled ("wrong identifier" for no reason).
    let username = NodeRef::<html::Input>::new();
    let password = NodeRef::<html::Input>::new();
    let error = RwSignal::new(None::<LoginError>);
    let pending = RwSignal::new(false);
    let ready = use_hydrated();
    // Arrival on the login page: any voluntary departure (logout...) is over.
    Effect::new(move |_| state.leaving.set(false));

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        // Without this the browser would submit the form itself (page reloaded).
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);
        let (u, p) = (
            browser::input_value(username),
            browser::input_value(password),
        );
        let navigate = navigate.clone();
        spawn_local(async move {
            match login(u, p).await {
                Ok(LoginOutcome::Success(_)) => {
                    refresh.0.update(|n| *n += 1);
                    go_to(&next(), &navigate);
                }
                Ok(LoginOutcome::InvalidCredentials) => error.set(Some(LoginError::Invalid)),
                Ok(LoginOutcome::Locked { retry_after_s }) => {
                    error.set(Some(LoginError::Locked(retry_after_s)));
                }
                Err(_) => error.set(Some(LoginError::Generic)),
            }
            browser::clear_input(password);
            pending.set(false);
        });
    };

    view! {
        <Transition fallback=|| ()>
            {move || {
                let signed_in = state.data.get().is_some_and(|d| d.user.is_some());
                already_signed_in(signed_in, next())
            }}
        </Transition>
        <main class="vc-auth-wrap">
            <div class="vc-auth-card">
                <div class="vc-auth-header">
                    <div class="vc-auth-icon" inner_html=chrome::LOGIN></div>
                    <h1>"DomiNet"</h1>
                    <p>{move_tr!("login-subtitle")}</p>
                </div>
                <Show when=notice>
                    <div class="vc-alert vc-alert--info" role="status" id="login-notice">
                        {move_tr!("login-notice-password-changed")}
                    </div>
                </Show>
                // `method="post"`: if the form were submitted before hydration, the password would not
                // end up in the address (the page then answers 405 instead of displaying it).
                <form method="post" on:submit=on_submit>
                    <ErrorAlert error/>
                    <div class="vc-form-group">
                        <label for="login-username">{move_tr!("login-username")}</label>
                        <input
                            id="login-username"
                            name="username"
                            type="text"
                            autocomplete="username"
                            autofocus
                            required
                            maxlength="150"
                            node_ref=username
                        />
                    </div>
                    <div class="vc-form-group">
                        <label for="login-password">{move_tr!("login-password")}</label>
                        <input
                            id="login-password"
                            name="password"
                            type="password"
                            autocomplete="current-password"
                            required
                            node_ref=password
                        />
                    </div>
                    <button
                        type="submit"
                        id="login-submit"
                        class="vc-btn vc-btn--primary vc-btn--block"
                        disabled=move || pending.get() || !ready.get()
                    >
                        {move_tr!("login-submit")}
                    </button>
                </form>
                <LanguageSwitcher/>
            </div>
        </main>
    }
}
