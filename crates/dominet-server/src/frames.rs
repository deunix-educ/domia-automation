// SPDX-License-Identifier: AGPL-3.0-or-later
//! Role `video` (R6 lot 6, ADR 0048) : un `ffmpeg` supervise par camera **active**, qui lit le re-stream de MediaMTX et produit des images
//! JPEG a faible cadence.
//!
//! - Les sources suivent la liste des cameras : ajout, desactivation, suppression (le role relit la base a chaque changement signale
//!   par `AppState::cameras_changed`, et toutes les 30 s par precaution).
//! - La **derniere image** de chaque camera est ecrite dans `<SNAPSHOTS_DIR>/latest/<uuid>.jpg` au plus une fois par seconde (ecriture
//!   atomique : fichier temporaire puis renommage). C'est l'interface avec le role `web`, **qui peut etre un autre processus** : le
//!   snapshot, la galerie et l'affichage de repli d'un navigateur sans H.264 en dependent. Le mouvement et YOLO (lots 7 et 8) lisent
//!   les images en direct par [`FrameHub::source`].

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dominet_domain::camera::{Camera, CameraUuid};
use dominet_surveillance::frames::{FfmpegFrameSource, Frame, FrameConfig, FrameSource, Hwaccel};
use tokio::task::JoinHandle;

use crate::AppState;
use crate::config::Config;

/// La derniere image n'est ecrite qu'une fois par seconde, meme si la source en donne plus.
const WRITE_EVERY: Duration = Duration::from_secs(1);
const RECONCILE_EVERY: Duration = Duration::from_secs(30);
/// Images gardees par camera pour le mouvement : la fenetre glissante de la reference (~20 images).
pub const RECENT_FRAMES: usize = 20;

type Recent = Arc<Mutex<VecDeque<Frame>>>;

/// Comment lire une camera : les arguments d'entree de `ffmpeg` (`None` : pas de flux a decoder).
type Factory = Arc<dyn Fn(&Camera) -> Option<FrameConfig> + Send + Sync>;

struct Running {
    source: Arc<FfmpegFrameSource>,
    recent: Recent,
    writer: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.writer.abort();
    }
}

#[derive(Clone)]
pub struct FrameHub {
    sources: Arc<Mutex<HashMap<CameraUuid, Running>>>,
    factory: Option<Factory>,
    latest_dir: PathBuf,
}

impl std::fmt::Debug for FrameHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameHub")
            .field("sources", &self.sources.lock().map_or(0, |s| s.len()))
            .finish_non_exhaustive()
    }
}

/// Dossier de la derniere image de chaque camera.
#[must_use]
pub fn latest_dir(snapshots_dir: &Path) -> PathBuf {
    snapshots_dir.join("latest")
}

