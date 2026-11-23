// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP server start-up, maintenance tasks and clean shutdown.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use dominet_domain::repo::{LoginAttemptRepo, McpAuditRepo, StateLogRepo};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower_sessions::session_store::ExpiredDeletion;

use crate::cli::Role;
use crate::config::Config;
use crate::{AppState, web};

/// Retention of the login-attempts log.
const ATTEMPTS_RETENTION_DAYS: i64 = 7;

/// Creates the **lazy** PostgreSQL pool: the server starts even if the database is absent and
/// `/readyz` then answers 503 (useful for orchestrators).
///
/// # Errors
/// Returns an error if the database URL is unusable.
pub fn lazy_pool(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(config.database_url.expose())
        .context("unusable DATABASE_URL")
}

/// Hourly purges the expired sessions and the old login attempts, and once a day the devices' history
/// older than `HISTORY_RETENTION_DAYS`. (Will be taken over by the scheduler in R4.)
fn spawn_maintenance(state: &AppState) {
    let sessions = state.store.sessions();
    let attempts = state.store.attempts();
    let history = state.store.state_log();
    let retention_days = i64::from(state.config.history_retention_days);
    let audit = state.store.mcp_audit();
    let audit_days = i64::from(state.config.mcp_audit_retention_days);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        let mut ticks: u64 = 0;
        loop {
            tick.tick().await;
            if ticks.is_multiple_of(24) {
                match audit
                    .purge_before(Utc::now() - chrono::Duration::days(audit_days))
                    .await
                {
                    Ok(n) if n > 0 => {
                        tracing::info!(purged = n, audit_days, "old MCP audit log deleted");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "cannot purge the MCP audit log"),
                }
                match history
                    .purge_before(Utc::now() - chrono::Duration::days(retention_days))
                    .await
                {
                    Ok(n) if n > 0 => {
                        tracing::info!(purged = n, retention_days, "old history deleted");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "cannot purge the history"),
                }
            }
            ticks += 1;
            if let Err(e) = sessions.delete_expired().await {
                tracing::warn!(error = %e, "cannot purge expired sessions");
            }
            match attempts
                .purge_before(Utc::now() - chrono::Duration::days(ATTEMPTS_RETENTION_DAYS))
                .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!(purged = n, "old login attempts deleted");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "cannot purge the attempts"),
            }
        }
    });
}

