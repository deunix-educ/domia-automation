// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serveur DomiNet : configuration, erreurs, observabilite, authentification, routes HTTP, CLI.

pub mod alerts;
pub mod android;
pub mod auth;
pub mod automation;
pub mod backup;
pub mod cameras;
pub mod cli;
pub mod commands;
pub mod config;
pub mod config_service;
pub mod control;
pub mod detection;
pub mod discovery;
pub mod doctor;
pub mod error;
pub mod external_alerts;
pub mod frames;
pub mod grafana;
pub mod html;
pub mod hub;
pub mod icons;
pub mod logbuf;
pub mod mcp_tokens;
pub mod messaging;
pub mod metrics;
pub mod mqtt;
pub mod notify;
pub mod prometheus_alerts;
pub mod publishers;
pub mod recordings;
pub mod rules;
pub mod scenes;
pub mod scheduler;
pub mod secrets;
pub mod server;
pub mod settings;
pub mod snapshots;
pub mod system_stats;
pub mod telemetry;
pub mod timefmt;
pub mod web;

use std::sync::Arc;

use dominet_domain::auth::LockoutPolicy;
use dominet_store::PgStore;
use sqlx::PgPool;

use crate::auth::password::Hasher;
use crate::auth::service::AuthService;
use crate::config::Config;
use crate::secrets::{MasterKey, SecretBox};
use crate::settings::SettingsService;

/// Etat partage par tous les gestionnaires HTTP.
#[derive(Clone, Debug)]
pub struct AppState {
    pub pool: PgPool,
    pub store: PgStore,
    pub config: Arc<Config>,
    pub auth: AuthService,
    pub settings: SettingsService,
    /// Cameras : secrets chiffres et synchronisation MediaMTX (R6).
    pub cameras: cameras::CameraService,
    /// Canaux d'envoi (e-mail, SMS Free Mobile) : messagerie et alertes.
    pub channels: notify::Channels,
    /// Diffusion temps reel (WebSocket).
    pub hub: hub::Hub,
    /// Appareils Android connectes (R5) et ordres qu'on peut leur adresser.
    pub android: android::AndroidRelay,
    /// Client MQTT, present si le role `mqtt` tourne (`MQTT_HOST` renseigne).
    pub mqtt: Option<dominet_mqtt::MqttClient>,
    /// `permit_join` de Zigbee2MQTT tel que lu dans `bridge/info` (faux tant qu'on ne l'a pas recu).
    pub permit_join: Arc<std::sync::atomic::AtomicBool>,
    /// Signale que les sources MQTT ont change (administration) : l'ecoute recharge ses abonnements.
    pub sources_changed: Arc<tokio::sync::Notify>,
    /// Signale que les automations ont change (administration) : le moteur les recharge et les abonnements suivent.
    pub automations_changed: Arc<tokio::sync::Notify>,
    /// Signale que les automations ont change : le planificateur resynchronise ses travaux et se reveille.
    pub schedule_changed: Arc<tokio::sync::Notify>,
    /// Signale que les regles d'alerte ont change (administration) : l'evaluation les relit.
    pub alert_rules_changed: Arc<tokio::sync::Notify>,
    /// Signale que les stations meteo ont change (administration) : le publisher interroge tout de suite.
    pub stations_changed: Arc<tokio::sync::Notify>,
    /// Signale que les cameras ont change (administration) : le role `video` relit la liste des flux a decoder.
    pub cameras_changed: Arc<tokio::sync::Notify>,
    /// Flux d'images decodees (role `video`).
    pub frames: frames::FrameHub,
    /// Derniere image de chaque camera et snapshots indexes.
    pub snapshots: snapshots::SnapshotService,
    /// Segments enregistres : index, archive ReductStore, lecture, enregistrement manuel.
    pub recordings: recordings::RecordingService,
    /// Mouvement et detection d'objets (role `video`).
    pub detector: detection::Detector,
    /// Regles d'alerte Prometheus et Grafana : fichiers de provisionnement et rechargement (R7).
    pub rules: rules::RulesService,
    /// Alertes en cours chez Prometheus et Grafana, en lecture seule (R13).
    pub external_alerts: external_alerts::ExternalAlerts,
}

