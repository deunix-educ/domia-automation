// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page de connexion.

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

/// Message d'echec de connexion, traduit a chaud.
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

/// Cible servie hors de l'interface (documentation, Grafana) : vrai chargement de page, et `true`. Le routeur du
/// navigateur ne connait pas ces adresses et afficherait « Page introuvable ».
fn left_the_app(target: &str) -> bool {
    let leaves = leaves_app(target);
    if leaves {
        browser::load_page(target);
    }
    leaves
}

/// Apres une connexion reussie : vers la cible, par le routeur ou par un vrai chargement selon le cas.
fn go_to(target: &str, navigate: &impl Fn(&str, NavigateOptions)) {
    if !left_the_app(target) {
        navigate(target, NavigateOptions::default());
    }
}

/// Visiteur deja connecte sur la page de connexion : renvoi vers la cible.
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

    // Champs « non controles » : on lit leur contenu REEL a la soumission. Le remplissage automatique et la restauration
    // d'un formulaire par le navigateur posent une valeur sans evenement `input` ; un signal alimente par `input` serait
    // alors vide alors que le champ est visiblement rempli (« identifiant incorrect » sans raison).
    let username = NodeRef::<html::Input>::new();
    let password = NodeRef::<html::Input>::new();
    let error = RwSignal::new(None::<LoginError>);
    let pending = RwSignal::new(false);
    let ready = use_hydrated();
    // Arrivee sur la page de connexion : tout depart volontaire (deconnexion...) est termine.
    Effect::new(move |_| state.leaving.set(false));

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        // Sans cela le navigateur enverrait le formulaire lui-meme (page rechargee).
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
                // `method="post"` : si le formulaire etait soumis avant l'hydratation, le mot de passe ne
                // finirait pas dans l'adresse (la page repond alors 405 au lieu de l'afficher).
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
