// SPDX-License-Identifier: AGPL-3.0-or-later
//! Surveillance (R6, ADR 0043) : cameras, presets PTZ, enregistrements, snapshots, evenements de mouvement et de detection.
//!
//! Les mots de passe et la cle d'API d'une camera sont des [`Sealed`] : le domaine ne manipule que le texte **deja chiffre**
//! (le chiffrement est fait par le serveur, `SecretBox`), et `Debug` n'en montre rien.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{Problem, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(pub i64);

/// Identifiant public d'une camera : 32 caracteres hexadecimaux, aussi **nom du path MediaMTX** (ADR 0043).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CameraUuid(pub Uuid);

impl fmt::Display for CameraUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.simple())
    }
}

impl fmt::Debug for CameraUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CameraUuid({self})")
    }
}

impl FromStr for CameraUuid {
    type Err = ValidationError;

    /// Accepte la forme a 32 caracteres hexadecimaux (et la forme avec tirets, que le modele de widget peut porter).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s.trim())
            .map(Self)
            .map_err(|_| ValidationError::new("camera identifier: 32 hexadecimal characters"))
    }
}

/// Secret deja chiffre (`v1:...`), vide s'il n'est pas renseigne. Jamais affiche.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Sealed(pub String);

impl Sealed {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Sealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Sealed(empty)"
        } else {
            "Sealed(***)"
        })
    }
}

/// Enumeration textuelle stockee : `as_str`, `ALL`, `FromStr`.
macro_rules! text_enum {
    ($(#[$m:meta])* $name:ident, $what:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl FromStr for $name {
            type Err = ValidationError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.as_str() == s)
                    .ok_or_else(|| ValidationError(format!(concat!($what, " unknown: \"{}\""), s)))
            }
        }
    };
}

text_enum!(StreamProto, "protocol", { Rtsp => "rtsp", Rtmp => "rtmp" });
text_enum!(
    /// Quand une camera est enregistree (`manual` : seulement sur demande).
    RecordingMode, "recording mode",
    { None => "none", Continuous => "continuous", Motion => "motion", Manual => "manual" }
);
text_enum!(
    /// Methode de bascule jour / nuit (IR-cut) : un backend par valeur, `none` = desactive.
    NightApi, "day/night mode",
    { None => "none", Onvif => "onvif", Openipc => "openipc", Thingino => "thingino", Goform => "goform" }
);
text_enum!(RecordingEvent, "recording type", { Continuous => "continuous", Motion => "motion", Manual => "manual" });
text_enum!(RecordingStatus, "recording status", { Pending => "pending", Uploading => "uploading", Done => "done", Error => "error" });
text_enum!(DetectionTrigger, "declencheur", { Motion => "motion", Periodic => "periodic" });
text_enum!(DetectorBackend, "backend de detection", { Yolov8n => "yolov8n", Yolov8s => "yolov8s", Onnx => "onnx", Coral => "coral" });

/// Reglages d'une camera (creation ou remplacement). Les secrets sont chiffres.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent form flags (active, ONVIF, PTZ, YOLO, RTSP options)"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct CameraDraft {
    pub name: String,
    pub location_id: Option<i64>,
    pub active: bool,
    /// 1 = haute priorite, 9 = basse.
    pub priority: i32,
    pub notes: String,
    pub proto: StreamProto,
    pub ip: String,
    pub port: i32,
    pub path: String,
    pub rtsp_user: String,
    pub rtsp_pass: Sealed,
    pub fps: i32,
    pub rtsp_any_port: bool,
    pub rtsp_transport_tcp: bool,
    pub recording_mode: RecordingMode,
    pub night_mode_api: NightApi,
    pub web_port: i32,
    pub web_api_key: Sealed,
    pub web_user: String,
    pub web_pass: Sealed,
    pub onvif: bool,
    pub onvif_port: i32,
    pub onvif_user: String,
    pub onvif_pass: Sealed,
    pub main_token: String,
    pub ptz: bool,
    pub yolo_enabled: bool,
    /// Classes COCO retenues ; vide = toutes.
    pub yolo_classes: Vec<String>,
    pub yolo_min_confidence: f64,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
}

impl CameraDraft {
    pub const NAME_MAX: usize = 128;

