// SPDX-License-Identifier: AGPL-3.0-or-later
//! Object and motion detection (R6 lot 8, ADR 0050), `video` role.
//!
//! - **Motion**: every `MOTION_INTERVAL_SECS`, for each active camera in `motion` mode, the first and the last image of the
//!   `FrameHub` sliding window (20 images, ~10 s at 2 images/s) are compared as the reference does (`dominet_surveillance::motion`).
//!   Beyond `MOTION_THRESHOLD`: a `MotionEvent`, an MQTT message `dominet/surveillance/<uuid>/motion`, a notification (per-camera delay
//!   `MOTION_ALERT_COOLDOWN`), then YOLO if the camera enables it.
//! - **YOLO**: one image (the camera's most recent), the global detector enabled in `DetectorConfig`, the camera's class filter and
//!   confidence threshold, annotated image if `save_frames`, `DetectionEvent`, MQTT message `.../detection`, notification (delay
//!   `DETECTION_ALERT_COOLDOWN`). The model is only loaded at the first detection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dominet_domain::camera::{
    Camera, CameraUuid, Detection, DetectionEvent, DetectionTrigger, DetectorConfig, RecordingMode,
};
use dominet_domain::repo::{DetectorConfigRepo, EventRepo};
use dominet_surveillance::motion::motion_ratio;
use dominet_surveillance::yolo::{self, YoloDetector};

use crate::AppState;

/// An image older than this is no longer the camera: neither motion nor detection.
const MAX_FRAME_AGE: Duration = Duration::from_secs(15);
/// Minimum images in the window to judge a motion (like the reference: `len(frames) < 5`).
const MIN_WINDOW: usize = 5;

/// Shared detector state: model loaded on demand, per-camera notification delays.
#[derive(Clone, Default)]
pub struct Detector {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    loaded: Mutex<Option<(PathBuf, usize, Arc<YoloDetector>)>>,
    motion_alert: Mutex<HashMap<CameraUuid, Instant>>,
    detection_alert: Mutex<HashMap<CameraUuid, Instant>>,
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detector").finish_non_exhaustive()
    }
}

/// Result of a motion evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionOutcome {
    /// Not enough recent images (stream absent, too old, window too short).
    NoWindow,
    /// Comparison done, below the threshold.
    Still(f64),
    /// Motion: event recorded.
    Moving(f64),
}

fn publish(state: &AppState, camera: &Camera, kind: &str, payload: &serde_json::Value) {
    let Some(mqtt) = &state.mqtt else { return };
    let topic = format!("dominet/surveillance/{}/{kind}", camera.uuid);
    if let Err(e) = mqtt.publish(&topic, payload.to_string(), 0, false) {
        tracing::warn!(camera = %camera.uuid, topic, error = %e, "publication MQTT impossible");
    }
}

/// Notifies (e-mail and SMS) unless the camera already was within `cooldown`.
async fn notify_unless_recent(
    state: &AppState,
    camera: &Camera,
    alerts: &Mutex<HashMap<CameraUuid, Instant>>,
    cooldown_secs: u32,
    subject_and_message: impl FnOnce(&str) -> (String, String),
) {
    if !cooldown_passed(
        alerts,
        camera.uuid,
        Duration::from_secs(u64::from(cooldown_secs)),
    ) {
        return;
    }
    let label = camera_label(camera, &locations(state).await);
    let (subject, message) = subject_and_message(&label);
    for (channel, why) in state
        .channels
        .notify_with_subject(&subject, &message, true)
        .await
    {
        tracing::warn!(camera = %camera.uuid, channel, error = %why, "notification not sent");
    }
}

/// True (and the delay restarts) if the camera has not been notified within `cooldown`.
fn cooldown_passed(
    map: &Mutex<HashMap<CameraUuid, Instant>>,
    camera: CameraUuid,
    cooldown: Duration,
) -> bool {
    let mut map = map.lock().unwrap();
    let now = Instant::now();
    if map
        .get(&camera)
        .is_some_and(|last| now.duration_since(*last) < cooldown)
    {
        return false;
    }
    map.insert(camera, now);
    true
}

fn camera_label(camera: &Camera, locations: &HashMap<i64, String>) -> String {
    match camera
        .config
        .location_id
        .and_then(|id| locations.get(&id))
        .filter(|l| !l.is_empty())
    {
        Some(place) => format!("{} ({place})", camera.config.name),
        None => camera.config.name.clone(),
    }
}

async fn locations(state: &AppState) -> HashMap<i64, String> {
    use dominet_domain::repo::LocationRepo;
    state
        .store
        .locations()
        .list()
        .await
        .map(|l| l.into_iter().map(|l| (l.id.0, l.name)).collect())
        .unwrap_or_default()
}

