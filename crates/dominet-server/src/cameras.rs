// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera service (R6, ADR 0043 and 0044): registration with secret encryption, synchronisation with MediaMTX.
//!
//! - The passwords (RTSP, ONVIF, web) and the web API key are encrypted with `SecretBox`, the **associated data** being
//!   `camera:<uuid>:<field>`: a ciphertext copied to another camera or another field does not decrypt.
//! - On modification, a secret that is **not supplied** stays unchanged (the interface never sends a secret back).
//! - After each write, the camera's MediaMTX path is created, replaced or deleted; a MediaMTX failure does not undo
//!   the registration (the database is authoritative, `sync_all` catches up), it is reported.

use std::collections::BTreeMap;

use dominet_domain::camera::{Camera, CameraDraft, CameraId, CameraUuid, OnvifInfo, Sealed};
use dominet_domain::repo::CameraRepo;
use dominet_domain::{Problem, RepoError};
use dominet_store::PgStore;
pub use dominet_surveillance::night::{NightAction, NightError, NightOutcome};
use dominet_surveillance::night::{NightBackend, NightControl, NightState, NightTarget};
pub use dominet_surveillance::onvif::OnvifError;
use dominet_surveillance::onvif::{Credentials, Inspection, ONVIF_PORTS, OnvifClient, probe_port};
use dominet_surveillance::{MediaMtx, PathConf, SyncReport};
use uuid::Uuid;

use crate::secrets::SecretBox;

/// A form secret: `Keep` (field left empty) or a new value in clear.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum SecretInput {
    #[default]
    Keep,
    Set(String),
}

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep",
            Self::Set(_) => "Set(***)",
        })
    }
}

impl SecretInput {
    /// An empty input means "unchanged".
    #[must_use]
    pub fn from_form(s: &str) -> Self {
        if s.is_empty() {
            Self::Keep
        } else {
            Self::Set(s.to_string())
        }
    }
}

/// What the administration sends: the settings (the draft's secrets are ignored) and the secrets in clear.
#[derive(Debug, Clone)]
pub struct CameraInput {
    pub draft: CameraDraft,
    pub rtsp_pass: SecretInput,
    pub onvif_pass: SecretInput,
    pub web_pass: SecretInput,
    pub web_api_key: SecretInput,
}

