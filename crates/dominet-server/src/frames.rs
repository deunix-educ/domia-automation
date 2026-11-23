// SPDX-License-Identifier: AGPL-3.0-or-later
//! `video` role (R6 lot 6, ADR 0048): one supervised `ffmpeg` per **active** camera, which reads MediaMTX's re-stream and produces low-rate
//! JPEG images.
//!
//! - The sources follow the list of cameras: addition, deactivation, deletion (the role re-reads the database at each change signalled
//!   by `AppState::cameras_changed`, and every 30 s as a precaution).
//! - The **last image** of each camera is written to `<SNAPSHOTS_DIR>/latest/<uuid>.jpg` at most once per second (atomic
//!   write: temporary file then rename). It is the interface with the `web` role, **which may be another process**: the
//!   snapshot, the gallery and the fallback display of a browser without H.264 depend on it. Motion and YOLO (lots 7 and 8) read
//!   the live images through [`FrameHub::source`].

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dominet_domain::camera::{Camera, CameraUuid};
use dominet_surveillance::frames::{FfmpegFrameSource, Frame, FrameConfig, FrameSource, Hwaccel};
use tokio::task::JoinHandle;

use crate::AppState;
use crate::config::Config;

/// The last image is only written once per second, even if the source gives more.
const WRITE_EVERY: Duration = Duration::from_secs(1);
const RECONCILE_EVERY: Duration = Duration::from_secs(30);
/// Images kept per camera for motion: the reference's sliding window (~20 images).
pub const RECENT_FRAMES: usize = 20;

type Recent = Arc<Mutex<VecDeque<Frame>>>;

/// How to read a camera: `ffmpeg`'s input arguments (`None`: no stream to decode).
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

/// Folder of each camera's last image.
#[must_use]
pub fn latest_dir(snapshots_dir: &Path) -> PathBuf {
    snapshots_dir.join("latest")
}

impl FrameHub {
    /// Reads `<MEDIAMTX_RTSP_URL>/<uuid>`; without this address, no source starts.
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

    /// Chosen factory (tests: a `lavfi` source instead of the re-stream).
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

    /// Does a source decode this camera?
    #[must_use]
    pub fn has(&self, uuid: CameraUuid) -> bool {
        self.sources.lock().unwrap().contains_key(&uuid)
    }

    /// A camera's last images (at most [`RECENT_FRAMES`]), from oldest to most recent: the motion window.
    #[must_use]
    pub fn recent(&self, uuid: CameraUuid) -> Vec<Frame> {
        let sources = self.sources.lock().unwrap();
        sources
            .get(&uuid)
            .map(|r| r.recent.lock().unwrap().iter().cloned().collect())
            .unwrap_or_default()
    }

    /// A camera's source (motion, detection).
    #[must_use]
    pub fn source(&self, uuid: CameraUuid) -> Option<Arc<FfmpegFrameSource>> {
        self.sources
            .lock()
            .unwrap()
            .get(&uuid)
            .map(|r| r.source.clone())
    }

    /// Matches the sources to the cameras: starts those of the active cameras that have none, stops the others.
    pub fn reconcile(&self, cameras: &[Camera]) {
        let Some(factory) = &self.factory else { return };
        let mut sources = self.sources.lock().unwrap();
        let wanted: Vec<&Camera> = cameras.iter().filter(|c| c.config.active).collect();
        // Stops (and erases the last image of) what is no longer wanted.
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

    /// Stops all the sources.
    pub fn stop_all(&self) {
        self.sources.lock().unwrap().clear();
    }
}

/// Writes the source's last image to `latest/<uuid>.jpg`, at most once per second, atomically.
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
            // Read lag: we resume at the next image; source closed: end.
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

/// `video` role: follows the list of cameras until the process stops.
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
