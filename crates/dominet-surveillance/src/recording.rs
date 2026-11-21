// SPDX-License-Identifier: AGPL-3.0-or-later
//! Segments enregistres par MediaMTX (R6 lot 7, ADR 0049) : `<dossier>/<uuid>/<AAAA-MM-JJ_HH-MM-SS-ffffff>.mp4`.
//!
//! MediaMTX ecrit un segment fMP4 en continu (une partie par seconde environ) : tant que le fichier grossit, sa date de modification
//! avance. Un segment est **termine** quand elle n'a plus bouge depuis `grace` (15 s comme la reference). Ecart voulu avec reference, qui
//! ignorait toujours le dernier fichier : ici le dernier segment d'une camera dont l'enregistrement s'est arrete est aussi pris.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDateTime, Utc};
use dominet_domain::camera::CameraUuid;

/// Un segment termine, pret a etre indexe puis archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub camera: CameraUuid,
    pub path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub size_bytes: u64,
    /// Duree reelle (derniere ecriture - debut), `None` si la date de modification precede le debut (horloge).
    pub duration: Option<Duration>,
}

/// Date de debut d'un segment d'apres son nom (`2026-11-08_12-00-00-123456.mp4`, avec ou sans fraction), en UTC comme MediaMTX
/// dans son conteneur et comme la reference.
#[must_use]
pub fn parse_segment_time(file_name: &str) -> Option<DateTime<Utc>> {
    let stem = Path::new(file_name).file_stem()?.to_str()?;
    // `%f` de MediaMTX = microsecondes (jusqu'a 6 chiffres) ; chrono lirait des nanosecondes, on les lit donc a part.
    let (whole, micros) = match stem.split_once('-').and(stem.get(19..)) {
        Some(rest) if stem.len() > 19 => {
            let digits = rest.strip_prefix('-')?;
            if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            (
                stem.get(..19)?,
                digits.parse::<u32>().ok()? * 10_u32.pow(6 - u32::try_from(digits.len()).ok()?),
            )
        }
        _ => (stem, 0),
    };
    let naive = NaiveDateTime::parse_from_str(whole, "%Y-%m-%d_%H-%M-%S").ok()?;
    Some(naive.and_utc() + chrono::Duration::microseconds(i64::from(micros)))
}

/// Les segments termines de toutes les cameras (dossiers nommes d'un uuid), du plus ancien au plus recent.
/// Un fichier au nom illisible prend la date de sa derniere modification (comme la reference).
///
/// # Errors
/// Le dossier racine est illisible (un dossier de camera illisible est ignore).
pub fn finished_segments(
    root: &Path,
    now: SystemTime,
    grace: Duration,
) -> std::io::Result<Vec<Segment>> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(root)? {
        let Ok(dir) = dir else { continue };
        let Some(camera) = dir
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<CameraUuid>().ok())
        else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("mp4") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if now.duration_since(modified).unwrap_or_default() < grace {
                continue; // encore en ecriture
            }
            let started_at = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(parse_segment_time)
                .unwrap_or_else(|| DateTime::<Utc>::from(modified));
            let duration = DateTime::<Utc>::from(modified)
                .signed_duration_since(started_at)
                .to_std()
                .ok();
            out.push(Segment {
                camera,
                path,
                started_at,
                size_bytes: meta.len(),
                duration,
            });
        }
    }
    out.sort_by_key(|s| (s.started_at, s.camera.to_string()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs::{File, FileTimes};

    use super::*;

    const CAM: &str = "5ca0e51964bd43c3bfebec36be21afc3";

    /// Dossier jetable (supprime a la fin du test).
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "dominet-seg-{tag}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn make(root: &Path, cam: &str, name: &str, modified: SystemTime, bytes: usize) {
        let dir = root.join(cam);
        std::fs::create_dir_all(&dir).unwrap();
        let file = File::create(dir.join(name)).unwrap();
        file.set_len(bytes as u64).unwrap();
        file.set_times(FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[test]
    fn names_are_read_with_or_without_fraction() {
        let t = parse_segment_time("2026-11-08_12-30-05-250000.mp4").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-11-08T12:30:05.250+00:00");
        assert_eq!(
            parse_segment_time("2026-11-08_12-30-05.mp4")
                .unwrap()
                .timestamp(),
            t.timestamp()
        );
        assert!(parse_segment_time("segment.mp4").is_none());
    }

    #[test]
    fn only_cooled_segments_of_known_cameras_are_returned_in_order() {
        let tmp = Tmp::new("t1");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(120);
        make(tmp.path(), CAM, "2026-11-08_12-01-00-000000.mp4", old, 20);
        make(tmp.path(), CAM, "2026-11-08_12-00-00-000000.mp4", old, 10);
        // Encore en ecriture : ignore.
        make(tmp.path(), CAM, "2026-11-08_12-02-00-000000.mp4", now, 5);
        // Pas un mp4, pas une camera.
        make(tmp.path(), CAM, "notes.txt", old, 1);
        make(
            tmp.path(),
            "pas-une-camera",
            "2026-11-08_12-00-00-000000.mp4",
            old,
            1,
        );

        let found = finished_segments(tmp.path(), now, Duration::from_secs(15)).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|s| s.path.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "2026-11-08_12-00-00-000000.mp4",
                "2026-11-08_12-01-00-000000.mp4"
            ]
        );
        assert_eq!(found[0].size_bytes, 10);
        assert_eq!(found[0].camera.to_string(), CAM);
    }

    #[test]
    fn the_last_segment_of_a_stopped_camera_is_taken_too() {
        let tmp = Tmp::new("t2");
        let now = SystemTime::now();
        make(
            tmp.path(),
            CAM,
            "2026-11-08_12-00-00-000000.mp4",
            now - Duration::from_secs(30),
            3,
        );
        assert_eq!(
            finished_segments(tmp.path(), now, Duration::from_secs(15))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_missing_root_is_an_error_and_a_bad_name_falls_back_to_the_modification_date() {
        assert!(
            finished_segments(
                Path::new("/nonexistent-dominet"),
                SystemTime::now(),
                Duration::ZERO
            )
            .is_err()
        );
        let tmp = Tmp::new("t3");
        let old = SystemTime::now() - Duration::from_secs(100);
        make(tmp.path(), CAM, "bizarre.mp4", old, 1);
        let found = finished_segments(tmp.path(), SystemTime::now(), Duration::ZERO).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].started_at.timestamp(),
            DateTime::<Utc>::from(old).timestamp()
        );
    }
}