impl Detector {
    /// Evaluates a camera's motion over its window of images; if the threshold is exceeded: event, MQTT, notification, YOLO.
    pub async fn check_motion(&self, state: &AppState, camera: &Camera) -> MotionOutcome {
        let frames = state.frames.recent(camera.uuid);
        let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
            return MotionOutcome::NoWindow;
        };
        let fresh = (Utc::now() - last.at)
            .to_std()
            .is_ok_and(|age| age < MAX_FRAME_AGE);
        if frames.len() < MIN_WINDOW || !fresh {
            return MotionOutcome::NoWindow;
        }
        let (a, b) = (first.jpeg.clone(), last.jpeg.clone());
        let ratio = match tokio::task::spawn_blocking(move || motion_ratio(&a, &b)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(camera = %camera.uuid, error = %e, "motion: unusable frames");
                return MotionOutcome::NoWindow;
            }
            Err(_) => return MotionOutcome::NoWindow,
        };
        if ratio <= state.config.motion_threshold {
            return MotionOutcome::Still(ratio);
        }
        crate::metrics::motion(&camera.uuid.to_string());
        tracing::info!(camera = %camera.uuid, ratio_pct = format!("{:.1}", ratio * 100.0), "motion");
        let now = Utc::now();
        if let Err(e) = state
            .store
            .camera_events()
            .record_motion(camera.id, now, ratio)
            .await
        {
            tracing::warn!(camera = %camera.uuid, error = %e, "motion not saved");
        }
        publish(
            state,
            camera,
            "motion",
            &serde_json::json!({
                "camera_uuid": camera.uuid.to_string(),
                "camera_name": camera.config.name,
                "confidence": (ratio * 10_000.0).round() / 10_000.0,
                "timestamp": now.to_rfc3339(),
            }),
        );
        notify_unless_recent(
            state,
            camera,
            &self.inner.motion_alert,
            state.config.motion_alert_cooldown_secs,
            |label| {
                (
                    format!("[DomiNet] Motion - {label}"),
                    format!("Motion detected \u{2014} {label}: {:.1}%", ratio * 100.0),
                )
            },
        )
        .await;
        if camera.config.yolo_enabled {
            self.detect(state, camera, DetectionTrigger::Motion).await;
        }
        MotionOutcome::Moving(ratio)
    }

    /// The loaded (or to be loaded) detector for this model; `None` if the model is not found or invalid.
    async fn detector(&self, state: &AppState, model: PathBuf) -> Option<Arc<YoloDetector>> {
        let workers = usize::try_from(state.config.yolo_workers).unwrap_or(1);
        if let Some((path, w, d)) = self.inner.loaded.lock().unwrap().as_ref()
            && *path == model
            && *w == workers
        {
            return Some(d.clone());
        }
        let load_from = model.clone();
        match tokio::task::spawn_blocking(move || YoloDetector::load(&load_from, workers)).await {
            Ok(Ok(d)) => {
                let d = Arc::new(d);
                tracing::info!(model = %model.display(), workers, "detection model loaded");
                *self.inner.loaded.lock().unwrap() = Some((model, workers, d.clone()));
                Some(d)
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "detection model unavailable (make fetch-model)");
                None
            }
            Err(_) => None,
        }
    }

    /// Runs the model on the camera's last image: detector setting, recent image, inference (off the async thread).
    async fn infer(
        &self,
        state: &AppState,
        camera: &Camera,
    ) -> Option<(DetectorConfig, Arc<Vec<u8>>, Vec<yolo::Det>)> {
        if !camera.config.yolo_enabled {
            return None;
        }
        let config: DetectorConfig = state.store.detector_config().get().await.ok()?;
        if !config.enabled {
            return None;
        }
        let model = if config.model_path.trim().is_empty() {
            PathBuf::from(&state.config.yolo_model_path)
        } else {
            PathBuf::from(config.model_path.trim())
        };
        let frame = state.frames.recent(camera.uuid).pop()?;
        if (Utc::now() - frame.at)
            .to_std()
            .map_or(true, |age| age > MAX_FRAME_AGE)
        {
            tracing::warn!(camera = %camera.uuid, "detection: no recent frame");
            return None;
        }
        let detector = self.detector(state, model).await?;
        let jpeg = frame.jpeg.clone();
        let found = match tokio::task::spawn_blocking({
            let d = detector.clone();
            let j = jpeg.clone();
            move || d.detect_jpeg(&j)
        })
        .await
        {
            Ok(Ok(found)) => found,
            Ok(Err(e)) => {
                tracing::warn!(camera = %camera.uuid, error = %e, "detection impossible");
                return None;
            }
            Err(_) => return None,
        };
        Some((config, jpeg, found))
    }

    /// Detection on the camera's last image. Returns the event created (`None`: detector off, no image, nothing kept).
    pub async fn detect(
        &self,
        state: &AppState,
        camera: &Camera,
        trigger: DetectionTrigger,
    ) -> Option<DetectionEvent> {
        let (config, jpeg, found) = self.infer(state, camera).await?;
        let retained = filter(
            &found,
            &camera.config.yolo_classes,
            camera.config.yolo_min_confidence,
        );
        if retained.is_empty() {
            return None;
        }
        let now = Utc::now();
        let frame_path = if config.save_frames {
            save_annotated(
                Path::new(&state.config.detections_dir),
                camera.uuid,
                now,
                &jpeg,
                &retained,
            )
            .await
        } else {
            String::new()
        };
        let labels: Vec<Detection> = retained
            .iter()
            .map(|d| Detection {
                class: d.class.to_string(),
                confidence: (f64::from(d.confidence) * 1000.0).round() / 1000.0,
                bbox: d.bbox.map(f64::from),
            })
            .collect();
        let event = state
            .store
            .camera_events()
            .record_detection(camera.id, now, trigger, &labels, &frame_path)
            .await
            .map_err(|e| tracing::warn!(camera = %camera.uuid, error = %e, "detection not saved"))
            .ok()?;
        crate::metrics::detection(&camera.uuid.to_string());
        let classes = event.class_names().join(", ");
        tracing::info!(camera = %camera.uuid, classes, trigger = trigger.as_str(), "detection");
        publish(
            state,
            camera,
            "detection",
            &serde_json::json!({
                "camera_uuid": camera.uuid.to_string(),
                "camera_name": camera.config.name,
                "detections": labels.iter().map(|d| serde_json::json!({
                    "class": d.class, "confidence": d.confidence, "bbox": d.bbox,
                })).collect::<Vec<_>>(),
                "timestamp": now.to_rfc3339(),
            }),
        );
        notify_unless_recent(
            state,
            camera,
            &self.inner.detection_alert,
            state.config.detection_alert_cooldown_secs,
            |label| {
                (
                    format!("[DomiNet] D\u{e9}tection {classes} - {label}"),
                    format!("D\u{e9}tection \u{2014} {label} : {classes}"),
                )
            },
        )
        .await;
        Some(event)
    }
}

