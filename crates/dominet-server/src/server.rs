// SPDX-License-Identifier: AGPL-3.0-or-later
//! Demarrage du serveur HTTP, taches de maintenance et arret propre.

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

/// Duree de conservation du journal des tentatives de connexion.
const ATTEMPTS_RETENTION_DAYS: i64 = 7;

/// Cree le pool PostgreSQL **paresseux** : le serveur demarre meme si la base est absente et
/// `/readyz` repond alors 503 (utile pour les orchestrateurs).
///
/// # Errors
/// Retourne une erreur si l'URL de la base est inexploitable.
pub fn lazy_pool(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(config.database_url.expose())
        .context("unusable DATABASE_URL")
}

/// Purge toutes les heures les sessions expirees et les tentatives de connexion anciennes, et une fois par jour l'historique
/// des appareils plus vieux que `HISTORY_RETENTION_DAYS`. (Sera reprise par le planificateur en R4.)
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

/// Lance les roles demandes jusqu'a SIGINT/SIGTERM : `Web` (serveur HTTP) et/ou `Mqtt` (client MQTT supervise). `ui` construit l'interface (pages et fonctions serveur) a partir
/// de l'etat de l'application ; elle est servie pour tout ce que l'API et les sondes ne traitent pas. `extra` ajoute des routes
/// (serveur MCP) sous les memes middlewares ; `None` si la fonctionnalite est coupee.
///
/// # Errors
/// Retourne une erreur si le port est indisponible ou si le serveur s'arrete anormalement.
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
        // Role `mqtt` present : on publie par son client ; sinon le role a le sien (sans ecoute).
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
        // Seul avec la base : un client MQTT a lui (sans ecoute), si un broker est configure, pour publier et activer des scenes.
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
        // Mouvement et detection publient sur MQTT : le client du role `mqtt` s'il tourne ici, sinon un client a part (sans ecoute).
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
    // Cache des reglages charge avant la premiere page : l'en-tete CSP en depend (iframe Grafana), sans pouvoir attendre.
    if let Err(e) = state.settings.reload().await {
        tracing::warn!(error = %e, "settings not loaded at start-up (database unreachable?)");
    }
    // Nombre d'alertes en cours : retenu des le demarrage (la pastille de la barre laterale l'affiche des l'arrivee).
    crate::alerts::publish_count(&state.hub, &state.store, &state.external_alerts).await;
    crate::system_stats::spawn(
        state.hub.clone(),
        Duration::from_secs(u64::from(state.config.system_stats_secs)),
    );
    if roles.contains(&Role::Web) {
        // Aucune connexion n'existe encore : « en ligne » ne doit plus rien affirmer (D4). Puis balayage des SMS sans accuse.
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
    // `ConnectInfo` donne l'adresse du pair (necessaire au blocage par IP).
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

/// Taches de demarrage du role web, en fond : une base injoignable ne doit pas retarder le serveur HTTP.
fn spawn_startup_tasks(state: &AppState) {
    // Les fichiers de regles refletent la base des le demarrage (un Prometheus / Grafana neuf les lit au sien).
    if state.rules.is_configured() {
        let rules = state.rules.clone();
        tokio::spawn(async move {
            let (prom, graf) = rules.sync_all().await;
            tracing::info!(?prom, ?graf, "alert rules synchronised at start-up");
        });
    }
    // Les messages interrompus par l'arret precedent sont clotures.
    let recovering = state.clone();
    // MediaMTX est remis d'aplomb avec la base (un MediaMTX redemarre a perdu les paths ajoutes a chaud).
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
