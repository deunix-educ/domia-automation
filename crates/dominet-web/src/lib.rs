// SPDX-License-Identifier: AGPL-3.0-or-later
//! Interface DomiNet : rendu cote serveur (SSR) puis hydratation en WASM.
//!
//! - `app` : coque HTML, fournisseur i18n, routeur et pages ;
//! - `layout` : barre laterale, navigation dynamique et contenu ;
//! - `crud` : composant generique des pages d'administration ;
//! - `server_fns` : fonctions serveur (appelees depuis le navigateur, executees cote serveur) ;
//! - `ssr` (feature `ssr`) : assemblage du service HTTP de l'interface.

// Idiomes Leptos : les composants et leurs proprietes sont passes par valeur, et les fonctions de vue renvoient
// des `impl IntoView` que personne ne "doit utiliser" au sens de `must_use`.
// Le code genere par `#[server]` est `async` sans `await` cote navigateur (simple stub d'appel).
#![allow(
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]

/// Cle de traduction resolue plus tard (`I18n::tr`). Ne fait rien d'autre que rendre la cle reperable : le test
/// `tests/i18n.rs` la compte comme utilisee et verifie qu'elle existe dans les deux langues.
macro_rules! tk {
    ($key:literal) => {
        $key
    };
}

pub mod account;
pub mod alert_rules_admin;
pub mod alerts;
pub mod android;
pub mod app;
pub mod automations_admin;
pub mod browser;
pub mod camera_fns;
pub mod cameras_admin;
pub mod chrome;
pub mod crud;
#[cfg(feature = "ssr")]
pub mod csp;
pub mod dashboard;
pub mod dashboard_fns;
pub mod dashboards_admin;
pub mod device_fns;
mod device_widget;
pub mod devices_admin;
pub mod devices_pages;
pub mod drag;
pub mod energy;
pub mod events_page;
pub mod guard;
#[cfg(feature = "ssr")]
pub use dominet_server::icons;
pub mod assistant;
pub mod layout;
pub mod live;
pub mod locations;
pub mod login;
pub mod logs_admin;
pub mod mcp_admin;
pub mod messaging;
pub mod mqtt_sources;
pub mod nav;
pub mod navbar;
pub mod onvif_pages;
pub mod ptz_panel;
pub mod realtime;
pub mod recordings_page;
pub mod rules_admin;
pub mod scenes;
pub mod scenes_admin;
pub mod scripts;
pub mod server_fns;
pub mod settings;
pub mod snapshots_page;
#[cfg(feature = "ssr")]
pub mod ssr;
pub mod surveillance;
pub mod users;
pub mod weather_admin;
pub mod whep;
pub mod widgets_admin;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
