// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recordings (R6 lot 7, ADR 0049): MediaMTX writes fMP4 segments into `RECORDINGS_DIR`; the `video` role **indexes** them (`recording`
//! table), **pushes** them into ReductStore with the reference implementation's labels, then **deletes the local file** (the volume no
//! longer grows, which the reference did not do). The `web` role serves them: local file if it still exists, otherwise ReductStore.
//!
//! Type of a segment: `manual` if the camera is in manual mode, `motion` if a motion falls in its window, `continuous` otherwise
//! (like the reference). Without ReductStore configured, the segments stay on disk (`pending`) and are served from it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::camera::{
    Camera, CameraId, CameraUuid, NewRecording, Recording, RecordingEvent, RecordingMode,
};
use dominet_domain::repo::{CameraRepo, EventRepo, RecordingRepo};
use dominet_store::PgStore;
use dominet_store::video::ReductVideo;
use dominet_surveillance::MediaMtx;
use dominet_surveillance::recording::finished_segments;

/// Segments handled per archiving pass (the rest waits for the next pass).
const ARCHIVE_BATCH: i64 = 50;

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("camera not found")]
    NotFound,
    #[error("la camera n'est pas en mode d'enregistrement manuel")]
    NotManual,
    #[error("MediaMTX n'est pas configure")]
    NoMediaMtx,
    #[error("MediaMTX : {0}")]
    MediaMtx(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// Outcome of a pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    pub indexed: usize,
    pub archived: usize,
    pub failed: usize,
}

/// The content of a segment to serve.
#[derive(Debug)]
pub enum SegmentBody {
    Local(PathBuf),
    Memory(Vec<u8>),
}

#[derive(Clone)]
pub struct RecordingService {
    store: PgStore,
    video: Option<Arc<ReductVideo>>,
    mediamtx: Option<MediaMtx>,
    dir: PathBuf,
    segment: Duration,
    grace: Duration,
    warned_dir: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for RecordingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingService")
            .field("dir", &self.dir)
            .field("archive", &self.video.is_some())
            .finish_non_exhaustive()
    }
}

impl RecordingService {
    #[must_use]
    pub fn new(
        store: PgStore,
        video: Option<ReductVideo>,
        mediamtx: Option<MediaMtx>,
        dir: PathBuf,
        segment: Duration,
        grace: Duration,
    ) -> Self {
        Self {
            store,
            video: video.map(Arc::new),
            mediamtx,
            dir,
            segment,
            grace,
            warned_dir: Arc::default(),
        }
    }

    /// Is the ReductStore archive configured?
    #[must_use]
    pub fn has_archive(&self) -> bool {
        self.video.is_some()
    }

    /// A full pass: indexes the finished segments, then archives what is not yet archived.
    ///
    /// # Errors
    /// Repository error (the list of cameras or segments is not readable).
    pub async fn scan(&self) -> Result<ScanReport, RecordingError> {
        let indexed = self.index_new(SystemTime::now()).await?;
        let (archived, failed) = self.archive_pending().await?;
        Ok(ScanReport {
            indexed,
            archived,
            failed,
        })
    }

