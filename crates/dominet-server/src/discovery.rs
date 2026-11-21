// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decouverte ONVIF entre le role `discovery` et le role `web` (R6 lot 3, ADR 0045).
//!
//! Le role `discovery` tourne en reseau hote (le multicast ne traverse pas un reseau Docker ponte) ; le serveur web n'y a pas
//! acces. Les deux se parlent **par PostgreSQL** (`LISTEN`/`NOTIFY`), sans nouveau port ni nouveau service :
//!
//! - le navigateur demande (`request`) -> `NOTIFY onvif_discovery_request` ;
//! - le role `discovery` ecoute, sonde le reseau et publie chaque etape sur `onvif_discovery_result` (JSON de [`DiscoveryEvent`]) ;
//! - le role `web` ecoute les resultats, repere les cameras deja enregistrees et les diffuse sur le sujet `discovery`.
//!
//! Une demande recue pendant une recherche est ignoree (une seule recherche a la fois).

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use dominet_domain::repo::CameraRepo;
use dominet_proto::{DiscoveredDto, DiscoveryEvent, Topic, WsServerMsg};
use dominet_surveillance::onvif::discovery::{self, Discovered};
use sqlx::PgPool;
use sqlx::postgres::PgListener;

use crate::AppState;

pub const REQUEST: &str = "onvif_discovery_request";
pub const RESULT: &str = "onvif_discovery_result";
const RETRY: Duration = Duration::from_secs(5);

/// Demande une recherche (un role `discovery` doit l'ecouter).
///
/// # Errors
/// La base ne repond pas.
pub async fn request(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_notify($1, 'start')")
        .bind(REQUEST)
        .execute(pool)
        .await
        .map(|_| ())
}

async fn publish(pool: &PgPool, event: &DiscoveryEvent) {
    let Ok(payload) = serde_json::to_string(event) else {
        return;
    };
    if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
        .bind(RESULT)
        .bind(payload)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %e, "discovery result not published");
    }
}

/// Comment sonder : le multicast en production, un repondeur de la boucle locale dans les tests.
#[derive(Debug, Clone)]
pub struct Probe {
    pub target: SocketAddr,
    pub interface: Option<Ipv4Addr>,
    pub probes: u32,
    pub listen: Duration,
}

impl Probe {
    /// WS-Discovery sur le reseau local (3 sondes, 6 s d'ecoute).
    #[must_use]
    pub fn multicast() -> Self {
        Self {
            target: discovery::MULTICAST,
            interface: Some(discovery::local_ipv4()),
            probes: discovery::PROBES,
            listen: discovery::LISTEN,
        }
    }
}

fn dto(d: &Discovered) -> DiscoveredDto {
    DiscoveredDto {
        ip: d.ip.to_string(),
        port: d.port,
        name: d.name.clone(),
        hardware: d.hardware.clone(),
        known: false,
    }
}

/// Une recherche complete, publiee etape par etape.
async fn scan(pool: &PgPool, probe: &Probe) {
    publish(pool, &DiscoveryEvent::Scanning).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let probe = probe.clone();
    let probing = async move {
        discovery::discover_at(
            probe.target,
            probe.interface,
            probe.probes,
            probe.listen,
            move |d| {
                let _ = tx.send(d);
            },
        )
        .await
    };
    let forwarding = async {
        while let Some(d) = rx.recv().await {
            publish(pool, &DiscoveryEvent::Found { device: dto(&d) }).await;
        }
    };
    let (result, ()) = tokio::join!(probing, forwarding);
    match result {
        Ok(()) => publish(pool, &DiscoveryEvent::Done).await,
        Err(e) => {
            tracing::warn!(error = %e, "ONVIF discovery failed");
            publish(
                pool,
                &DiscoveryEvent::Failed {
                    message: e.to_string(),
                },
            )
            .await;
        }
    }
}

/// Role `discovery` : ecoute les demandes jusqu'a l'arret du processus, se reconnecte si la base disparait.
pub fn spawn_role(state: &AppState) {
    spawn_role_with(state, Probe::multicast());
}

/// Comme [`spawn_role`], avec une sonde choisie (tests).
pub fn spawn_role_with(state: &AppState, probe: Probe) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        tracing::info!("discovery role: waiting for requests");
        loop {
            if let Err(e) = serve_requests(&pool, &probe).await {
                tracing::warn!(error = %e, "listening for discovery requests interrupted");
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}

async fn serve_requests(pool: &PgPool, probe: &Probe) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(REQUEST).await?;
    loop {
        listener.recv().await?;
        scan(pool, probe).await;
        // Les demandes arrivees pendant la recherche sont abandonnees.
        while listener.try_recv().await?.is_some() {}
    }
}

/// Role `web` : diffuse les resultats aux navigateurs abonnes au sujet `discovery`.
pub fn spawn_relay(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = relay(&state).await {
                tracing::warn!(error = %e, "relay of discovery results interrupted");
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}

async fn relay(state: &AppState) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(&state.pool).await?;
    listener.listen(RESULT).await?;
    loop {
        let note = listener.recv().await?;
        let Ok(mut event) = serde_json::from_str::<DiscoveryEvent>(note.payload()) else {
            continue;
        };
        if let DiscoveryEvent::Found { device } = &mut event {
            device.known = state
                .store
                .cameras()
                .list()
                .await
                .is_ok_and(|all| all.iter().any(|c| c.config.ip == device.ip));
        }
        state
            .hub
            .publish(&Topic::Discovery, WsServerMsg::Discovery { event });
    }
}