impl AppState {
    /// Etat avec les parametres de hachage de production.
    #[must_use]
    pub fn new(pool: PgPool, config: Arc<Config>) -> Self {
        Self::with_hasher(pool, config, Hasher::new())
    }

    /// Etat avec un hacheur fourni (tests : parametres legers).
    #[must_use]
    pub fn with_hasher(pool: PgPool, config: Arc<Config>, hasher: Hasher) -> Self {
        let store = PgStore::new(pool.clone());
        let auth = AuthService::new(store.clone(), hasher, LockoutPolicy::default());
        // La cle maitresse a deja ete validee par `Config` ; invalide ici, on la traite comme absente.
        let sealer = config
            .master_key
            .as_ref()
            .and_then(|k| MasterKey::parse(k.expose()).ok())
            .map(|k| SecretBox::new(&k));
        let mediamtx = config
            .mediamtx_api_url
            .as_deref()
            .and_then(|u| dominet_surveillance::MediaMtx::new(u).ok());
        let cameras_changed = Arc::new(tokio::sync::Notify::new());
        let cameras = cameras::CameraService::new(store.clone(), sealer.clone(), mediamtx)
            .with_changes(cameras_changed.clone())
            .with_snapshots_dir(std::path::PathBuf::from(&config.snapshots_dir))
            .with_recordings_dir(std::path::PathBuf::from(&config.recordings_dir));
        let snapshots = snapshots::SnapshotService::new(
            store.clone(),
            std::path::PathBuf::from(&config.snapshots_dir),
            config.snapshots_max_per_camera,
        );
        let video = config.reductstore.as_ref().and_then(|r| {
            dominet_store::video::ReductVideo::new(
                &r.url,
                r.token.as_ref().map(config::Secret::expose),
                &r.video_bucket,
                u64::from(r.video_quota_gb) * 1024 * 1024 * 1024,
            )
            .ok()
        });
        let recordings = recordings::RecordingService::new(
            store.clone(),
            video,
            config
                .mediamtx_api_url
                .as_deref()
                .and_then(|u| dominet_surveillance::MediaMtx::new(u).ok()),
            std::path::PathBuf::from(&config.recordings_dir),
            std::time::Duration::from_secs(u64::from(config.recordings_segment_secs)),
            std::time::Duration::from_secs(u64::from(config.recordings_grace_secs)),
        );
        let frames = frames::FrameHub::from_config(&config);
        let settings = SettingsService::new(store.settings(), sealer);
        let channels = notify::Channels::new(&config, settings.clone());
        let rules = rules::RulesService::new(store.clone(), &config);
        let external_alerts = external_alerts::ExternalAlerts::new(&config);
        Self {
            pool,
            store,
            config,
            auth,
            settings,
            cameras,
            channels,
            hub: hub::Hub::new(),
            android: android::AndroidRelay::default(),
            mqtt: None,
            permit_join: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sources_changed: Arc::new(tokio::sync::Notify::new()),
            automations_changed: Arc::new(tokio::sync::Notify::new()),
            schedule_changed: Arc::new(tokio::sync::Notify::new()),
            alert_rules_changed: Arc::new(tokio::sync::Notify::new()),
            stations_changed: Arc::new(tokio::sync::Notify::new()),
            cameras_changed,
            frames,
            snapshots,
            recordings,
            detector: detection::Detector::default(),
            rules,
            external_alerts,
        }
    }

    /// Les automations viennent de changer (creation, modification, suppression) : le moteur et le planificateur les relisent.
    pub fn automations_updated(&self) {
        self.automations_changed.notify_one();
        self.schedule_changed.notify_one();
    }
}
