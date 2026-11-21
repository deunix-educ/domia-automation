// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots de la surveillance (R6, ADR 0043) : cameras, presets PTZ, enregistrements, snapshots, evenements, detecteur.
//!
//! Les secrets (mots de passe, cle d'API) arrivent et repartent **chiffres** : ce module ne chiffre ni ne dechiffre.

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::camera::{
    Camera, CameraDraft, CameraId, CameraUuid, Detection, DetectionEvent, DetectionEventId,
    DetectionTrigger, DetectorConfig, MotionEvent, MotionEventId, NewRecording, OnvifInfo,
    PtzPreset, PtzPresetDraft, PtzPresetId, Recording, RecordingId, RecordingMode, Sealed,
    Snapshot, SnapshotId,
};
use dominet_domain::repo::{
    CameraRepo, DetectorConfigRepo, EventRepo, PtzPresetRepo, RecordingRepo, RepoResult,
    SnapshotRepo,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::pg::{corrupt, repo_err};

macro_rules! pg_struct {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone)]
        pub struct $name {
            pool: sqlx::PgPool,
        }
        impl $name {
            pub(crate) fn new(pool: sqlx::PgPool) -> Self {
                Self { pool }
            }
        }
    )*};
}
pg_struct!(
    PgCameras,
    PgPtzPresets,
    PgRecordings,
    PgSnapshots,
    PgCameraEvents,
    PgDetectorConfig
);

fn parse<T: std::str::FromStr<Err = dominet_domain::ValidationError>>(
    what: &str,
    text: &str,
) -> RepoResult<T> {
    text.parse()
        .map_err(|_| corrupt(format!("{what} {text:?}")))
}

// ------------------------------------------------------------------ cameras

