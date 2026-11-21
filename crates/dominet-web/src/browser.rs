// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acces au navigateur (stockage local, plein ecran). Sans effet cote serveur : le rendu serveur n'a pas de navigateur.

/// Cle de stockage de l'etat replie de la barre laterale (la meme que l'interface reference).
const COLLAPSED_KEY: &str = "vc-sidebar-collapsed";

#[cfg(feature = "hydrate")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Etat replie memorise, s'il existe.
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

/// Memorise l'etat replie (ignore les erreurs : stockage bloque, navigation privee...).
pub fn store_collapsed(collapsed: bool) {
    #[cfg(feature = "hydrate")]
    if let Some(s) = storage() {
        let _ = s.set_item(COLLAPSED_KEY, if collapsed { "1" } else { "0" });
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = collapsed;
}

/// Remonte au debut de la page (ouverture d'un formulaire de modification depuis une longue liste : le formulaire est en haut).
pub fn scroll_to_top() {
    #[cfg(feature = "hydrate")]
    if let Some(w) = web_sys::window() {
        w.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

/// Charge `path` par une vraie requete au serveur (page hors de l'interface : documentation, Grafana).
pub fn load_page(path: &str) {
    #[cfg(feature = "hydrate")]
    if let Some(w) = web_sys::window() {
        let _ = w.location().assign(path);
    }
    #[cfg(not(feature = "hydrate"))]
    let _ = path;
}

/// Bascule le plein ecran.
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

/// Contenu REEL d'un champ (vide s'il n'est pas encore monte). A preferer a un signal alimente par `input` pour les
/// formulaires de connexion : le remplissage automatique et la restauration d'un formulaire par le navigateur posent une
/// valeur sans evenement `input`.
#[must_use]
pub fn input_value(field: leptos::prelude::NodeRef<leptos::html::Input>) -> String {
    use leptos::prelude::GetUntracked;
    field.get_untracked().map(|e| e.value()).unwrap_or_default()
}

/// Vide un champ (apres un echec, pour ne pas laisser un mot de passe a l'ecran).
pub fn clear_input(field: leptos::prelude::NodeRef<leptos::html::Input>) {
    use leptos::prelude::GetUntracked;
    if let Some(e) = field.get_untracked() {
        e.set_value("");
    }
}
