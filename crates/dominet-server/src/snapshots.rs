// SPDX-License-Identifier: AGPL-3.0-or-later
//! Snapshots (R6 lot 6, ADR 0048): the **last image** of each camera (written by the `video` role) and the **indexed** snapshots (gallery).
//!
//! The `web` role decodes nothing: it reads `<SNAPSHOTS_DIR>/latest/<uuid>.jpg`, which the `video` role renews every second (the two
//! roles may be two processes). An indexed snapshot is copied to `<SNAPSHOTS_DIR>/<uuid>/<timestamp>.jpg` and recorded in the database;
//! beyond `SNAPSHOTS_MAX_PER_CAMERA` per camera, the oldest are deleted (an addition compared with the reference, which had no limit).

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::camera::{Camera, CameraId, CameraUuid, Snapshot, SnapshotId};
use dominet_domain::repo::SnapshotRepo;
use dominet_store::PgStore;

/// A last image older than this is no longer the camera: we do not serve it.
pub const STALE_AFTER: Duration = Duration::from_secs(60);
/// A snapshot is a JPEG of a few hundred KiB; 20 MiB is already an error.
const MAX_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("aucune image recente de cette camera (le role video tourne-t-il ?)")]
    NoImage,
    #[error("image invalide")]
    Invalid,
    #[error("ecriture du fichier impossible")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// An image read from disk and its age.
#[derive(Debug, Clone)]
pub struct LatestImage {
    pub bytes: Vec<u8>,
    pub age: Duration,
    modified: SystemTime,
}

#[derive(Clone, Debug)]
pub struct SnapshotService {
    store: PgStore,
    dir: PathBuf,
    max_per_camera: i64,
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes.starts_with(&[0xFF, 0xD8]) && bytes.ends_with(&[0xFF, 0xD9])
}

/// A relative path without `..`, without a root: the one we ourselves wrote in the database.
fn safe_relative(file: &str) -> Option<&Path> {
    let p = Path::new(file);
    (!file.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))).then_some(p)
}

impl SnapshotService {
    #[must_use]
    pub fn new(store: PgStore, dir: PathBuf, max_per_camera: u32) -> Self {
        Self {
            store,
            dir,
            max_per_camera: i64::from(max_per_camera),
        }
    }

    fn latest_path(&self, uuid: CameraUuid) -> PathBuf {
        crate::frames::latest_dir(&self.dir).join(format!("{uuid}.jpg"))
    }

    async fn read_latest(&self, uuid: CameraUuid) -> Option<LatestImage> {
        let path = self.latest_path(uuid);
        let modified = tokio::fs::metadata(&path).await.ok()?.modified().ok()?;
        let bytes = tokio::fs::read(&path).await.ok()?;
        is_jpeg(&bytes).then(|| LatestImage {
            age: SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default(),
            bytes,
            modified,
        })
    }

    /// The camera's last image, if it is less than [`STALE_AFTER`] old.
    pub async fn latest(&self, uuid: CameraUuid) -> Option<LatestImage> {
        self.read_latest(uuid).await.filter(|i| i.age < STALE_AFTER)
    }