impl FrameHub {
    /// Lit `<MEDIAMTX_RTSP_URL>/<uuid>` ; sans cette adresse, aucune source ne demarre.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let base = config.mediamtx_rtsp_url.clone();
        let hw = if config.ffmpeg_hwaccel_auto {
            Hwaccel::Auto
        } else {
            Hwaccel::Off
        };
        let factory: Option<Factory> = base.map(|base| {
            let base = base.trim_end_matches('/').to_string();
            Arc::new(move |c: &Camera| {
                let mut cfg =
                    FrameConfig::new(FrameConfig::rtsp_input(&format!("{base}/{}", c.uuid)));
                cfg.hwaccel = hw;
                Some(cfg)
            }) as Factory
        });
        Self::new(factory, latest_dir(Path::new(&config.snapshots_dir)))
    }

    /// Fabrique choisie (tests : une source `lavfi` au lieu du re-stream).
    #[must_use]
    pub fn with_factory(
        factory: impl Fn(&Camera) -> Option<FrameConfig> + Send + Sync + 'static,
        snapshots_dir: &Path,
    ) -> Self {
        Self::new(Some(Arc::new(factory)), latest_dir(snapshots_dir))
    }

    fn new(factory: Option<Factory>, latest_dir: PathBuf) -> Self {
        Self {
            sources: Arc::default(),
            factory,
            latest_dir,
        }
    }

    /// Une source decode-t-elle cette camera ?
    #[must_use]
    pub fn has(&self, uuid: CameraUuid) -> bool {
        self.sources.lock().unwrap().contains_key(&uuid)
    }

    /// Les dernieres images d'une camera (au plus [`RECENT_FRAMES`]), de la plus ancienne a la plus recente : fenetre du mouvement.
    #[must_use]
    pub fn recent(&self, uuid: CameraUuid) -> Vec<Frame> {
        let sources = self.sources.lock().unwrap();
        sources
            .get(&uuid)
            .map(|r| r.recent.lock().unwrap().iter().cloned().collect())
            .unwrap_or_default()
    }

    /// La source d'une camera (mouvement, detection).
    #[must_use]
    pub fn source(&self, uuid: CameraUuid) -> Option<Arc<FfmpegFrameSource>> {
        self.sources
            .lock()
            .unwrap()
            .get(&uuid)
            .map(|r| r.source.clone())
    }

    /// Fait correspondre les sources aux cameras : demarre celles des cameras actives qui n'en ont pas, arrete les autres.
    pub fn reconcile(&self, cameras: &[Camera]) {
        let Some(factory) = &self.factory else { return };
        let mut sources = self.sources.lock().unwrap();
        let wanted: Vec<&Camera> = cameras.iter().filter(|c| c.config.active).collect();
        // Arrete (et efface la derniere image de) ce qui n'est plus voulu.
        let gone: Vec<CameraUuid> = sources
            .keys()
            .copied()
            .filter(|u| !wanted.iter().any(|c| c.uuid == *u))
            .collect();
        for uuid in gone {
            sources.remove(&uuid);
            let _ = std::fs::remove_file(self.latest_dir.join(format!("{uuid}.jpg")));
            tracing::info!(camera = %uuid, "frame stream stopped");
        }
        for camera in wanted {
            if sources.contains_key(&camera.uuid) {
                continue;
            }
            let Some(cfg) = factory(camera) else { continue };
            let source = Arc::new(FfmpegFrameSource::spawn(cfg));
            let recent: Recent = Arc::default();
            let writer = tokio::spawn(write_latest(
                source.clone(),
                recent.clone(),
                self.latest_dir.clone(),
                camera.uuid,
            ));
            tracing::info!(camera = %camera.uuid, "frame stream started");
            sources.insert(
                camera.uuid,
                Running {
                    source,
                    recent,
                    writer,
                },
            );
        }
    }

    /// Arrete toutes les sources.
    pub fn stop_all(&self) {
        self.sources.lock().unwrap().clear();
    }
}

/// Ecrit la derniere image de la source dans `latest/<uuid>.jpg`, au plus une fois par seconde, de facon atomique.
async fn write_latest(
    source: Arc<FfmpegFrameSource>,
    recent: Recent,
    dir: PathBuf,
    uuid: CameraUuid,
) {
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        tracing::warn!(dir = %dir.display(), "cannot create the latest-frames folder");
        return;
    }
    let (path, tmp) = (
        dir.join(format!("{uuid}.jpg")),
        dir.join(format!(".{uuid}.tmp")),
    );
    let mut rx = source.subscribe();
    let mut last_write = tokio::time::Instant::now() - WRITE_EVERY;
    loop {
        let Ok(frame) = rx.recv().await else {
            // Retard de lecture : on reprend a la prochaine image ; source fermee : fin.
            if matches!(
                rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Closed)
            ) {
                return;
            }
            continue;
        };
        {
            let mut window = recent.lock().unwrap();
            if window.len() >= RECENT_FRAMES {
                window.pop_front();
            }
            window.push_back(frame.clone());
        }
        if last_write.elapsed() < WRITE_EVERY {
            continue;
        }
        last_write = tokio::time::Instant::now();
        if tokio::fs::write(&tmp, frame.jpeg.as_slice()).await.is_ok()
            && let Err(e) = tokio::fs::rename(&tmp, &path).await
        {
            tracing::warn!(camera = %uuid, error = %e, "latest frame not written");
        }
    }
}

/// Role `video` : suit la liste des cameras jusqu'a l'arret du processus.
pub fn spawn_role(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        if state.config.mediamtx_rtsp_url.is_none() {
            tracing::warn!("video role: MEDIAMTX_RTSP_URL missing, no frame will be decoded");
        }
        loop {
            match state.cameras.list().await {
                Ok(cameras) => state.frames.reconcile(&cameras),
                Err(e) => tracing::warn!(error = %e, "video role: unreadable cameras"),
            }
            tokio::select! {
                () = state.cameras_changed.notified() => {}
                () = tokio::time::sleep(RECONCILE_EVERY) => {}
            }
        }
    });
}