/// Keeps the detections the camera asked for: minimum confidence and, if the list is not empty, wanted classes.
#[must_use]
pub fn filter<'a>(
    found: &'a [yolo::Det],
    wanted: &[String],
    min_confidence: f64,
) -> Vec<&'a yolo::Det> {
    found
        .iter()
        .filter(|d| f64::from(d.confidence) >= min_confidence)
        .filter(|d| wanted.is_empty() || wanted.iter().any(|w| w == d.class))
        .collect()
}

/// Writes the annotated image `<uuid>_<YYYYMMDD_HHMMSS>.jpg`; returns its name (empty on failure, the detection stays recorded).
async fn save_annotated(
    dir: &Path,
    camera: CameraUuid,
    at: DateTime<Utc>,
    jpeg: &Arc<Vec<u8>>,
    detections: &[&yolo::Det],
) -> String {
    let owned: Vec<yolo::Det> = detections.iter().map(|d| (*d).clone()).collect();
    let source = jpeg.clone();
    let Ok(Ok(annotated)) =
        tokio::task::spawn_blocking(move || yolo::annotate(&source, &owned)).await
    else {
        return String::new();
    };
    let name = format!("{camera}_{}.jpg", at.format("%Y%m%d_%H%M%S"));
    if tokio::fs::create_dir_all(dir).await.is_err()
        || tokio::fs::write(dir.join(&name), annotated).await.is_err()
    {
        tracing::warn!(dir = %dir.display(), "annotated image not written");
        return String::new();
    }
    name
}

/// `video` role: evaluates the motion of each camera in `motion` mode, every `MOTION_INTERVAL_SECS`.
pub fn spawn_role(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        let every = Duration::from_secs(u64::from(state.config.motion_interval_secs));
        loop {
            tokio::time::sleep(every).await;
            let cameras = match state.cameras.list().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "motion: unreadable cameras");
                    continue;
                }
            };
            for camera in cameras
                .iter()
                .filter(|c| c.config.active && c.config.recording_mode == RecordingMode::Motion)
            {
                state.detector.check_motion(&state, camera).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(class: &'static str, confidence: f32) -> yolo::Det {
        yolo::Det {
            class,
            confidence,
            bbox: [0.0; 4],
        }
    }

    #[test]
    fn the_camera_filters_by_confidence_and_by_class() {
        let found = [det("person", 0.8), det("car", 0.6), det("dog", 0.3)];
        let names = |v: Vec<&yolo::Det>| v.iter().map(|d| d.class).collect::<Vec<_>>();
        assert_eq!(names(filter(&found, &[], 0.5)), ["person", "car"]);
        assert_eq!(
            names(filter(&found, &["car".into(), "dog".into()], 0.5)),
            ["car"]
        );
        assert_eq!(filter(&found, &[], 0.9).len(), 0);
        // The threshold is inclusive (>=), like the reference.
        assert_eq!(filter(&[det("cat", 0.5)], &[], 0.5).len(), 1);
    }

    #[test]
    fn the_cooldown_is_per_camera_and_lets_the_first_through() {
        let map = Mutex::new(HashMap::new());
        let (a, b): (CameraUuid, CameraUuid) = (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse().unwrap(),
        );
        let long = Duration::from_secs(3600);
        assert!(cooldown_passed(&map, a, long));
        assert!(!cooldown_passed(&map, a, long));
        assert!(cooldown_passed(&map, b, long));
        assert!(cooldown_passed(&map, a, Duration::ZERO));
    }
}
