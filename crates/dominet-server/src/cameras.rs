// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service des cameras (R6, ADR 0043 et 0044) : enregistrement avec chiffrement des secrets, synchronisation avec MediaMTX.
//!
//! - Les mots de passe (RTSP, ONVIF, web) et la cle d'API web sont chiffres avec `SecretBox`, la **donnee associee** etant
//!   `camera:<uuid>:<champ>` : un chiffre recopie dans une autre camera ou un autre champ ne se dechiffre pas.
//! - A la modification, un secret **non fourni** reste inchange (l'interface ne renvoie jamais un secret).
//! - Apres chaque ecriture, le path MediaMTX de la camera est cree, remplace ou supprime ; un echec de MediaMTX ne defait pas
//!   l'enregistrement (la base fait foi, `sync_all` rattrape), il est rapporte.

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

/// Un secret de formulaire : `Keep` (champ laisse vide) ou une nouvelle valeur en clair.
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
    /// Une saisie vide veut dire « inchange ».
    #[must_use]
    pub fn from_form(s: &str) -> Self {
        if s.is_empty() {
            Self::Keep
        } else {
            Self::Set(s.to_string())
        }
    }
}

/// Ce que l'administration envoie : les reglages (les secrets du brouillon sont ignores) et les secrets en clair.
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
    /// La camera n'a pas ONVIF active.
    #[error("ONVIF n'est pas active sur cette camera")]
    OnvifDisabled,
    /// Aucun service ONVIF n'a repondu sur les ports usuels.
    #[error("aucun service ONVIF ne repond a cette adresse")]
    NoOnvifService,
    #[error(transparent)]
    Onvif(#[from] OnvifError),
    #[error(transparent)]
    Night(#[from] NightError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// Etat de la synchronisation MediaMTX apres une ecriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// `MEDIAMTX_API_URL` n'est pas renseignee.
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
    /// Dernier etat envoye a chaque bascule Thingino (la camera ne le relit pas).
    night_state: NightState,
    /// Prevenu a chaque creation, modification ou suppression (le role `video` relit ses flux).
    changed: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Dossier des snapshots : retire avec la camera.
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

    /// Previent `changed` apres chaque ecriture.
    #[must_use]
    pub fn with_changes(mut self, changed: std::sync::Arc<tokio::sync::Notify>) -> Self {
        self.changed = Some(changed);
        self
    }

    /// Dossier des segments enregistres, dont le sous-dossier de la camera est supprime avec elle (apres l'arret de son path MediaMTX).
    #[must_use]
    pub fn with_recordings_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.recordings_dir = Some(dir);
        self
    }

    /// Dossier des snapshots, dont le sous-dossier de la camera est supprime avec elle.
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

    /// MediaMTX est-il configure ?
    #[must_use]
    pub fn has_mediamtx(&self) -> bool {
        self.mediamtx.is_some()
    }

    /// Toutes les cameras, par priorite puis par nom.
    ///
    /// # Errors
    /// Erreur du depot.
    pub async fn list(&self) -> Result<Vec<Camera>, CameraError> {
        Ok(self.store.cameras().list().await?)
    }

    /// # Errors
    /// Erreur du depot.
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

    /// Secret en clair ; vide s'il n'est pas renseigne. A ne jamais journaliser ni renvoyer a l'interface.
    ///
    /// # Errors
    /// `Unreadable` si la cle maitresse manque ou a change.
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

    /// Cree (`id` absent) ou remplace (`id` present) une camera, puis met son path MediaMTX a jour.
    ///
    /// # Errors
    /// `Invalid` champ par champ, `NotFound`, `MasterKeyMissing` si un secret est fourni sans cle maitresse, erreur du depot.
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
        // Ce que la synchronisation ONVIF a appris n'est pas dans le formulaire : on le garde.
        if let Some(c) = current {
            draft.manufacturer.clone_from(&c.manufacturer);
            draft.model.clone_from(&c.model);
            draft.firmware.clone_from(&c.firmware);
            // Un jeton saisi dans le formulaire l'emporte ; vide : on garde celui de la synchronisation.
            if draft.main_token.is_empty() {
                draft.main_token.clone_from(&c.main_token);
            }
        }
        // Comme la reference : une camera ONVIF sans identifiants propres reprend ceux du flux RTSP.
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

    /// Supprime la camera (et, en cascade, ce qui lui appartient) puis son path MediaMTX.
    ///
    /// # Errors
    /// `NotFound` ou erreur du depot.
    pub async fn delete(&self, id: CameraId) -> Result<SyncStatus, CameraError> {
        let camera = self.get(id).await?.ok_or(CameraError::NotFound)?;
        self.store.cameras().delete(id).await.map_err(|e| match e {
            RepoError::NotFound => CameraError::NotFound,
            other => CameraError::Repo(other),
        })?;
        // Les fichiers de la camera partent avec elle (les lignes de la base sont supprimees en cascade).
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
        // Le path est arrete : MediaMTX n'ecrit plus dans le dossier.
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

    /// Synchronisation complete : un path par camera **active**, les paths de camera inconnus sont retires.
    ///
    /// # Errors
    /// `Disabled` sans `MEDIAMTX_API_URL`, MediaMTX qui ne repond pas, erreur du depot.
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
        // Une camera dont le mot de passe est illisible garde son path actuel : on ne le supprime pas par erreur.
        let keep: Vec<String> = unreadable.iter().map(|(n, _)| n.clone()).collect();
        let mut report = api.sync_keeping(&wanted, &keep).await?;
        report.failed.extend(unreadable);
        Ok(report)
    }

    /// Client ONVIF d'une camera : ses identifiants ONVIF, ou ceux du flux a defaut (comme la reference).
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

    /// Bascule jour / nuit, IR-cut ou lumiere d'une camera **active** par la methode configuree (`night_mode_api`).
    ///
    /// # Errors
    /// `NotFound` (inconnue ou inactive), `Night(NotConfigured)`, action inconnue, secret illisible, erreur de la camera.
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
        // Identifiants ONVIF, a defaut ceux du flux (comme la reference).
        let (onvif_user, onvif_pass) = if d.onvif_user.is_empty() {
            (d.rtsp_user.clone(), rtsp_pass.clone())
        } else {
            (d.onvif_user.clone(), open("onvif_pass", &d.onvif_pass)?)
        };
        let (mut web_user, mut web_pass) = (d.web_user.clone(), open("web_pass", &d.web_pass)?);
        if d.night_mode_api == NightApi::Goform && web_user.is_empty() {
            // Goform : identifiants web, a defaut ceux du flux.
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

    /// Synchronisation ONVIF : lit identite, profil principal et PTZ de la camera et les enregistre.
    ///
    /// # Errors
    /// `OnvifDisabled`, identifiants refuses, camera injoignable, erreur du depot.
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
                    // Le PTZ est une decision de l'administrateur : ONVIF ne le propose qu'a la premiere synchronisation.
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

    /// Ajout par adresse IP (decouverte, ou camera que le multicast n'atteint pas). Si une camera a deja cette adresse, elle est
    /// renvoyee telle quelle (`false`). Sinon le port ONVIF est cherche (`onvif_port` fourni, ou les ports usuels), la camera est
    /// creee avec ONVIF active et, si des identifiants sont fournis, synchronisee tout de suite (un echec n'annule pas l'ajout).
    ///
    /// # Errors
    /// Adresse invalide, `NoOnvifService`, erreur du depot.
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

/// Adresse WHEP d'une camera chez MediaMTX, telle que le navigateur doit l'appeler : schema et domaine des reglages
/// (`MEDIAMTX_WEBRTC_SCHEME`, `MEDIAMTX_WEBRTC_DOMAIN`), l'hote etant celui de la requete si le domaine configure est un hote propre de
/// l'instance (voir `dominet_domain::public_url`). Un schema autre que `http` / `https` est remplace par `http`.
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

/// Coeur de [`whep_url`], sans acces aux reglages.
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
        // Sans requete ni reglage : les valeurs par defaut de la reference.
        assert_eq!(
            whep_url_from(None, None, None, &[], uuid()),
            format!("http://localhost:8889/{id}/whep")
        );
        // Schema inconnu refuse, barre finale retiree.
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