/// Runs the requested roles until SIGINT/SIGTERM: `Web` (HTTP server) and/or `Mqtt` (supervised MQTT client). `ui` builds the interface (pages and server functions) from
/// the application state; it is served for everything the API and the probes do not handle. `extra` adds routes
/// (MCP server) under the same middlewares; `None` if the feature is off.
///
/// # Errors
/// Returns an error if the port is unavailable or if the server stops abnormally.
pub async fn serve(
    config: Config,
    roles: &[Role],
    ui: impl FnOnce(&AppState) -> axum::Router,
    extra: impl FnOnce(&AppState) -> Option<axum::Router<AppState>>,
) -> anyhow::Result<()> {
    let pool = lazy_pool(&config)?;
    let addr = config.http_addr;
    let mut state = AppState::new(pool, Arc::new(config));
    if roles.contains(&Role::Mqtt) {
        state.mqtt = Some(crate::mqtt::spawn(&state).await?);
    }
    if roles.contains(&Role::Publishers) {
        // `mqtt` role present: we publish through its client; otherwise the role has its own (without listening).
        let client = if let Some(c) = &state.mqtt {
            c.clone()
        } else {
            let c = crate::publishers::own_client(&state)?;
            state.mqtt = Some(c.clone());
            c
        };
        crate::publishers::spawn(&state, &client);
    }
    if roles.contains(&Role::Scheduler) {
        // Alone with the database: its own MQTT client (without listening), if a broker is configured, to publish and activate scenes.
        if state.mqtt.is_none() && state.config.mqtt.is_some() {
            match crate::scheduler::own_client(&state) {
                Ok(c) => state.mqtt = Some(c),
                Err(e) => tracing::warn!(error = %e, "scheduler without an MQTT client"),
            }
        }
        crate::scheduler::spawn(&state);
        crate::prometheus_alerts::spawn(&state);
        crate::backup::spawn(&state);
    }
    if roles.contains(&Role::Video) {
        // Motion and detection publish over MQTT: the `mqtt` role's client if it runs here, otherwise a separate client (without listening).
        if state.mqtt.is_none()
            && let Some(mut cfg) = state.config.mqtt.clone()
        {
            cfg.client_id = format!("{}-video", cfg.client_id);
            state.mqtt = Some(dominet_mqtt::start(&crate::mqtt::settings(&cfg)).client);
        }
        crate::frames::spawn_role(&state);
        crate::recordings::spawn_role(&state);
        crate::detection::spawn_role(&state);
    }
    if roles.contains(&Role::Discovery) {
        crate::discovery::spawn_role(&state);
    }
    if roles.contains(&Role::Web) {
        crate::discovery::spawn_relay(&state);
        crate::external_alerts::spawn(&state);
    }
    crate::metrics::spawn_listener(&state).await;
    spawn_maintenance(&state);
    // Settings cache loaded before the first page: the CSP header depends on it (Grafana iframe), without being able to wait.
    if let Err(e) = state.settings.reload().await {
        tracing::warn!(error = %e, "settings not loaded at start-up (database unreachable?)");
    }
    // Number of ongoing alerts: retained from start-up (the sidebar badge shows it as soon as the page arrives).
    crate::alerts::publish_count(&state.hub, &state.store, &state.external_alerts).await;
    crate::system_stats::spawn(
        state.hub.clone(),
        Duration::from_secs(u64::from(state.config.system_stats_secs)),
    );
    if roles.contains(&Role::Web) {
        // No connection exists yet: "online" must no longer claim anything (D4). Then sweep of the SMS without an acknowledgement.
        match dominet_domain::repo::AndroidDeviceRepo::set_all_offline(
            &state.store.android_devices(),
        )
        .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(devices = n, "Android devices set offline at start-up");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "Android device presence not reset");
            }
        }
        tokio::spawn(crate::android::run_expiry(state.clone()));
        spawn_startup_tasks(&state);
    }
    if !roles.contains(&Role::Web) {
        tracing::info!(?roles, "no HTTP role: waiting for the stop signal");
        shutdown_signal().await;
        if let Some(m) = &state.mqtt {
            m.shutdown();
        }
        return Ok(());
    }
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot listen on {addr}"))?;
    tracing::info!(%addr, version = env!("CARGO_PKG_VERSION"), "HTTP server started");
    // `ConnectInfo` gives the peer's address (needed for IP lock-out).
    let router = web::router_with_extra(state.clone(), ui(&state), extra(&state));
    let app = router.into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    if let Some(m) = &state.mqtt {
        m.shutdown();
    }
    tracing::info!("clean shutdown finished");
    Ok(())
}

/// Web role start-up tasks, in the background: an unreachable database must not delay the HTTP server.
fn spawn_startup_tasks(state: &AppState) {
    // The rules files reflect the database from start-up (a fresh Prometheus / Grafana reads them at its own start).
    if state.rules.is_configured() {
        let rules = state.rules.clone();
        tokio::spawn(async move {
            let (prom, graf) = rules.sync_all().await;
            tracing::info!(?prom, ?graf, "alert rules synchronised at start-up");
        });
    }
    // Messages interrupted by the previous shutdown are closed.
    let recovering = state.clone();
    // MediaMTX is brought back in line with the database (a restarted MediaMTX lost the paths added live).
    let syncing = state.clone();
    tokio::spawn(async move {
        if syncing.cameras.has_mediamtx() {
            match syncing.cameras.sync_all().await {
                Ok(r) => tracing::info!(
                    added = r.added.len(),
                    replaced = r.replaced.len(),
                    removed = r.removed.len(),
                    failed = r.failed.len(),
                    "MediaMTX synchronised at start-up"
                ),
                Err(e) => tracing::warn!(error = %e, "MediaMTX not synchronised at start-up"),
            }
        }
    });
    tokio::spawn(async move {
        let resumed = crate::messaging::recover_interrupted(&recovering).await;
        if resumed > 0 {
            tracing::warn!(
                messages = resumed,
                "messages interrupted by the previous stop: closed"
            );
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("stop signal received");
}
