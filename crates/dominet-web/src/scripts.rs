// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dry run of Rhai scripts (R4 lot 5, ADR 0034): the script runs in the same sandboxed host as the real
//! runs, with the real device states, but **nothing is published, enabled or saved**: we show what it would have done.

use dominet_proto::DryRunDto;
use leptos::prelude::*;

/// Tries a script with a topic and a sample message (JSON text, or plain text). Administrators only.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the devices are unreadable.
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
