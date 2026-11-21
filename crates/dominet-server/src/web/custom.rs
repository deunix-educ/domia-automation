// SPDX-License-Identifier: AGPL-3.0-or-later
//! Feuille de style de personnalisation (`/custom.css`) : un fichier de la machine (`CUSTOM_CSS`), relu a chaque requete, charge apres la
//! feuille principale. Elle permet de changer couleurs, police et largeurs (variables CSS de `style/main.css`) **sans reconstruire l'image**.
//!
//! Publique (la page de connexion est aussi habillee) : ce n'est que du style. Absente, illisible ou trop grosse : feuille vide, jamais une erreur.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::AppState;

/// Taille maximale lue (256 Kio) : au-dela, le fichier est ignore.
const MAX_BYTES: u64 = 256 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/custom.css", get(custom_css))
}

async fn custom_css(State(state): State<AppState>) -> Response {
    let body = match state.config.custom_css.as_deref() {
        Some(path) => read_limited(path).await,
        None => String::new(),
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            // Une modification du fichier se voit au rechargement de la page.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

async fn read_limited(path: &str) -> String {
    match tokio::fs::metadata(path).await {
        Ok(m) if m.is_file() && m.len() <= MAX_BYTES => {
            tokio::fs::read_to_string(path).await.unwrap_or_default()
        }
        Ok(m) if m.is_file() => {
            tracing::warn!(
                path,
                taille = m.len(),
                "custom.css ignored: more than 256 KiB"
            );
            String::new()
        }
        _ => String::new(),
    }
}