    /// An image **later than this call** (waiting at most `timeout`, the `video` role writes one per second); failing that, the
    /// last image that is still recent.
    pub async fn fresh(&self, uuid: CameraUuid, timeout: Duration) -> Option<LatestImage> {
        let asked = SystemTime::now();
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if let Some(img) = self.read_latest(uuid).await
                && img.modified >= asked
            {
                return Some(img);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.latest(uuid).await
    }

    /// Records a snapshot: file, database row, then deletion of the oldest beyond the camera's cap.
    ///
    /// # Errors
    /// `Invalid` (not a JPEG, or too big), `Io`, repository error.
    pub async fn index(
        &self,
        camera: &Camera,
        at: DateTime<Utc>,
        jpeg: &[u8],
    ) -> Result<Snapshot, SnapshotError> {
        if !is_jpeg(jpeg) || jpeg.len() > MAX_BYTES {
            return Err(SnapshotError::Invalid);
        }
        let relative = format!("{}/{}.jpg", camera.uuid, at.format("%Y%m%d_%H%M%S_%3f"));
        let path = self.dir.join(&relative);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, jpeg).await?;
        let size = i32::try_from(jpeg.len()).unwrap_or(i32::MAX);
        let snapshot = match self
            .store
            .snapshots()
            .create(camera.id, at, &relative, size)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(e.into());
            }
        };
        self.prune(camera.id).await;
        Ok(snapshot)
    }

    /// Capture: fresh image (or the last recent one) then [`Self::index`].
    ///
    /// # Errors
    /// `NoImage` without a recent image, then those of [`Self::index`].
    pub async fn capture(&self, camera: &Camera) -> Result<Snapshot, SnapshotError> {
        let image = self
            .fresh(camera.uuid, Duration::from_secs(4))
            .await
            .ok_or(SnapshotError::NoImage)?;
        self.index(camera, Utc::now(), &image.bytes).await
    }

    async fn prune(&self, camera: CameraId) {
        let Ok(excess) = self
            .store
            .snapshots()
            .excess(camera, self.max_per_camera)
            .await
        else {
            return;
        };
        for s in excess {
            let _ = self.delete(s.id).await;
        }
    }

    /// Indexed snapshots, most recent first.
    ///
    /// # Errors
    /// Repository error.
    pub async fn list(
        &self,
        camera: Option<CameraId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Snapshot>, SnapshotError> {
        Ok(self
            .store
            .snapshots()
            .list_filtered(camera, from, to, limit)
            .await?)
    }

    /// An indexed snapshot and its file (`None` if it is unknown or if its file has disappeared).
    ///
    /// # Errors
    /// Repository error.
    pub async fn read(&self, id: SnapshotId) -> Result<Option<(Snapshot, Vec<u8>)>, SnapshotError> {
        let Some(s) = self.store.snapshots().get(id).await? else {
            return Ok(None);
        };
        let Some(relative) = safe_relative(&s.file) else {
            return Ok(None);
        };
        Ok(tokio::fs::read(self.dir.join(relative))
            .await
            .ok()
            .map(|b| (s, b)))
    }

    /// Deletes the snapshot (row and file). An already absent file is not an error.
    ///
    /// # Errors
    /// `Repo(NotFound)` if it does not exist.
    pub async fn delete(&self, id: SnapshotId) -> Result<(), SnapshotError> {
        let s = self
            .store
            .snapshots()
            .get(id)
            .await?
            .ok_or(RepoError::NotFound)?;
        self.store.snapshots().delete(id).await?;
        if let Some(relative) = safe_relative(&s.file) {
            let _ = tokio::fs::remove_file(self.dir.join(relative)).await;
        }
        Ok(())
    }
}

/// The current day `YYYY-MM-DD` in time zone `tz`.
#[must_use]
pub fn local_today(tz: &str) -> String {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    Utc::now().with_timezone(&tz).format("%Y-%m-%d").to_string()
}

/// An instant written in RFC 3339 in time zone `tz` (server local time, for displaying the recordings).
#[must_use]
pub fn local_rfc3339(tz: &str, at: DateTime<Utc>) -> String {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    at.with_timezone(&tz).to_rfc3339()
}

/// UTC bounds of a day `YYYY-MM-DD` read in time zone `tz` (from midnight to midnight); `(None, None)` if the text is not a day.
#[must_use]
pub fn day_bounds(tz: &str, day: &str) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    use chrono::{NaiveDate, TimeZone};
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    let Ok(d) = NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d") else {
        return (None, None);
    };
    let at_midnight = |d: NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|t| tz.from_local_datetime(&t).earliest())
            .map(|t| t.with_timezone(&Utc))
    };
    (at_midnight(d), d.succ_opt().and_then(at_midnight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_followed() {
        assert!(safe_relative("abc/20260101_000000_000.jpg").is_some());
        for bad in ["", "/etc/passwd", "../x.jpg", "a/../../x"] {
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_day_is_read_in_the_servers_time_zone() {
        // Paris in winter: UTC+1, so 2 January starts on the 1st at 23:00 UTC.
        let (from, to) = day_bounds("Europe/Paris", "2026-01-02");
        assert_eq!(from.unwrap().to_rfc3339(), "2026-01-01T23:00:00+00:00");
        assert_eq!(to.unwrap().to_rfc3339(), "2026-01-02T23:00:00+00:00");
        // Clock change: the day lasts 23 h.
        let (from, to) = day_bounds("Europe/Paris", "2026-03-29");
        assert_eq!((to.unwrap() - from.unwrap()).num_hours(), 23);
        assert_eq!(day_bounds("Europe/Paris", "pas un jour"), (None, None));
        assert_eq!(
            day_bounds("Fuseau/Inconnu", "2026-01-02")
                .0
                .unwrap()
                .to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );
    }

    #[test]
    fn only_complete_jpegs_pass() {
        assert!(is_jpeg(&[0xFF, 0xD8, 1, 2, 0xFF, 0xD9]));
        assert!(!is_jpeg(b"<html>"));
        assert!(!is_jpeg(&[0xFF, 0xD8, 1, 2, 3]));
        assert!(!is_jpeg(&[]));
    }
}