#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("champs invalides")]
    Invalid(Vec<(&'static str, Problem)>),
    #[error("camera not found")]
    NotFound,
    #[error("DOMINET_MASTER_KEY absente : impossible d'enregistrer un secret de camera")]
    MasterKeyMissing,
    #[error("un secret de la camera est illisible (mauvaise DOMINET_MASTER_KEY ?)")]
    Unreadable,
    /// The camera does not have ONVIF enabled.
    #[error("ONVIF n'est pas active sur cette camera")]
    OnvifDisabled,
    /// No ONVIF service answered on the usual ports.
    #[error("aucun service ONVIF ne repond a cette adresse")]
    NoOnvifService,
    #[error(transparent)]
    Onvif(#[from] OnvifError),
    #[error(transparent)]
    Night(#[from] NightError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// State of the MediaMTX synchronisation after a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// `MEDIAMTX_API_URL` is not set.
    Disabled,
    Synced,
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("MEDIAMTX_API_URL n'est pas renseignee")]
    Disabled,
    #[error(transparent)]
    MediaMtx(#[from] dominet_surveillance::MediaMtxError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

#[derive(Clone, Debug)]
pub struct CameraService {
    store: PgStore,
    sealer: Option<SecretBox>,
    mediamtx: Option<MediaMtx>,
    /// Last state sent to each Thingino toggle (the camera does not read it back).
    night_state: NightState,
    /// Notified at every creation, modification or deletion (the `video` role re-reads its streams).
    changed: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Snapshots folder: removed with the camera.
    snapshots_dir: Option<std::path::PathBuf>,
    recordings_dir: Option<std::path::PathBuf>,
}

fn aad(uuid: CameraUuid, field: &str) -> String {
    format!("camera:{uuid}:{field}")
}

impl CameraService {
    #[must_use]
    pub fn new(store: PgStore, sealer: Option<SecretBox>, mediamtx: Option<MediaMtx>) -> Self {
        Self {
            store,
            sealer,
            mediamtx,
            night_state: NightState::default(),
            changed: None,
            snapshots_dir: None,
            recordings_dir: None,
        }
    }

    /// Notifies `changed` after each write.
    #[must_use]
    pub fn with_changes(mut self, changed: std::sync::Arc<tokio::sync::Notify>) -> Self {
        self.changed = Some(changed);
        self
    }

    /// Folder of the recorded segments, whose camera subfolder is deleted with it (after its MediaMTX path is stopped).
    #[must_use]
    pub fn with_recordings_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.recordings_dir = Some(dir);
        self
    }

    /// Snapshots folder, whose camera subfolder is deleted with it.
    #[must_use]
    pub fn with_snapshots_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.snapshots_dir = Some(dir);
        self
    }

    fn notify_changed(&self) {
        if let Some(n) = &self.changed {
            n.notify_one();
        }
    }

    /// Is MediaMTX configured?
    #[must_use]
    pub fn has_mediamtx(&self) -> bool {
        self.mediamtx.is_some()
    }

    /// All the cameras, by priority then by name.
    ///
    /// # Errors
    /// Repository error.
    pub async fn list(&self) -> Result<Vec<Camera>, CameraError> {
        Ok(self.store.cameras().list().await?)
    }

    /// # Errors
    /// Repository error.
    pub async fn get(&self, id: CameraId) -> Result<Option<Camera>, CameraError> {
        Ok(self.store.cameras().get(id).await?)
    }

    fn seal(&self, uuid: CameraUuid, field: &str, plain: &str) -> Result<Sealed, CameraError> {
        if plain.is_empty() {
            return Ok(Sealed::default());
        }
        let sealer = self.sealer.as_ref().ok_or(CameraError::MasterKeyMissing)?;
        Ok(Sealed(sealer.seal(&aad(uuid, field), plain)))
    }

    /// Secret in clear; empty if it is not set. Never to be logged or sent back to the interface.
    ///
    /// # Errors
    /// `Unreadable` if the master key is missing or has changed.
    pub fn open(
        &self,
        uuid: CameraUuid,
        field: &str,
        sealed: &Sealed,
    ) -> Result<String, CameraError> {
        if sealed.is_empty() {
            return Ok(String::new());
        }
        let cipher = self.sealer.as_ref().ok_or(CameraError::Unreadable)?;
        cipher
            .open(&aad(uuid, field), &sealed.0)
            .map_err(|_| CameraError::Unreadable)
    }

    fn merge(
        &self,
        uuid: CameraUuid,
        field: &str,
        input: &SecretInput,
        current: &Sealed,
    ) -> Result<Sealed, CameraError> {
        match input {
            SecretInput::Keep => Ok(current.clone()),
            SecretInput::Set(plain) => self.seal(uuid, field, plain),
        }
    }

    /// Creates (`id` absent) or replaces (`id` present) a camera, then updates its MediaMTX path.
    ///
    /// # Errors
    /// `Invalid` field by field, `NotFound`, `MasterKeyMissing` if a secret is supplied without a master key, repository error.
    pub async fn save(
        &self,
        id: Option<CameraId>,
        input: CameraInput,
    ) -> Result<(Camera, SyncStatus), CameraError> {
        let mut draft = input.draft;
        let problems = draft.problems();
        if !problems.is_empty() {
            return Err(CameraError::Invalid(problems));
        }
        let existing = match id {
            Some(id) => Some(self.get(id).await?.ok_or(CameraError::NotFound)?),
            None => None,
        };
        let uuid = existing
            .as_ref()
            .map_or_else(|| CameraUuid(Uuid::new_v4()), |c| c.uuid);
        let current = existing.as_ref().map(|c| &c.config);
        let none = Sealed::default();
        draft.rtsp_pass = self.merge(
            uuid,
            "rtsp_pass",
            &input.rtsp_pass,
            current.map_or(&none, |c| &c.rtsp_pass),
        )?;
        draft.onvif_pass = self.merge(
            uuid,
            "onvif_pass",
            &input.onvif_pass,
            current.map_or(&none, |c| &c.onvif_pass),
        )?;
        draft.web_pass = self.merge(
            uuid,
            "web_pass",
            &input.web_pass,
            current.map_or(&none, |c| &c.web_pass),
        )?;
        draft.web_api_key = self.merge(
            uuid,
            "web_api_key",
            &input.web_api_key,
            current.map_or(&none, |c| &c.web_api_key),
        )?;
        // What the ONVIF synchronisation learned is not in the form: we keep it.
        if let Some(c) = current {
            draft.manufacturer.clone_from(&c.manufacturer);
            draft.model.clone_from(&c.model);
            draft.firmware.clone_from(&c.firmware);
            // A token entered in the form wins; empty: we keep the synchronisation's.
            if draft.main_token.is_empty() {
                draft.main_token.clone_from(&c.main_token);
            }
        }
        // Like the reference: an ONVIF camera without its own credentials takes those of the RTSP stream.
        if draft.onvif {
            if draft.onvif_user.is_empty() {
                draft.onvif_user.clone_from(&draft.rtsp_user);
            }
            if draft.onvif_pass.is_empty() && !draft.rtsp_pass.is_empty() {
                let plain = self.open(uuid, "rtsp_pass", &draft.rtsp_pass)?;
                draft.onvif_pass = self.seal(uuid, "onvif_pass", &plain)?;
            }
        }
        let saved = match id {
            Some(id) => self.store.cameras().update(id, &draft).await,
            None => self.store.cameras().create(Some(uuid), &draft).await,
        }
        .map_err(|e| match e {
            RepoError::NotFound => CameraError::NotFound,
            other => CameraError::Repo(other),
        })?;
        let status = self.sync_one(&saved).await;
        self.notify_changed();
        Ok((saved, status))
    }

    /// Deletes the camera (and, in cascade, what belongs to it) then its MediaMTX path.
    ///
    /// # Errors
    /// `NotFound` or repository error.
    pub async fn delete(&self, id: CameraId) -> Result<SyncStatus, CameraError> {
        let camera = self.get(id).await?.ok_or(CameraError::NotFound)?;
        self.store.cameras().delete(id).await.map_err(|e| match e {
            RepoError::NotFound => CameraError::NotFound,
            other => CameraError::Repo(other),
        })?;
        // The camera's files go with it (the database rows are deleted in cascade).
        if let Some(dir) = &self.snapshots_dir {
            let _ = tokio::fs::remove_dir_all(dir.join(camera.uuid.to_string())).await;
            let _ = tokio::fs::remove_file(dir.join("latest").join(format!("{}.jpg", camera.uuid)))
                .await;
        }
        self.notify_changed();
        let Some(api) = &self.mediamtx else {
            self.remove_recordings(camera.uuid).await;
            return Ok(SyncStatus::Disabled);
        };
        let status = match api.remove(&camera.uuid.to_string()).await {
            Ok(()) => SyncStatus::Synced,
            Err(e) => {
                tracing::warn!(camera = %camera.uuid, error = %e, "MediaMTX path not removed");
                SyncStatus::Failed(e.to_string())
            }
        };
        // The path is stopped: MediaMTX no longer writes into the folder.
        self.remove_recordings(camera.uuid).await;
        Ok(status)
    }

    async fn remove_recordings(&self, uuid: CameraUuid) {
        if let Some(dir) = &self.recordings_dir {
            let _ = tokio::fs::remove_dir_all(dir.join(uuid.to_string())).await;
        }
    }

    fn path_conf(&self, camera: &Camera) -> Result<PathConf, CameraError> {
        let password = self.open(camera.uuid, "rtsp_pass", &camera.config.rtsp_pass)?;
        Ok(PathConf::for_camera(&camera.config, &password))
    }

    async fn sync_one(&self, camera: &Camera) -> SyncStatus {
        let Some(api) = &self.mediamtx else {
            return SyncStatus::Disabled;
        };
        let name = camera.uuid.to_string();
        let result = if camera.config.active {
            match self.path_conf(camera) {
                Ok(conf) => api.replace(&name, &conf).await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        } else {
            api.remove(&name).await.map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => SyncStatus::Synced,
            Err(message) => {
                tracing::warn!(camera = %camera.uuid, error = %message, "MediaMTX path not synchronised");
                SyncStatus::Failed(message)
            }
        }
    }

    /// Full synchronisation: one path per **active** camera, the paths of unknown cameras are removed.
    ///
    /// # Errors
    /// `Disabled` without `MEDIAMTX_API_URL`, MediaMTX not answering, repository error.
    pub async fn sync_all(&self) -> Result<SyncReport, SyncError> {
        let api = self.mediamtx.as_ref().ok_or(SyncError::Disabled)?;
        let mut wanted = BTreeMap::new();
        let mut unreadable = vec![];
        for camera in self.store.cameras().list().await? {
            if !camera.config.active {
                continue;
            }
            match self.path_conf(&camera) {
                Ok(conf) => {
                    wanted.insert(camera.uuid.to_string(), conf);
                }
                Err(e) => unreadable.push((camera.uuid.to_string(), e.to_string())),
            }
        }
        // A camera whose password is unreadable keeps its current path: we do not delete it by mistake.
        let keep: Vec<String> = unreadable.iter().map(|(n, _)| n.clone()).collect();
        let mut report = api.sync_keeping(&wanted, &keep).await?;
        report.failed.extend(unreadable);
        Ok(report)
    }

    /// A camera's ONVIF client: its ONVIF credentials, or the stream's by default (like the reference).
    pub fn onvif_client(&self, camera: &Camera) -> Result<OnvifClient, CameraError> {
        let d = &camera.config;
        let (user, sealed, field) = if d.onvif_user.is_empty() {
            (&d.rtsp_user, &d.rtsp_pass, "rtsp_pass")
        } else {
            (&d.onvif_user, &d.onvif_pass, "onvif_pass")
        };
        let pass = self.open(camera.uuid, field, sealed)?;
        let creds = (!user.is_empty()).then(|| Credentials {
            user: user.clone(),
            pass,
        });
        let port = u16::try_from(d.onvif_port).unwrap_or(80);
        Ok(OnvifClient::new(&d.ip, port, creds)?)
    }

    /// Day / night toggle, IR-cut or light of an **active** camera through the configured method (`night_mode_api`).
    ///
    /// # Errors
    /// `NotFound` (unknown or inactive), `Night(NotConfigured)`, unknown action, unreadable secret, camera error.
    pub async fn night(&self, id: CameraId, action: &str) -> Result<NightOutcome, CameraError> {
        use dominet_domain::camera::NightApi;
        let action: NightAction = action.parse()?;
        let camera = self
            .get(id)
            .await?
            .filter(|c| c.config.active)
            .ok_or(CameraError::NotFound)?;
        let d = &camera.config;
        let open = |field: &str, sealed: &Sealed| self.open(camera.uuid, field, sealed);
        let rtsp_pass = open("rtsp_pass", &d.rtsp_pass)?;
        // ONVIF credentials, by default those of the stream (like the reference).
        let (onvif_user, onvif_pass) = if d.onvif_user.is_empty() {
            (d.rtsp_user.clone(), rtsp_pass.clone())
        } else {
            (d.onvif_user.clone(), open("onvif_pass", &d.onvif_pass)?)
        };
        let (mut web_user, mut web_pass) = (d.web_user.clone(), open("web_pass", &d.web_pass)?);
        if d.night_mode_api == NightApi::Goform && web_user.is_empty() {
            // Goform: web credentials, by default those of the stream.
            web_user = d.rtsp_user.clone();
            web_pass = rtsp_pass;
        }
        let target = NightTarget {
            id: camera.uuid.to_string(),
            host: d.ip.clone(),
            web_port: u16::try_from(d.web_port).unwrap_or(80),
            web_user,
            web_pass,
            web_api_key: open("web_api_key", &d.web_api_key)?,
            onvif_port: u16::try_from(d.onvif_port).unwrap_or(80),
            onvif_user,
            onvif_pass,
        };
        let backend = match d.night_mode_api {
            NightApi::None => return Err(NightError::NotConfigured.into()),
            NightApi::Openipc => NightBackend::OpenIpc(target),
            NightApi::Thingino => NightBackend::Thingino(target, self.night_state.clone()),
            NightApi::Onvif => NightBackend::Onvif(target),
            NightApi::Goform => NightBackend::Goform(target),
        };
        let outcome = backend.run(action).await?;
        tracing::info!(camera = %camera.uuid, action = action.as_str(), api = d.night_mode_api.as_str(), "day/night command");
        Ok(outcome)
    }

    /// ONVIF synchronisation: reads the camera's identity, main profile and PTZ and records them.
    ///
    /// # Errors
    /// `OnvifDisabled`, credentials refused, camera unreachable, repository error.
    pub async fn onvif_sync(&self, id: CameraId) -> Result<Inspection, CameraError> {
        let camera = self.get(id).await?.ok_or(CameraError::NotFound)?;
        if !camera.config.onvif {
            return Err(CameraError::OnvifDisabled);
        }
        let inspection = self.onvif_client(&camera)?.inspect().await?;
        self.store
            .cameras()
            .apply_onvif(
                id,
                &OnvifInfo {
                    manufacturer: inspection.info.manufacturer.clone(),
                    model: inspection.info.model.clone(),
                    firmware: inspection.info.firmware.clone(),
                    main_token: inspection.main_token.clone(),
                    // PTZ is an administrator's decision: ONVIF only suggests it at the first synchronisation.
                    ptz: if camera.config.main_token.is_empty() {
                        inspection.ptz
                    } else {
                        camera.config.ptz
                    },
                },
            )
            .await?;
        tracing::info!(camera = %camera.uuid, model = %inspection.info.model, ptz = inspection.ptz, "ONVIF synchronisation");
        Ok(inspection)
    }

    /// Adding by IP address (discovery, or a camera the multicast does not reach). If a camera already has this address, it is
    /// returned as is (`false`). Otherwise the ONVIF port is searched (`onvif_port` supplied, or the usual ports), the camera is
    /// created with ONVIF enabled and, if credentials are supplied, synchronised right away (a failure does not cancel the addition).
    ///
    /// # Errors
    /// Invalid address, `NoOnvifService`, repository error.
    pub async fn add_by_ip(
        &self,
        ip: &str,
        onvif_port: Option<u16>,
        rtsp_user: &str,
        rtsp_pass: &str,
        name: Option<&str>,
    ) -> Result<(Camera, bool), CameraError> {
        let ip = ip.trim();
        let mut draft = CameraDraft::new(name.unwrap_or(&format!("Cam\u{e9}ra {ip}")));
        draft.ip = ip.to_string();
        draft.onvif = true;
        draft.rtsp_user = rtsp_user.trim().to_string();
        let problems: Vec<_> = draft
            .problems()
            .into_iter()
            .filter(|(field, _)| *field == "ip" || *field == "name")
            .collect();
        if !problems.is_empty() {
            return Err(CameraError::Invalid(problems));
        }
        if let Some(existing) = self.list().await?.into_iter().find(|c| c.config.ip == ip) {
            return Ok((existing, false));
        }
        let port = match onvif_port {
            Some(p) => p,
            None => probe_port(ip, &ONVIF_PORTS)
                .await
                .ok_or(CameraError::NoOnvifService)?,
        };
        draft.onvif_port = i32::from(port);
        let input = CameraInput {
            draft,
            rtsp_pass: SecretInput::from_form(rtsp_pass),
            onvif_pass: SecretInput::Keep,
            web_pass: SecretInput::Keep,
            web_api_key: SecretInput::Keep,
        };
        let (camera, _) = self.save(None, input).await?;
        if !rtsp_user.is_empty()
            && let Err(e) = self.onvif_sync(camera.id).await
        {
            tracing::info!(camera = %camera.uuid, error = %e, "ONVIF synchronisation deferred after adding");
        }
        let camera = self.get(camera.id).await?.unwrap_or(camera);
        Ok((camera, true))
    }
}

/// WHEP address of a camera at MediaMTX, as the browser must call it: scheme and domain from the settings
/// (`MEDIAMTX_WEBRTC_SCHEME`, `MEDIAMTX_WEBRTC_DOMAIN`), the host being the request's if the configured domain is one of the
/// instance's own hosts (see `dominet_domain::public_url`). A scheme other than `http` / `https` is replaced by `http`.
pub async fn whep_url(
    state: &crate::AppState,
    uuid: CameraUuid,
    request_host: Option<&str>,
) -> String {
    let (scheme, domain) = (
        state
            .settings
            .get_string("MEDIAMTX_WEBRTC_SCHEME")
            .await
            .ok(),
        state
            .settings
            .get_string("MEDIAMTX_WEBRTC_DOMAIN")
            .await
            .ok(),
    );
    whep_url_from(
        scheme.as_deref(),
        domain.as_deref(),
        request_host,
        &state.config.allowed_hosts,
        uuid,
    )
}

/// Core of [`whep_url`], without access to the settings.
#[must_use]
pub fn whep_url_from(
    scheme: Option<&str>,
    domain: Option<&str>,
    request_host: Option<&str>,
    allowed_hosts: &[String],
    uuid: CameraUuid,
) -> String {
    let scheme = scheme
        .map(str::trim)
        .filter(|s| matches!(*s, "http" | "https"))
        .unwrap_or("http");
    let domain = domain
        .map(|d| d.trim().trim_end_matches('/'))
        .filter(|d| !d.is_empty())
        .unwrap_or("localhost:8889");
    let own = dominet_domain::public_url::own_hosts(allowed_hosts);
    let base = dominet_domain::public_url::public_url(domain, request_host, &own);
    format!("{scheme}://{base}/{uuid}/whep")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid() -> CameraUuid {
        CameraUuid(Uuid::from_u128(0xabcd))
    }

    #[test]
    fn the_whep_url_follows_the_settings_and_the_request_host() {
        let hosts = vec!["dominet.local".to_string(), "192.168.1.2".to_string()];
        let id = "0000000000000000000000000000abcd";
        assert_eq!(
            whep_url_from(
                Some("http"),
                Some("dominet.local:8889"),
                Some("192.168.1.2"),
                &hosts,
                uuid()
            ),
            format!("http://192.168.1.2:8889/{id}/whep")
        );
        // Without a request or a setting: the reference's default values.
        assert_eq!(
            whep_url_from(None, None, None, &[], uuid()),
            format!("http://localhost:8889/{id}/whep")
        );
        // Unknown scheme refused, trailing slash removed.
        assert_eq!(
            whep_url_from(Some("javascript"), Some("h:1/"), None, &[], uuid()),
            format!("http://h:1/{id}/whep")
        );
        assert_eq!(
            whep_url_from(
                Some("https"),
                Some("dominet.local/webrtc"),
                None,
                &hosts,
                uuid()
            ),
            format!("https://dominet.local/webrtc/{id}/whep")
        );
    }
}
