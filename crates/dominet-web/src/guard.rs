// SPDX-License-Identifier: AGPL-3.0-or-later
//! Protection des pages : un visiteur anonyme est redirige vers la page de connexion.
//!
//! Cote serveur la redirection est une vraie reponse HTTP 302 (rendu `Async`), donc la page protegee n'est jamais
//! envoyee ; cote navigateur (navigation interne) c'est un changement de route. Le serveur reverifie de toute facon
//! chaque appel de fonction serveur : cette garde sert a l'affichage, pas a la securite.

use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::{A, Redirect};
use leptos_router::hooks::use_location;

use crate::layout::{AuthStatus, LayoutState, status_of};
use crate::nav::login_url;

/// N'affiche `children` que pour un compte connecte.
#[component]
pub fn RequireAuth(children: ChildrenFn) -> impl IntoView {
    view! {
        <Transition fallback=|| ()>
            <Gate admin=false children=children.clone()/>
        </Transition>
    }
}

/// N'affiche `children` que pour un administrateur : un anonyme est renvoye vers la connexion, un compte ordinaire
/// recoit la page « acces refuse » (statut 403 cote serveur).
#[component]
pub fn RequireAdmin(children: ChildrenFn) -> impl IntoView {
    view! {
        <Transition fallback=|| ()>
            <Gate admin=true children=children.clone()/>
        </Transition>
    }
}

/// Decide quoi afficher selon le *statut* du visiteur.
///
/// Deux contraintes. (1) La ressource doit etre lue **sous** la `Transition` : sinon le rendu serveur ne l'attend
/// pas. (2) Un rechargement de la navigation (modification du menu par exemple) ne doit pas recreer la page
/// affichee : le composant n'est cree qu'une fois par page et ne reagit qu'a un changement de *statut*, grace au
/// memo (qui ne notifie que si la valeur change).
#[component]
fn Gate(admin: bool, children: ChildrenFn) -> impl IntoView {
    let state = expect_context::<LayoutState>();
    let location = use_location();
    // Pendant un rechargement la ressource garde sa derniere valeur ; sans valeur du tout on garde le dernier statut.
    let status = Memo::new(
        move |prev: Option<&Option<AuthStatus>>| match state.data.get() {
            Some(d) => Some(status_of(&d)),
            None => prev.copied().flatten(),
        },
    );
    move || match status.get() {
        // Pas encore de reponse du serveur.
        None => ().into_any(),
        Some(AuthStatus::Anonymous) => {
            if state.leaving.get_untracked() {
                // Depart volontaire (deconnexion...) : la navigation est deja en cours.
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
    // Le statut HTTP n'existe que cote serveur.
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