#[allow(
    clippy::struct_excessive_bools,
    reason = "one row of the `camera` table, column for column"
)]
struct CameraRow {
    id: i64,
    uuid: Uuid,
    name: String,
    location_id: Option<i64>,
    active: bool,
    priority: i32,
    last_seen: Option<DateTime<Utc>>,
    notes: String,
    proto: String,
    ip: String,
    port: i32,
    path: String,
    rtsp_user: String,
    rtsp_pass_enc: String,
    fps: i32,
    rtsp_any_port: bool,
    rtsp_transport_tcp: bool,
    recording_mode: String,
    night_mode_api: String,
    web_port: i32,
    web_api_key_enc: String,
    web_user: String,
    web_pass_enc: String,
    onvif: bool,
    onvif_port: i32,
    onvif_user: String,
    onvif_pass_enc: String,
    main_token: String,
    ptz: bool,
    yolo_enabled: bool,
    yolo_classes: Vec<String>,
    yolo_min_confidence: f64,
    manufacturer: String,
    model: String,
    firmware: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<CameraRow> for Camera {
    type Error = RepoError;

    fn try_from(r: CameraRow) -> Result<Self, RepoError> {
        Ok(Self {
            id: CameraId(r.id),
            uuid: CameraUuid(r.uuid),
            last_seen: r.last_seen,
            created_at: r.created_at,
            config: CameraDraft {
                name: r.name,
                location_id: r.location_id,
                active: r.active,
                priority: r.priority,
                notes: r.notes,
                proto: parse("protocol", &r.proto)?,
                ip: r.ip,
                port: r.port,
                path: r.path,
                rtsp_user: r.rtsp_user,
                rtsp_pass: Sealed(r.rtsp_pass_enc),
                fps: r.fps,
                rtsp_any_port: r.rtsp_any_port,
                rtsp_transport_tcp: r.rtsp_transport_tcp,
                recording_mode: parse("recording mode", &r.recording_mode)?,
                night_mode_api: parse("day/night mode", &r.night_mode_api)?,
                web_port: r.web_port,
                web_api_key: Sealed(r.web_api_key_enc),
                web_user: r.web_user,
                web_pass: Sealed(r.web_pass_enc),
                onvif: r.onvif,
                onvif_port: r.onvif_port,
                onvif_user: r.onvif_user,
                onvif_pass: Sealed(r.onvif_pass_enc),
                main_token: r.main_token,
                ptz: r.ptz,
                yolo_enabled: r.yolo_enabled,
                yolo_classes: r.yolo_classes,
                yolo_min_confidence: r.yolo_min_confidence,
                manufacturer: r.manufacturer,
                model: r.model,
                firmware: r.firmware,
            },
        })
    }
}

fn done(rows: u64) -> RepoResult<()> {
    if rows == 0 {
        Err(RepoError::NotFound)
    } else {
        Ok(())
    }
}

impl CameraRepo for PgCameras {
    async fn list(&self) -> RepoResult<Vec<Camera>> {
        sqlx::query_as!(
            CameraRow,
            "SELECT * FROM camera ORDER BY priority, name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(Camera::try_from)
        .collect()
    }

    async fn get(&self, id: CameraId) -> RepoResult<Option<Camera>> {
        sqlx::query_as!(CameraRow, "SELECT * FROM camera WHERE id = $1", id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?
            .map(Camera::try_from)
            .transpose()
    }

    async fn get_by_uuid(&self, uuid: CameraUuid) -> RepoResult<Option<Camera>> {
        sqlx::query_as!(CameraRow, "SELECT * FROM camera WHERE uuid = $1", uuid.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?
            .map(Camera::try_from)
            .transpose()
    }

    async fn create(&self, uuid: Option<CameraUuid>, d: &CameraDraft) -> RepoResult<Camera> {
        sqlx::query_as!(
            CameraRow,
            "INSERT INTO camera (uuid, name, location_id, active, priority, notes, proto, ip, port, path, rtsp_user, rtsp_pass_enc,
                 fps, rtsp_any_port, rtsp_transport_tcp, recording_mode, night_mode_api, web_port, web_api_key_enc, web_user,
                 web_pass_enc, onvif, onvif_port, onvif_user, onvif_pass_enc, main_token, ptz, yolo_enabled, yolo_classes,
                 yolo_min_confidence, manufacturer, model, firmware)
             VALUES (COALESCE($33, gen_random_uuid()), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                 $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32)
             RETURNING *",
            d.name,
            d.location_id,
            d.active,
            d.priority,
            d.notes,
            d.proto.as_str(),
            d.ip,
            d.port,
            d.path,
            d.rtsp_user,
            d.rtsp_pass.0,
            d.fps,
            d.rtsp_any_port,
            d.rtsp_transport_tcp,
            d.recording_mode.as_str(),
            d.night_mode_api.as_str(),
            d.web_port,
            d.web_api_key.0,
            d.web_user,
            d.web_pass.0,
            d.onvif,
            d.onvif_port,
            d.onvif_user,
            d.onvif_pass.0,
            d.main_token,
            d.ptz,
            d.yolo_enabled,
            &d.yolo_classes,
            d.yolo_min_confidence,
            d.manufacturer,
            d.model,
            d.firmware,
            uuid.map(|u| u.0),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .try_into()
    }

    async fn update(&self, id: CameraId, d: &CameraDraft) -> RepoResult<Camera> {
        sqlx::query_as!(
            CameraRow,
            "UPDATE camera SET name = $2, location_id = $3, active = $4, priority = $5, notes = $6, proto = $7, ip = $8,
                 port = $9, path = $10, rtsp_user = $11, rtsp_pass_enc = $12, fps = $13, rtsp_any_port = $14,
                 rtsp_transport_tcp = $15, recording_mode = $16, night_mode_api = $17, web_port = $18, web_api_key_enc = $19,
                 web_user = $20, web_pass_enc = $21, onvif = $22, onvif_port = $23, onvif_user = $24, onvif_pass_enc = $25,
                 main_token = $26, ptz = $27, yolo_enabled = $28, yolo_classes = $29, yolo_min_confidence = $30,
                 manufacturer = $31, model = $32, firmware = $33
             WHERE id = $1 RETURNING *",
            id.0,
            d.name,
            d.location_id,
            d.active,
            d.priority,
            d.notes,
            d.proto.as_str(),
            d.ip,
            d.port,
            d.path,
            d.rtsp_user,
            d.rtsp_pass.0,
            d.fps,
            d.rtsp_any_port,
            d.rtsp_transport_tcp,
            d.recording_mode.as_str(),
            d.night_mode_api.as_str(),
            d.web_port,
            d.web_api_key.0,
            d.web_user,
            d.web_pass.0,
            d.onvif,
            d.onvif_port,
            d.onvif_user,
            d.onvif_pass.0,
            d.main_token,
            d.ptz,
            d.yolo_enabled,
            &d.yolo_classes,
            d.yolo_min_confidence,
            d.manufacturer,
            d.model,
            d.firmware,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?
        .try_into()
    }

    async fn delete(&self, id: CameraId) -> RepoResult<()> {
        let r = sqlx::query!("DELETE FROM camera WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn touch_last_seen(&self, id: CameraId, at: DateTime<Utc>) -> RepoResult<()> {
        let r = sqlx::query!("UPDATE camera SET last_seen = $2 WHERE id = $1", id.0, at)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn apply_onvif(&self, id: CameraId, info: &OnvifInfo) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE camera SET manufacturer = $2, model = $3, firmware = $4, main_token = $5, ptz = $6 WHERE id = $1",
            id.0,
            info.manufacturer,
            info.model,
            info.firmware,
            info.main_token,
            info.ptz
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn set_recording_mode(&self, id: CameraId, mode: RecordingMode) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE camera SET recording_mode = $2 WHERE id = $1",
            id.0,
            mode.as_str()
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn set_yolo_enabled(&self, id: CameraId, enabled: bool) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE camera SET yolo_enabled = $2 WHERE id = $1",
            id.0,
            enabled
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }
}

// ------------------------------------------------------------------ presets PTZ

struct PresetRow {
    id: i64,
    camera_id: i64,
    name: String,
    token: String,
    pan: f64,
    tilt: f64,
    zoom: f64,
    position: i16,
}

impl From<PresetRow> for PtzPreset {
    fn from(r: PresetRow) -> Self {
        Self {
            id: PtzPresetId(r.id),
            camera: CameraId(r.camera_id),
            draft: PtzPresetDraft {
                name: r.name,
                token: r.token,
                pan: r.pan,
                tilt: r.tilt,
                zoom: r.zoom,
                position: r.position,
            },
        }
    }
}

impl PtzPresetRepo for PgPtzPresets {
    async fn list(&self, camera: CameraId) -> RepoResult<Vec<PtzPreset>> {
        let rows = sqlx::query_as!(
            PresetRow,
            "SELECT id, camera_id, name, token, pan, tilt, zoom, position FROM ptz_preset
             WHERE camera_id = $1 ORDER BY position, name",
            camera.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(PtzPreset::from).collect())
    }

    async fn create(&self, camera: CameraId, d: &PtzPresetDraft) -> RepoResult<PtzPreset> {
        let row = sqlx::query_as!(
            PresetRow,
            "INSERT INTO ptz_preset (camera_id, name, token, pan, tilt, zoom, position) VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, camera_id, name, token, pan, tilt, zoom, position",
            camera.0,
            d.name,
            d.token,
            d.pan,
            d.tilt,
            d.zoom,
            d.position
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn upsert(&self, camera: CameraId, d: &PtzPresetDraft) -> RepoResult<PtzPreset> {
        let row = sqlx::query_as!(
            PresetRow,
            "INSERT INTO ptz_preset (camera_id, name, token, pan, tilt, zoom, position) VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (camera_id, name) DO UPDATE SET token = EXCLUDED.token, pan = EXCLUDED.pan, tilt = EXCLUDED.tilt,
                 zoom = EXCLUDED.zoom
             RETURNING id, camera_id, name, token, pan, tilt, zoom, position",
            camera.0,
            d.name,
            d.token,
            d.pan,
            d.tilt,
            d.zoom,
            d.position
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn get(&self, id: PtzPresetId) -> RepoResult<Option<PtzPreset>> {
        let row = sqlx::query_as!(
            PresetRow,
            "SELECT id, camera_id, name, token, pan, tilt, zoom, position FROM ptz_preset WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(PtzPreset::from))
    }

    async fn delete(&self, id: PtzPresetId) -> RepoResult<()> {
        let r = sqlx::query!("DELETE FROM ptz_preset WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        done(r.rows_affected())
    }
}

// ------------------------------------------------------------------ enregistrements

struct RecordingRow {
    id: i64,
    camera_id: i64,
    started_at: DateTime<Utc>,
    duration_ms: Option<i64>,
    segment_path: String,
    reduct_ts: Option<i64>,
    event_type: String,
    status: String,
    error: String,
    size_bytes: Option<i64>,
}

impl TryFrom<RecordingRow> for Recording {
    type Error = RepoError;

    fn try_from(r: RecordingRow) -> Result<Self, RepoError> {
        Ok(Self {
            id: RecordingId(r.id),
            camera: CameraId(r.camera_id),
            started_at: r.started_at,
            duration_ms: r.duration_ms,
            segment_path: r.segment_path,
            reduct_ts: r.reduct_ts,
            event_type: parse("recording type", &r.event_type)?,
            status: parse("recording status", &r.status)?,
            error: r.error,
            size_bytes: r.size_bytes,
        })
    }
}

impl RecordingRepo for PgRecordings {
    async fn create(&self, camera: CameraId, n: &NewRecording) -> RepoResult<Recording> {
        sqlx::query_as!(
            RecordingRow,
            "INSERT INTO recording (camera_id, started_at, segment_path, event_type, size_bytes, duration_ms)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, camera_id, started_at, duration_ms, segment_path, reduct_ts, event_type, status, error, size_bytes",
            camera.0,
            n.started_at,
            n.segment_path,
            n.event_type.as_str(),
            n.size_bytes,
            n.duration_ms
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .try_into()
    }

    async fn get(&self, id: RecordingId) -> RepoResult<Option<Recording>> {
        sqlx::query_as!(
            RecordingRow,
            "SELECT id, camera_id, started_at, duration_ms, segment_path, reduct_ts, event_type, status, error, size_bytes
             FROM recording WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(Recording::try_from)
        .transpose()
    }

    async fn list(&self, camera: CameraId, limit: i64) -> RepoResult<Vec<Recording>> {
        sqlx::query_as!(
            RecordingRow,
            "SELECT id, camera_id, started_at, duration_ms, segment_path, reduct_ts, event_type, status, error, size_bytes
             FROM recording WHERE camera_id = $1 ORDER BY started_at DESC, id DESC LIMIT $2",
            camera.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(Recording::try_from)
        .collect()
    }

    async fn list_between(
        &self,
        camera: CameraId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> RepoResult<Vec<Recording>> {
        sqlx::query_as!(
            RecordingRow,
            "SELECT id, camera_id, started_at, duration_ms, segment_path, reduct_ts, event_type, status, error, size_bytes
             FROM recording WHERE camera_id = $1 AND started_at >= $2 AND started_at < $3 ORDER BY started_at, id",
            camera.0,
            from,
            to
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(Recording::try_from)
        .collect()
    }

    async fn list_unarchived(&self, limit: i64) -> RepoResult<Vec<Recording>> {
        sqlx::query_as!(
            RecordingRow,
            "SELECT id, camera_id, started_at, duration_ms, segment_path, reduct_ts, event_type, status, error, size_bytes
             FROM recording WHERE status <> 'done' ORDER BY started_at, id LIMIT $1",
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(Recording::try_from)
        .collect()
    }

    async fn path_known(&self, path: &str) -> RepoResult<bool> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM recording WHERE segment_path = $1"#,
            path
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(n > 0)
    }

    async fn mark_uploading(&self, id: RecordingId) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE recording SET status = 'uploading', error = '' WHERE id = $1",
            id.0
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn mark_done(&self, id: RecordingId, reduct_ts: i64) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE recording SET status = 'done', error = '', reduct_ts = $2 WHERE id = $1",
            id.0,
            reduct_ts
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn mark_error(&self, id: RecordingId, message: &str) -> RepoResult<()> {
        let r = sqlx::query!(
            "UPDATE recording SET status = 'error', error = $2 WHERE id = $1",
            id.0,
            message
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        done(r.rows_affected())
    }

    async fn delete(&self, id: RecordingId) -> RepoResult<()> {
        let r = sqlx::query!("DELETE FROM recording WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        done(r.rows_affected())
    }
}

// ------------------------------------------------------------------ snapshots

struct SnapshotRow {
    id: i64,
    camera_id: i64,
    taken_at: DateTime<Utc>,
    file: String,
    size_bytes: i32,
}

impl From<SnapshotRow> for Snapshot {
    fn from(r: SnapshotRow) -> Self {
        Self {
            id: SnapshotId(r.id),
            camera: CameraId(r.camera_id),
            taken_at: r.taken_at,
            file: r.file,
            size_bytes: r.size_bytes,
        }
    }
}

impl SnapshotRepo for PgSnapshots {
    async fn create(
        &self,
        camera: CameraId,
        taken_at: DateTime<Utc>,
        file: &str,
        size_bytes: i32,
    ) -> RepoResult<Snapshot> {
        let row = sqlx::query_as!(
            SnapshotRow,
            "INSERT INTO snapshot (camera_id, taken_at, file, size_bytes) VALUES ($1, $2, $3, $4)
             RETURNING id, camera_id, taken_at, file, size_bytes",
            camera.0,
            taken_at,
            file,
            size_bytes
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn get(&self, id: SnapshotId) -> RepoResult<Option<Snapshot>> {
        let row = sqlx::query_as!(
            SnapshotRow,
            "SELECT id, camera_id, taken_at, file, size_bytes FROM snapshot WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Snapshot::from))
    }

    async fn list(&self, camera: CameraId, limit: i64) -> RepoResult<Vec<Snapshot>> {
        let rows = sqlx::query_as!(
            SnapshotRow,
            "SELECT id, camera_id, taken_at, file, size_bytes FROM snapshot
             WHERE camera_id = $1 ORDER BY taken_at DESC, id DESC LIMIT $2",
            camera.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Snapshot::from).collect())
    }

    async fn list_filtered(
        &self,
        camera: Option<CameraId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> RepoResult<Vec<Snapshot>> {
        let rows = sqlx::query_as!(
            SnapshotRow,
            "SELECT id, camera_id, taken_at, file, size_bytes FROM snapshot
             WHERE ($1::BIGINT IS NULL OR camera_id = $1) AND ($2::TIMESTAMPTZ IS NULL OR taken_at >= $2)
               AND ($3::TIMESTAMPTZ IS NULL OR taken_at < $3)
             ORDER BY taken_at DESC, id DESC LIMIT $4",
            camera.map(|c| c.0),
            from,
            to,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Snapshot::from).collect())
    }

    async fn excess(&self, camera: CameraId, keep: i64) -> RepoResult<Vec<Snapshot>> {
        let rows = sqlx::query_as!(
            SnapshotRow,
            "SELECT id, camera_id, taken_at, file, size_bytes FROM snapshot
             WHERE camera_id = $1 ORDER BY taken_at DESC, id DESC OFFSET $2",
            camera.0,
            keep
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().rev().map(Snapshot::from).collect())
    }

    async fn delete(&self, id: SnapshotId) -> RepoResult<()> {
        let r = sqlx::query!("DELETE FROM snapshot WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        done(r.rows_affected())
    }
}

// ------------------------------------------------------------------ mouvements et detections

fn labels_json(labels: &[Detection]) -> Value {
    Value::Array(
        labels
            .iter()
            .map(|d| json!({"class": d.class, "confidence": d.confidence, "bbox": d.bbox}))
            .collect(),
    )
}

fn labels_from_json(v: &Value) -> RepoResult<Vec<Detection>> {
    let bad = || corrupt("detection labels");
    v.as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|e| {
            let bbox = e["bbox"].as_array().ok_or_else(bad)?;
            if bbox.len() != 4 {
                return Err(bad());
            }
            let mut b = [0.0; 4];
            for (slot, n) in b.iter_mut().zip(bbox) {
                *slot = n.as_f64().ok_or_else(bad)?;
            }
            Ok(Detection {
                class: e["class"].as_str().ok_or_else(bad)?.to_string(),
                confidence: e["confidence"].as_f64().ok_or_else(bad)?,
                bbox: b,
            })
        })
        .collect()
}

struct DetectionRow {
    id: i64,
    camera_id: i64,
    detected_at: DateTime<Utc>,
    triggered_by: String,
    labels: Value,
    frame_path: String,
}

impl TryFrom<DetectionRow> for DetectionEvent {
    type Error = RepoError;

    fn try_from(r: DetectionRow) -> Result<Self, RepoError> {
        Ok(Self {
            id: DetectionEventId(r.id),
            camera: CameraId(r.camera_id),
            detected_at: r.detected_at,
            triggered_by: parse("declencheur", &r.triggered_by)?,
            labels: labels_from_json(&r.labels)?,
            frame_path: r.frame_path,
        })
    }
}

impl EventRepo for PgCameraEvents {
    async fn record_motion(
        &self,
        camera: CameraId,
        at: DateTime<Utc>,
        confidence: f64,
    ) -> RepoResult<MotionEvent> {
        let r = sqlx::query!(
            "INSERT INTO motion_event (camera_id, detected_at, confidence) VALUES ($1, $2, $3) RETURNING id",
            camera.0,
            at,
            confidence
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(MotionEvent {
            id: MotionEventId(r.id),
            camera,
            detected_at: at,
            confidence,
        })
    }

    async fn list_motion(&self, camera: CameraId, limit: i64) -> RepoResult<Vec<MotionEvent>> {
        let rows = sqlx::query!(
            "SELECT id, detected_at, confidence FROM motion_event WHERE camera_id = $1
             ORDER BY detected_at DESC, id DESC LIMIT $2",
            camera.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| MotionEvent {
                id: MotionEventId(r.id),
                camera,
                detected_at: r.detected_at,
                confidence: r.confidence,
            })
            .collect())
    }

    async fn motion_between(
        &self,
        camera: CameraId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> RepoResult<bool> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM motion_event WHERE camera_id = $1 AND detected_at BETWEEN $2 AND $3"#,
            camera.0,
            from,
            to
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(n > 0)
    }

    async fn record_detection(
        &self,
        camera: CameraId,
        at: DateTime<Utc>,
        trigger: DetectionTrigger,
        labels: &[Detection],
        frame_path: &str,
    ) -> RepoResult<DetectionEvent> {
        let r = sqlx::query!(
            "INSERT INTO detection_event (camera_id, detected_at, triggered_by, labels, frame_path)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
            camera.0,
            at,
            trigger.as_str(),
            labels_json(labels),
            frame_path
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(DetectionEvent {
            id: DetectionEventId(r.id),
            camera,
            detected_at: at,
            triggered_by: trigger,
            labels: labels.to_vec(),
            frame_path: frame_path.to_string(),
        })
    }

    async fn get_detection(&self, id: DetectionEventId) -> RepoResult<Option<DetectionEvent>> {
        sqlx::query_as!(
            DetectionRow,
            "SELECT id, camera_id, detected_at, triggered_by, labels, frame_path FROM detection_event WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(DetectionEvent::try_from)
        .transpose()
    }

    async fn list_detections(
        &self,
        camera: CameraId,
        limit: i64,
    ) -> RepoResult<Vec<DetectionEvent>> {
        sqlx::query_as!(
            DetectionRow,
            "SELECT id, camera_id, detected_at, triggered_by, labels, frame_path FROM detection_event
             WHERE camera_id = $1 ORDER BY detected_at DESC, id DESC LIMIT $2",
            camera.0,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(DetectionEvent::try_from)
        .collect()
    }
}

// ------------------------------------------------------------------ detecteur

impl DetectorConfigRepo for PgDetectorConfig {
    async fn get(&self) -> RepoResult<DetectorConfig> {
        let r = sqlx::query!(
            "SELECT backend, model_path, device, enabled, save_frames, updated_at FROM detector_config WHERE id = 1"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(DetectorConfig {
            backend: parse("backend de detection", &r.backend)?,
            model_path: r.model_path,
            device: r.device,
            enabled: r.enabled,
            save_frames: r.save_frames,
            updated_at: r.updated_at,
        })
    }

    async fn update(&self, c: &DetectorConfig) -> RepoResult<DetectorConfig> {
        sqlx::query!(
            "UPDATE detector_config SET backend = $1, model_path = $2, device = $3, enabled = $4, save_frames = $5,
                 updated_at = now() WHERE id = 1",
            c.backend.as_str(),
            c.model_path,
            c.device,
            c.enabled,
            c.save_frames
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        self.get().await
    }
}
