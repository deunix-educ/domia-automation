// SPDX-License-Identifier: AGPL-3.0-or-later
//! DomiNet server: configuration, errors, observability, authentication, HTTP routes, CLI.

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

/// State shared by all the HTTP handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    pub pool: PgPool,
    pub store: PgStore,
    pub config: Arc<Config>,
    pub auth: AuthService,
    pub settings: SettingsService,
    /// Cameras: encrypted secrets and MediaMTX synchronisation (R6).
    pub cameras: cameras::CameraService,
    /// Sending channels (e-mail, Free Mobile SMS): messaging and alerts.
    pub channels: notify::Channels,
    /// Real-time broadcast (WebSocket).
    pub hub: hub::Hub,
    /// Connected Android devices (R5) and the orders that can be sent to them.
    pub android: android::AndroidRelay,
    /// MQTT client, present if the `mqtt` role is running (`MQTT_HOST` set).
    pub mqtt: Option<dominet_mqtt::MqttClient>,
    /// Zigbee2MQTT's `permit_join` as read from `bridge/info` (false until it has been received).
    pub permit_join: Arc<std::sync::atomic::AtomicBool>,
    /// Signals that the MQTT sources have changed (administration): listening reloads its subscriptions.
    pub sources_changed: Arc<tokio::sync::Notify>,
    /// Signals that the automations have changed (administration): the engine reloads them and the subscriptions follow.
    pub automations_changed: Arc<tokio::sync::Notify>,
    /// Signals that the automations have changed: the scheduler resynchronises its jobs and wakes up.
    pub schedule_changed: Arc<tokio::sync::Notify>,
    /// Signals that the alert rules have changed (administration): the evaluation re-reads them.
    pub alert_rules_changed: Arc<tokio::sync::Notify>,
    /// Signals that the weather stations have changed (administration): the publisher queries right away.
    pub stations_changed: Arc<tokio::sync::Notify>,
    /// Signals that the cameras have changed (administration): the `video` role re-reads the list of streams to decode.
    pub cameras_changed: Arc<tokio::sync::Notify>,
    /// Stream of decoded images (`video` role).
    pub frames: frames::FrameHub,
    /// Last image of each camera and indexed snapshots.
    pub snapshots: snapshots::SnapshotService,
    /// Recorded segments: index, ReductStore archive, playback, manual recording.
    pub recordings: recordings::RecordingService,
    /// Motion and object detection (`video` role).
    pub detector: detection::Detector,
    /// Prometheus and Grafana alert rules: provisioning files and reload (R7).
    pub rules: rules::RulesService,
    /// Ongoing alerts at Prometheus and Grafana, read-only (R13).
    pub external_alerts: external_alerts::ExternalAlerts,
}

impl AppState {
    /// State with the production hashing parameters.
    #[must_use]
    pub fn new(pool: PgPool, config: Arc<Config>) -> Self {
        Self::with_hasher(pool, config, Hasher::new())
    }

    /// State with a supplied hasher (tests: light parameters).
    #[must_use]
    pub fn with_hasher(pool: PgPool, config: Arc<Config>, hasher: Hasher) -> Self {
        let store = PgStore::new(pool.clone());
        let auth = AuthService::new(store.clone(), hasher, LockoutPolicy::default());
        // The master key has already been validated by `Config`; invalid here, we treat it as absent.
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

    /// The automations have just changed (creation, modification, deletion): the engine and the scheduler re-read them.
    pub fn automations_updated(&self) {
        self.automations_changed.notify_one();
        self.schedule_changed.notify_one();
    }
}
