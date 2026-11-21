// SPDX-License-Identifier: AGPL-3.0-or-later
//! Essai a blanc des scripts Rhai (R4 lot 5, ADR 0034) : le script s'execute dans le meme hote sandboxe que les vraies
//! executions, avec les vrais etats d'appareils, mais **rien n'est publie, active ni enregistre** : on montre ce qu'il aurait fait.

use dominet_proto::DryRunDto;
use leptos::prelude::*;

/// Essaie un script avec un topic et un message d'exemple (texte JSON, ou texte simple). Administrateurs seulement.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si les appareils sont illisibles.
#[server(name = DryRunScript, prefix = "/sf", endpoint = "scripts/dry-run")]
pub async fn dry_run_script(
    code: String,
    topic: String,
    payload: String,
) -> Result<DryRunDto, ServerFnError> {
    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let run = dominet_server::config_service::dry_run_script(&state, code, topic, &payload)
        .await
        .map_err(|e| internal("device list", &e))?;
    tracing::info!(
        admin_id = admin.id.0,
        ok = run.error.is_empty(),
        "dry run of a script"
    );
    Ok(DryRunDto {
        error: run.error,
        effects: run.effects,
        logs: run.logs,
        elapsed_ms: run.elapsed_ms,
    })
}