    /// Camera aux valeurs par defaut de la reference (RTSP, port 554, 10 fps, rien d'enregistre).
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            location_id: None,
            active: true,
            priority: 1,
            notes: String::new(),
            proto: StreamProto::Rtsp,
            ip: String::new(),
            port: 554,
            path: String::new(),
            rtsp_user: String::new(),
            rtsp_pass: Sealed::default(),
            fps: 10,
            rtsp_any_port: false,
            rtsp_transport_tcp: false,
            recording_mode: RecordingMode::None,
            night_mode_api: NightApi::None,
            web_port: 80,
            web_api_key: Sealed::default(),
            web_user: String::new(),
            web_pass: Sealed::default(),
            onvif: false,
            onvif_port: 80,
            onvif_user: String::new(),
            onvif_pass: Sealed::default(),
            main_token: String::new(),
            ptz: false,
            yolo_enabled: false,
            yolo_classes: vec![],
            yolo_min_confidence: 0.5,
            manufacturer: String::new(),
            model: String::new(),
            firmware: String::new(),
        }
    }

    /// Tous les problemes, champ par champ.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > Self::NAME_MAX {
            out.push((
                "name",
                Problem::TooLong {
                    max: Self::NAME_MAX,
                },
            ));
        }
        if self.ip.trim().is_empty() {
            out.push(("ip", Problem::Required));
        } else if !valid_host(&self.ip) {
            out.push(("ip", Problem::Invalid));
        }
        for (field, value, min, max) in [
            ("priority", self.priority, 1, 9),
            ("port", self.port, 1, 65535),
            ("fps", self.fps, 1, 60),
            ("web_port", self.web_port, 1, 65535),
            ("onvif_port", self.onvif_port, 1, 65535),
        ] {
            if !(min..=max).contains(&value) {
                out.push((
                    field,
                    Problem::OutOfRange {
                        min: i64::from(min),
                        max: i64::from(max),
                    },
                ));
            }
        }
        // Le chemin vient d'un champ libre : pas d'espace, de retour a la ligne ni de `@` (injection dans l'URL du flux).
        if self.path.chars().count() > 128
            || self.path.chars().any(|c| c.is_whitespace() || c == '@')
        {
            out.push(("path", Problem::Invalid));
        }
        for (field, value) in [
            ("rtsp_user", &self.rtsp_user),
            ("web_user", &self.web_user),
            ("onvif_user", &self.onvif_user),
        ] {
            if value.chars().count() > 128 {
                out.push((field, Problem::TooLong { max: 128 }));
            }
        }
        if !(0.0..=1.0).contains(&self.yolo_min_confidence) {
            out.push(("yolo_min_confidence", Problem::Invalid));
        }
        out
    }

    /// # Errors
    /// Le premier probleme, en francais.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self.problems().first() {
            None => Ok(()),
            Some((field, problem)) => Err(ValidationError(format!("{field} : {problem:?}"))),
        }
    }

    /// URL du flux source **avec** le mot de passe en clair (`password` est le secret dechiffre) : a ne jamais journaliser.
    #[must_use]
    pub fn source_url(&self, password: &str) -> String {
        self.build_url(&if self.rtsp_user.is_empty() {
            String::new()
        } else {
            format!("{}:{}@", userinfo(&self.rtsp_user), userinfo(password))
        })
    }

    /// Meme URL, mot de passe masque : celle-ci peut s'afficher et se journaliser.
    #[must_use]
    pub fn masked_source_url(&self) -> String {
        self.build_url(&if self.rtsp_user.is_empty() {
            String::new()
        } else {
            format!("{}:***@", userinfo(&self.rtsp_user))
        })
    }

    fn build_url(&self, auth: &str) -> String {
        let path = if self.path.is_empty() {
            String::new()
        } else {
            format!("/{}", self.path.trim_start_matches('/'))
        };
        format!(
            "{}://{auth}{}:{}{path}",
            self.proto.as_str(),
            self.ip,
            self.port
        )
    }
}

