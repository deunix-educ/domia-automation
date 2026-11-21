// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP server of DomiNet (R11, ADR 0058 and 0059): lets an assistant read, drive and configure the application.
//!
//! The crate depends on `dominet-server` (shared [`AppState`]) and is mounted by the binary through
//! `server::serve(.., dominet_mcp::routes)`: the routes go through the same middlewares as the rest of the HTTP server
//! (trace, request id, timeout, security headers). Nothing is mounted unless `MCP_ENABLED=true`, and nobody gets in without an
//! access token created in the administration (`/admin/mcp`): tokens are stored hashed, carry scopes and an end date, and can
//! be revoked at once.
//!
//! Transport: Streamable HTTP, **stateless** (no session, plain JSON responses): every request is self-contained, which suits
//! tools that answer in a few milliseconds and keeps the server free of per-client state.

mod auth;
pub mod config;
mod lookup;
pub mod prompts;
mod tools;
mod views;

use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use dominet_server::AppState;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

pub use tools::{DominetMcp, SCOPES, StatusReport, scope_of};

/// Path of the endpoint.
pub const PATH: &str = "/mcp";

/// The `/mcp` routes, or `None` when the server is switched off (`MCP_ENABLED=false`, the default).
#[must_use]
pub fn routes(state: &AppState) -> Option<Router<AppState>> {
    if !state.config.mcp_enabled {
        return None;
    }
    let hosts = allowed_hosts(&state.config.allowed_hosts);
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(hosts);
    let app = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(DominetMcp::new(app.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    tracing::info!("MCP server mounted on {PATH}");
    Some(
        Router::new()
            .nest_service(PATH, service)
            .layer(from_fn_with_state(
                auth::Guard::new(state.clone()),
                auth::require_bearer,
            )),
    )
}

/// `Host` values accepted: the loopback names plus `ALLOWED_HOSTS` (protection against DNS rebinding).
fn allowed_hosts(configured: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = ["localhost", "127.0.0.1", "::1"]
        .into_iter()
        .map(String::from)
        .collect();
    for h in configured {
        if !hosts.contains(h) {
            hosts.push(h.clone());
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::allowed_hosts;

    #[test]
    fn loopback_is_always_allowed_and_configured_hosts_are_added_once() {
        let h = allowed_hosts(&["dominet.local".into(), "localhost".into()]);
        assert_eq!(h, ["localhost", "127.0.0.1", "::1", "dominet.local"]);
    }
}