    /// Records in the database the finished segments of the active cameras that record (like the reference: a `none` or inactive camera is ignored).
    ///
    /// # Errors
    /// Repository error.
    pub async fn index_new(&self, now: SystemTime) -> Result<usize, RecordingError> {
        let (dir, grace) = (self.dir.clone(), self.grace);
        let found = tokio::task::spawn_blocking(move || finished_segments(&dir, now, grace))
            .await
            .ok()
            .and_then(Result::ok);
        let Some(found) = found else {
            // A single alert (the scan comes back every minute); the information comes back as soon as the folder exists.
            if !self
                .warned_dir
                .swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                tracing::warn!(dir = %self.dir.display(), "unreadable recordings folder");
            }
            return Ok(0);
        };
        self.warned_dir
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let mut created = 0;
        for segment in found {
            let path = segment.path.to_string_lossy().into_owned();
            if self.store.recordings().path_known(&path).await? {
                continue;
            }
            let Some(camera) = self.store.cameras().get_by_uuid(segment.camera).await? else {
                continue;
            };
            if !camera.config.active || camera.config.recording_mode == RecordingMode::None {
                continue;
            }
            let event_type = self.event_type(&camera, segment.started_at).await?;
            let new = NewRecording {
                started_at: segment.started_at,
                segment_path: path,
                event_type,
                size_bytes: i64::try_from(segment.size_bytes).ok(),
                duration_ms: segment
                    .duration
                    .and_then(|d| i64::try_from(d.as_millis()).ok()),
            };
            match self.store.recordings().create(camera.id, &new).await {
                Ok(_) => created += 1,
                // Indexed meanwhile by another pass: of no importance.
                Err(RepoError::Conflict { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if created > 0 {
            tracing::info!(segments = created, "recording segments indexed");
        }
        Ok(created)
    }

    async fn event_type(
        &self,
        camera: &Camera,
        started_at: DateTime<Utc>,
    ) -> Result<RecordingEvent, RecordingError> {
        if camera.config.recording_mode == RecordingMode::Manual {
            return Ok(RecordingEvent::Manual);
        }
        let end = started_at
            + chrono::Duration::from_std(self.segment)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));
        let motion = self
            .store
            .camera_events()
            .motion_between(camera.id, started_at, end)
            .await?;
        Ok(if motion {
            RecordingEvent::Motion
        } else {
            RecordingEvent::Continuous
        })
    }

    /// Pushes the unarchived segments to ReductStore then deletes the local file. A failure is noted on the segment (`error`) and
    /// retried at the next pass. Returns (archived, failures).
    ///
    /// # Errors
    /// Repository error (unreadable list).
    pub async fn archive_pending(&self) -> Result<(usize, usize), RecordingError> {
        let Some(video) = &self.video else {
            return Ok((0, 0));
        };
        let (mut done, mut failed) = (0, 0);
        for rec in self
            .store
            .recordings()
            .list_unarchived(ARCHIVE_BATCH)
            .await?
        {
            match self.archive_one(video, &rec).await {
                Ok(()) => done += 1,
                Err(message) => {
                    failed += 1;
                    tracing::warn!(segment = rec.id.0, error = %message, "archiving a segment failed");
                    self.store.recordings().mark_error(rec.id, &message).await?;
                }
            }
        }
        Ok((done, failed))
    }

    async fn archive_one(&self, video: &ReductVideo, rec: &Recording) -> Result<(), String> {
        let camera = self
            .store
            .cameras()
            .get(rec.camera)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("camera deleted")?;
        self.store
            .recordings()
            .mark_uploading(rec.id)
            .await
            .map_err(|e| e.to_string())?;
        let path = PathBuf::from(&rec.segment_path);
        let data = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("unreadable file: {e}"))?;
        let size = i64::try_from(data.len()).ok();
        video
            .push(
                camera.uuid,
                &camera.config.name,
                rec.started_at,
                rec.event_type,
                data,
            )
            .await
            .map_err(|e| format!("ReductStore : {e}"))?;
        self.store
            .recordings()
            .mark_done(
                rec.id,
                dominet_store::video::timestamp_us(rec.started_at).cast_signed(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let _ = size;
        // Safe archive: the local file no longer has any use (D10).
        if let Err(e) = tokio::fs::remove_file(&path).await {
            tracing::warn!(path = %path.display(), error = %e, "local file not removed after archiving");
        }
        Ok(())
    }

    /// A segment's content: the local file if it still exists (and stays under `RECORDINGS_DIR`), otherwise ReductStore.
    ///
    /// # Errors
    /// ReductStore unreachable.
    pub async fn open(
        &self,
        camera: CameraUuid,
        rec: &Recording,
    ) -> Result<Option<SegmentBody>, String> {
        let local = PathBuf::from(&rec.segment_path);
        if is_under(&local, &self.dir) && tokio::fs::metadata(&local).await.is_ok() {
            return Ok(Some(SegmentBody::Local(local)));
        }
        let (Some(video), Some(ts)) = (&self.video, rec.reduct_ts) else {
            return Ok(None);
        };
        let ts = u64::try_from(ts).map_err(|_| "invalid timestamp".to_string())?;
        video
            .read(camera, ts)
            .await
            .map(|d| d.map(SegmentBody::Memory))
            .map_err(|e| format!("ReductStore : {e}"))
    }

    /// Deletes a segment: local file, archive and database row.
    ///
    /// # Errors
    /// ReductStore unreachable (nothing is then deleted) or repository error.
    pub async fn delete(&self, camera: CameraUuid, rec: &Recording) -> Result<(), String> {
        if let (Some(video), Some(ts)) = (&self.video, rec.reduct_ts)
            && let Ok(ts) = u64::try_from(ts)
        {
            video
                .remove(camera, ts)
                .await
                .map_err(|e| format!("ReductStore : {e}"))?;
        }
        let local = PathBuf::from(&rec.segment_path);
        if is_under(&local, &self.dir) {
            let _ = tokio::fs::remove_file(&local).await;
        }
        self.store
            .recordings()
            .delete(rec.id)
            .await
            .map_err(|e| e.to_string())
    }

    /// Manual recording in progress? (`None`: MediaMTX does not say or is not configured.)
    pub async fn manual_state(&self, camera: CameraUuid) -> Option<bool> {
        let conf = self
            .mediamtx
            .as_ref()?
            .get(&camera.to_string())
            .await
            .ok()??;
        Some(conf["record"].as_bool().unwrap_or(false))
    }

    /// Starts or stops the recording of a camera in manual mode (`PATCH record` on the MediaMTX path).
    ///
    /// # Errors
    /// Unknown camera, not in manual mode, MediaMTX absent or down.
    pub async fn set_manual(&self, id: CameraId, on: bool) -> Result<(), RecordingError> {
        let camera = self
            .store
            .cameras()
            .get(id)
            .await?
            .ok_or(RecordingError::NotFound)?;
        if camera.config.recording_mode != RecordingMode::Manual {
            return Err(RecordingError::NotManual);
        }
        let api = self.mediamtx.as_ref().ok_or(RecordingError::NoMediaMtx)?;
        api.set_record(&camera.uuid.to_string(), on)
            .await
            .map_err(|e| RecordingError::MediaMtx(e.to_string()))?;
        tracing::info!(camera = %camera.uuid, on, "manual recording");
        Ok(())
    }
}

/// `video` role: indexes and archives the segments every `RECORDINGS_SCAN_SECS`.
pub fn spawn_role(state: &crate::AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        let every = Duration::from_secs(u64::from(state.config.recordings_scan_secs));
        if !state.recordings.has_archive() {
            tracing::warn!(
                "video role: REDUCTSTORE_URL missing, segments stay on disk (not archived)"
            );
        }
        loop {
            match state.recordings.scan().await {
                Ok(r) if r.indexed + r.archived + r.failed > 0 => {
                    crate::metrics::recordings(r.indexed, r.archived, r.failed);
                    tracing::info!(
                        indexes = r.indexed,
                        archives = r.archived,
                        echecs = r.failed,
                        "recordings"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    crate::metrics::recordings(0, 0, 1);
                    tracing::warn!(error = %e, "recordings: pass failed");
                }
            }
            tokio::time::sleep(every).await;
        }
    });
}

/// Is `path` under `root` (without `..`)?
fn is_under(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        && path
            .components()
            .all(|c| !matches!(c, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_the_recordings_dir_are_served() {
        let root = Path::new("/recordings");
        assert!(is_under(Path::new("/recordings/a/b.mp4"), root));
        assert!(!is_under(Path::new("/etc/passwd"), root));
        assert!(!is_under(Path::new("/recordings/../etc/passwd"), root));
    }
}