/// Nom d'hote, adresse IPv4 ou IPv6 (sans crochets) : lettres, chiffres, `.`, `-`, `:`.
fn valid_host(host: &str) -> bool {
    host.len() <= 64
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

/// Echappe `:`, `@`, `/`, `%` et les caracteres non ASCII d'un identifiant ou d'un mot de passe (partie `userinfo` d'une URL).
fn userinfo(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub id: CameraId,
    pub uuid: CameraUuid,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub config: CameraDraft,
}

/// Ce que la synchronisation ONVIF apprend d'une camera.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OnvifInfo {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub main_token: String,
    pub ptz: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PtzPresetId(pub i64);

#[derive(Debug, Clone, PartialEq)]
pub struct PtzPresetDraft {
    pub name: String,
    pub token: String,
    pub pan: f64,
    pub tilt: f64,
    pub zoom: f64,
    pub position: i16,
}

impl PtzPresetDraft {
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 128 {
            out.push(("name", Problem::TooLong { max: 128 }));
        }
        if self.token.chars().count() > 64 {
            out.push(("token", Problem::TooLong { max: 64 }));
        }
        if self.position < 0 {
            out.push(("position", Problem::Invalid));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PtzPreset {
    pub id: PtzPresetId,
    pub camera: CameraId,
    pub draft: PtzPresetDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordingId(pub i64);

#[derive(Debug, Clone, PartialEq)]
pub struct NewRecording {
    pub started_at: DateTime<Utc>,
    pub segment_path: String,
    pub event_type: RecordingEvent,
    pub size_bytes: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub id: RecordingId,
    pub camera: CameraId,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub segment_path: String,
    /// Horodatage de l'entree dans ReductStore, en microsecondes.
    pub reduct_ts: Option<i64>,
    pub event_type: RecordingEvent,
    pub status: RecordingStatus,
    pub error: String,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub camera: CameraId,
    pub taken_at: DateTime<Utc>,
    /// Chemin relatif au dossier des snapshots.
    pub file: String,
    pub size_bytes: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MotionEventId(pub i64);

#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvent {
    pub id: MotionEventId,
    pub camera: CameraId,
    pub detected_at: DateTime<Utc>,
    /// Rapport de pixels modifies (0..1).
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DetectionEventId(pub i64);

/// Un objet reconnu : classe COCO, score, boite `[x1, y1, x2, y2]` en pixels de l'image d'origine.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub class: String,
    pub confidence: f64,
    pub bbox: [f64; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectionEvent {
    pub id: DetectionEventId,
    pub camera: CameraId,
    pub detected_at: DateTime<Utc>,
    pub triggered_by: DetectionTrigger,
    pub labels: Vec<Detection>,
    pub frame_path: String,
}

impl DetectionEvent {
    /// Classes distinctes, triees.
    #[must_use]
    pub fn class_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.labels.iter().map(|d| d.class.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Configuration globale du detecteur (une seule ligne).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorConfig {
    pub backend: DetectorBackend,
    pub model_path: String,
    pub device: String,
    pub enabled: bool,
    pub save_frames: bool,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> CameraDraft {
        let mut d = CameraDraft::new("Entree");
        d.ip = "192.168.1.29".into();
        d
    }

    #[test]
    fn uuid_is_32_hex_and_parses_both_forms() {
        let u = CameraUuid(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        let text = u.to_string();
        assert_eq!(text.len(), 32);
        assert!(text.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(text.parse::<CameraUuid>().unwrap(), u);
        assert_eq!(
            u.0.hyphenated().to_string().parse::<CameraUuid>().unwrap(),
            u
        );
        assert!("pas-un-uuid".parse::<CameraUuid>().is_err());
    }

    #[test]
    fn enums_round_trip_and_refuse_unknown_values() {
        for m in RecordingMode::ALL {
            assert_eq!(m.as_str().parse::<RecordingMode>().unwrap(), *m);
        }
        for n in NightApi::ALL {
            assert_eq!(n.as_str().parse::<NightApi>().unwrap(), *n);
        }
        assert_eq!(NightApi::ALL.len(), 5);
        assert!("manuel".parse::<RecordingMode>().is_err());
        assert!("rtsp".parse::<StreamProto>().is_ok());
    }

    #[test]
    fn debug_never_shows_a_secret() {
        let mut d = cam();
        d.rtsp_pass = Sealed("v1:abcdef".into());
        let shown = format!("{d:?}");
        assert!(!shown.contains("abcdef"), "{shown}");
        assert!(shown.contains("Sealed(***)"));
        assert!(format!("{:?}", Sealed::default()).contains("empty"));
    }

    #[test]
    fn problems_are_reported_per_field() {
        assert_eq!(cam().problems(), []);
        let mut bad = cam();
        bad.name = " ".into();
        bad.ip = String::new();
        bad.port = 70000;
        bad.fps = 0;
        bad.priority = 10;
        bad.path = "a b".into();
        bad.yolo_min_confidence = 1.5;
        let fields: Vec<_> = bad.problems().into_iter().map(|(f, _)| f).collect();
        assert_eq!(
            fields,
            [
                "name",
                "ip",
                "priority",
                "port",
                "fps",
                "path",
                "yolo_min_confidence"
            ]
        );
        assert!(bad.validate().is_err());
        let mut host = cam();
        host.ip = "cam.local; rm -rf".into();
        assert_eq!(host.problems(), [("ip", Problem::Invalid)]);
    }

    #[test]
    fn source_url_escapes_credentials_and_can_be_masked() {
        let mut d = cam();
        d.rtsp_user = "admin".into();
        d.path = "/stream1".into();
        assert_eq!(
            d.source_url("p@ss:w/rd"),
            "rtsp://admin:p%40ss%3Aw%2Frd@192.168.1.29:554/stream1"
        );
        assert_eq!(
            d.masked_source_url(),
            "rtsp://admin:***@192.168.1.29:554/stream1"
        );
        d.rtsp_user.clear();
        assert_eq!(d.source_url("x"), "rtsp://192.168.1.29:554/stream1");
    }

    #[test]
    fn detection_class_names_are_distinct_and_sorted() {
        let det = |c: &str| Detection {
            class: c.into(),
            confidence: 0.9,
            bbox: [0.0; 4],
        };
        let ev = DetectionEvent {
            id: DetectionEventId(1),
            camera: CameraId(1),
            detected_at: Utc::now(),
            triggered_by: DetectionTrigger::Motion,
            labels: vec![det("person"), det("car"), det("person")],
            frame_path: String::new(),
        };
        assert_eq!(ev.class_names(), ["car", "person"]);
    }
}
