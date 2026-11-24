// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page "My account": identity and password change.

use dominet_proto::{ChangePasswordOutcome, PasswordProblem, RoleDto, UserDto};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::browser;
use crate::guard::RequireAuth;
use crate::layout::{LayoutRefresh, LayoutState, leave_session, use_hydrated};
use crate::server_fns::change_password;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormError {
    WrongCurrent,
    Locked(u64),
    Mismatch,
    Problem(PasswordProblem),
    Generic,
}

pub(crate) fn problem_text(p: PasswordProblem) -> String {
    match p {
        PasswordProblem::TooShort { min } => tr!("pw-too-short", {"min" => min}),
        PasswordProblem::TooLong { max } => tr!("pw-too-long", {"max" => max}),
        PasswordProblem::AllDigits => tr!("pw-all-digits"),
        PasswordProblem::SameAsUsername => tr!("pw-same-as-username"),
    }
}

/// `2026-11-01T11:32:05+01:00` (server local time) -> `2026-11-01 11:32`.
pub(crate) fn format_instant(rfc3339: &str) -> String {
    let date = rfc3339.get(..10).unwrap_or(rfc3339);
    match rfc3339.get(11..16) {
        Some(time) => format!("{date} {time}"),
        None => date.to_string(),
    }
}

#[component]
pub fn AccountPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <h1 class="vc-page-title">{move_tr!("account-title")}</h1>
                <Identity/>
                <PasswordForm/>
                <section class="vc-card">
                    <h2 class="vc-card-title">{move_tr!("camaccess-title")}</h2>
                    <p class="vc-muted">{move_tr!("account-camera-access-hint")}</p>
                    <p><A href="/account/cameras" attr:id="account-camera-access">{move_tr!("account-camera-access-link")}</A></p>
                </section>
            </main>
        </RequireAuth>
    }
}

#[component]
fn Identity() -> impl IntoView {
    let state = expect_context::<LayoutState>();
    view! {
        <section class="vc-card">
            <h2 class="vc-card-title">{move_tr!("account-identity")}</h2>
            <Transition fallback=|| ()>
                {move || state.data.get().and_then(|d| d.user).map(|u| identity_list(&u))}
            </Transition>
        </section>
    }
}

fn identity_list(u: &UserDto) -> impl IntoView + use<> {
    let role = u.role;
    let (name, email) = (u.display_name.clone(), u.email.clone());
    let username = u.username.clone();
    let last = u.last_login_at.as_deref().map(format_instant);
    view! {
        <dl class="vc-def-list" id="account-identity">
            <dt>{move_tr!("account-name")}</dt>
            <dd>{name}</dd>
            <dt>{move_tr!("login-username")}</dt>
            <dd>{username}</dd>
            <dt>{move_tr!("account-email")}</dt>
            <dd>{if email.is_empty() { "-".to_string() } else { email }}</dd>
            <dt>{move_tr!("account-role")}</dt>
            <dd>
                {move || match role {
                    RoleDto::Admin => tr!("role-admin"),
                    RoleDto::User => tr!("role-user"),
                }}
            </dd>
            <dt>{move_tr!("account-last-login")}</dt>
            <dd>{move || last.clone().unwrap_or_else(|| tr!("account-never"))}</dd>
        </dl>
    }
}

#[component]
fn PasswordForm() -> impl IntoView {
    let refresh = expect_context::<LayoutRefresh>();
    let state = expect_context::<LayoutState>();
    let navigate = use_navigate();
    // Uncontrolled fields: see `login.rs` (autofill without an `input` event).
    let current = NodeRef::<html::Input>::new();
    let new = NodeRef::<html::Input>::new();
    let confirm = NodeRef::<html::Input>::new();
    let error = RwSignal::new(None::<FormError>);
    let pending = RwSignal::new(false);
    let ready = use_hydrated();

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let (c, n) = (browser::input_value(current), browser::input_value(new));
        if n != browser::input_value(confirm) {
            error.set(Some(FormError::Mismatch));
            return;
        }
        pending.set(true);
        error.set(None);
        let navigate = navigate.clone();
        spawn_local(async move {
            match change_password(c, n).await {
                Ok(ChangePasswordOutcome::Changed) => {
                    // All the account's sessions are closed: we start over from the login page.
                    leave_session(navigate, "/login?notice=password-changed", state, refresh);
                }
                Ok(ChangePasswordOutcome::WrongCurrent) => error.set(Some(FormError::WrongCurrent)),
                Ok(ChangePasswordOutcome::Locked { retry_after_s }) => {
                    error.set(Some(FormError::Locked(retry_after_s)));
                }
                Ok(ChangePasswordOutcome::Rejected(p)) => error.set(Some(FormError::Problem(p))),
                Err(_) => error.set(Some(FormError::Generic)),
            }
            browser::clear_input(current);
            pending.set(false);
        });
    };

    view! {
        <section class="vc-card">
            <h2 class="vc-card-title">{move_tr!("account-password-title")}</h2>
            <p class="vc-muted">{move_tr!("account-password-hint")}</p>
            <form method="post" on:submit=on_submit id="password-form">
                {move || {
                    error
                        .get()
                        .map(|e| {
                            view! {
                                <div class="vc-alert vc-alert--error" role="alert" id="password-error">
                                    {move || match e {
                                        FormError::WrongCurrent => tr!("account-error-current"),
                                        FormError::Locked(s) => {
                                            tr!("login-error-locked", {"minutes" => s.div_ceil(60).max(1)})
                                        }
                                        FormError::Mismatch => tr!("account-error-mismatch"),
                                        FormError::Problem(p) => problem_text(p),
                                        FormError::Generic => tr!("error-generic"),
                                    }}
                                </div>
                            }
                        })
                }}
                <div class="vc-form-group">
                    <label for="pw-current">{move_tr!("account-current")}</label>
                    <input
                        id="pw-current"
                        type="password"
                        autocomplete="current-password"
                        required
                        node_ref=current
                    />
                </div>
                <div class="vc-form-group">
                    <label for="pw-new">{move_tr!("account-new")}</label>
                    <input
                        id="pw-new"
                        type="password"
                        autocomplete="new-password"
                        required
                        node_ref=new
                    />
                </div>
                <div class="vc-form-group">
                    <label for="pw-confirm">{move_tr!("account-confirm")}</label>
                    <input
                        id="pw-confirm"
                        type="password"
                        autocomplete="new-password"
                        required
                        node_ref=confirm
                    />
                </div>
                <div class="vc-form-actions">
                    <button
                        type="submit"
                        id="pw-submit"
                        class="vc-btn vc-btn--primary"
                        disabled=move || pending.get() || !ready.get()
                    >
                        {move_tr!("account-submit")}
                    </button>
                </div>
            </form>
        </section>
    }
}
