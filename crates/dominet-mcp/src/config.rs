// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading and writing configuration through MCP (R11 M4): a few **kinds** (places, scenes, automations, alert rules), one set of
//! generic tools over them.
//!
//! Three rules shape this module:
//! - **One code path with the interface.** A JSON payload becomes the same domain draft the interface builds (same validation),
//!   and saving goes through `dominet_server::config_service`, which both share: the script that must compile, the scene that
//!   must exist, the engine that reloads. Nothing is re-implemented here.
//! - **What is read can be written back.** [`get`] returns the item in the exact shape [`plan_upsert`] accepts, so an assistant
//!   reads, changes a field and writes. The one thing it cannot read is the code of a Rhai script (`administration` scope):
//!   an update that does not mention `script` **keeps** the existing one, it never erases it.
//! - **A plan before a change.** [`plan_upsert`] and [`plan_delete`] say what would happen (fields that change, what a deletion
//!   takes with it) without touching anything; the tools apply the plan only when not asked for a dry run.

use std::collections::HashMap;

use dominet_domain::alert::{AlertRule, AlertRuleDraft, AlertRuleId, Operator, Severity};
use dominet_domain::automation::{Action, Automation, AutomationDraft, AutomationId};
use dominet_domain::camera::{CameraDraft, CameraId, NightApi, RecordingMode, StreamProto};
use dominet_domain::command::check_command;
use dominet_domain::dashboard::{
    DashboardDraft, DashboardId, GridRect, Presentation, WidgetDraft, WidgetId, WidgetType,
};
use dominet_domain::device::{
    Device, DeviceDraft, DeviceGroupDraft, DeviceGroupId, DeviceId, DeviceLabelDraft,
    DeviceLabelId, DeviceType, MqttSource, MqttSourceId, WeatherStation, WeatherStationId,
};
use dominet_domain::location::{Location, LocationDraft, LocationId};
use dominet_domain::navbar::{NavPosition, NavbarDraft, NavbarItemId};
use dominet_domain::observability::{
    EXEC_ERR_STATES, GrafanaRule, GrafanaRuleDraft, GrafanaRuleId, NO_DATA_STATES, PrometheusRule,
    PrometheusRuleDraft, PrometheusRuleId,
};
use dominet_domain::repo::{
    AlertRuleRepo, AutomationRepo, AutomationRunRepo, CameraRepo, DashboardRepo,
    DetectorConfigRepo, DeviceFilter, DeviceGroupRepo, DeviceLabelRepo, DeviceRepo,
    GrafanaRuleRepo, LocationRepo, MqttSourceRepo, NavbarRepo, PrometheusRuleRepo, SceneRepo,
    UserRepo, WeatherRepo, WidgetRepo,
};
use dominet_domain::scene::{Scene, SceneActionDraft, SceneDraft, SceneId};
use dominet_proto::{DeleteOutcome, FieldProblem, ProblemKind, SaveOutcome};
use dominet_server::cameras::{CameraError, CameraInput as CameraSave, SecretInput};
use dominet_server::{AppState, config_service};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

/// What can be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Location,
    Scene,
    Automation,
    AlertRule,
    DeviceGroup,
    DeviceLabel,
    MqttSource,
    WeatherStation,
    PrometheusRule,
    GrafanaRule,
    Dashboard,
    Widget,
    Device,
    Navbar,
    Detector,
    Camera,
}

impl Kind {
    pub const ALL: [Kind; 16] = [
        Kind::Location,
        Kind::Scene,
        Kind::Automation,
        Kind::AlertRule,
        Kind::DeviceGroup,
        Kind::DeviceLabel,
        Kind::MqttSource,
        Kind::WeatherStation,
        Kind::PrometheusRule,
        Kind::GrafanaRule,
        Kind::Dashboard,
        Kind::Widget,
        Kind::Device,
        Kind::Navbar,
        Kind::Detector,
        Kind::Camera,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Location => "location",
            Kind::Scene => "scene",
            Kind::Automation => "automation",
            Kind::AlertRule => "alert_rule",
            Kind::DeviceGroup => "device_group",
            Kind::DeviceLabel => "device_label",
            Kind::MqttSource => "mqtt_source",
            Kind::WeatherStation => "weather_station",
            Kind::PrometheusRule => "prometheus_rule",
            Kind::GrafanaRule => "grafana_rule",
            Kind::Dashboard => "dashboard",
            Kind::Widget => "widget",
            Kind::Device => "device",
            Kind::Navbar => "navbar",
            Kind::Detector => "detector",
            Kind::Camera => "camera",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Kind> {
        let text = text.trim();
        Kind::ALL
            .into_iter()
            .find(|k| k.name() == text)
            .or_else(|| Kind::from_french(text))
    }

    /// The French word a model (or a person) may use for a kind: `emplacement`, `scene`, `regle d'alerte`... Accents, case, spaces
    /// and hyphens do not matter. The identifiers of `config_kinds` stay the reference; this only forgives the usual wording.
    fn from_french(text: &str) -> Option<Kind> {
        let norm: String = text
            .to_lowercase()
            .chars()
            .map(|c| match c {
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'à' | 'â' => 'a',
                'î' | 'ï' => 'i',
                'ô' => 'o',
                'ù' | 'û' => 'u',
                ' ' | '-' | '\'' | '’' => '_',
                other => other,
            })
            .collect();
        if let Some(k) = Kind::ALL.into_iter().find(|k| k.name() == norm) {
            return Some(k);
        }
        Some(match norm.as_str() {
            "emplacement" | "emplacements" | "piece" | "lieu" => Kind::Location,
            "scenes" => Kind::Scene,
            "automations" => Kind::Automation,
            "regle_d_alerte" | "regle_alerte" | "alerte" | "regle_dalerte" => Kind::AlertRule,
            "groupe" | "groupe_d_appareils" | "groupe_dappareils" => Kind::DeviceGroup,
            "libelle" | "etiquette" | "libelle_d_appareil" => Kind::DeviceLabel,
            "source_mqtt" => Kind::MqttSource,
            "station_meteo" | "meteo" => Kind::WeatherStation,
            "regle_prometheus" => Kind::PrometheusRule,
            "regle_grafana" => Kind::GrafanaRule,
            "tableau_de_bord" | "tableau" => Kind::Dashboard,
            "widgets" => Kind::Widget,
            "appareil" | "appareils" => Kind::Device,
            "barre_de_navigation" | "navigation" => Kind::Navbar,
            "detecteur" | "detection" => Kind::Detector,
            "camera" | "cameras" => Kind::Camera,
            _ => return None,
        })
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Kind::Location => "Place (room, floor...) to which scenes and devices are attached.",
            Kind::Scene => "Scene: a list of commands sent together to devices.",
            Kind::Automation => {
                "Automation: a trigger (device, MQTT, schedule, sun), an optional condition, an action."
            }
            Kind::AlertRule => {
                "Alert rule: a threshold on a device key, with severity and notification channels."
            }
            Kind::DeviceGroup => "Device group (attached to a place, optional).",
            Kind::DeviceLabel => {
                "Label of a state key (readable name, icon, unit) used to display values."
            }
            Kind::MqttSource => {
                "MQTT source: a listened topic pattern, with automatic device creation and optional Rhai handler code."
            }
            Kind::WeatherStation => {
                "Weather station: a place (latitude, longitude) whose forecasts are published on meteo/<suffix>."
            }
            Kind::PrometheusRule => {
                "Prometheus alert rule (PromQL); saving rewrites rules.yml and reloads Prometheus."
            }
            Kind::GrafanaRule => {
                "Grafana alert rule; saving rewrites rules.yaml and reloads Grafana."
            }
            Kind::Dashboard => {
                "Dashboard of an account (the owner no longer changes after creation); named owner/name."
            }
            Kind::Widget => {
                "Dashboard widget: device, scene, camera, Grafana panel or text, placed on the grid (12 columns)."
            }
            Kind::Device => {
                "Device: its settings (name, type, group, place, tracked or hidden keys). Its state and what Zigbee2MQTT discovers cannot be changed here."
            }
            Kind::Navbar => {
                "Navigation bar entry: Lucide icon, label, address, position (top or bottom), order."
            }
            Kind::Camera => {
                "Camera: address, stream, ONVIF, detection. Reserved to the `administration` scope (address and credentials); passwords can be written, never read."
            }
            Kind::Detector => {
                "Camera object detector: a single setting (enabled or not, saving of annotated images). It can neither be created nor deleted."
            }
        }
    }
}

/// Why a configuration call failed, in words an assistant can act on.
#[derive(Debug)]
pub enum ConfigError {
    /// The payload is refused: one line per problem, `field : reason`.
    Invalid(Vec<String>),
    NotFound(String),
    /// Storage failed; the detail is in the log, not here.
    Storage(String),
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(vec![message.into()])
}

fn storage(context: &str, err: &dyn std::fmt::Display) -> ConfigError {
    tracing::error!(%err, "{context}");
    ConfigError::Storage(context.to_string())
}

/// A field problem as a sentence: `name : obligatoire`.
#[must_use]
pub fn problem_text(p: &FieldProblem) -> String {
    let why = match &p.kind {
        ProblemKind::Required => "mandatory".to_string(),
        ProblemKind::TooLong { max } => format!("too long ({max} characters at most)"),
        ProblemKind::Invalid => "invalid value".to_string(),
        ProblemKind::OutOfRange { min, max } => format!("out of range ({min} to {max})"),
        ProblemKind::Taken => "already taken".to_string(),
        ProblemKind::Overlap => "overlaps another element".to_string(),
        ProblemKind::Syntax(detail) => format!("syntax error: {detail}"),
        ProblemKind::Password(_) => "refused".to_string(),
    };
    format!("{} : {why}", p.field)
}

fn problems(list: &[FieldProblem]) -> ConfigError {
    ConfigError::Invalid(list.iter().map(problem_text).collect())
}

fn default_true() -> bool {
    true
}

// ------------------------------------------------------------------ what an assistant writes

/// A place.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationInput {
    /// Name of the place (`Salon`, `Etage`...). Unique.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Switched on (default `true`).
    #[serde(default = "default_true")]
    pub active: bool,
}

/// One command of a scene.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneActionInput {
    /// The device: its name (`Volet-salon`) or its numeric id.
    pub device: String,
    /// What to send, a JSON object: `{"state": "CLOSE"}`.
    pub command: Value,
}

/// A scene.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneInput {
    /// Name of the scene. Unique.
    pub name: String,
    /// Icon name (optional).
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    /// The place it belongs to: a name or a numeric id (optional).
    #[serde(default)]
    pub location: Option<String>,
    /// Switched on (default `true`); an inactive scene cannot be run.
    #[serde(default = "default_true")]
    pub active: bool,
    /// The commands, sent in this order. At most 100.
    #[serde(default)]
    pub actions: Vec<SceneActionInput>,
}

/// A trigger or an action: its type and the keys that type needs (see `config_schema`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypedConfig {
    /// Trigger: `device_state`, `mqtt_message`, `schedule` or `solar`. Action: `mqtt_publish`, `scene`, `notification` or `script`.
    pub r#type: String,
    /// The keys of that type, an object.
    #[serde(default)]
    pub config: Value,
}

/// An automation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomationInput {
    pub name: String,
    /// Switched on (default `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
    pub trigger: TypedConfig,
    /// Optional: `{"key": "action", "value": "1_single"}`; the automation only runs if the key is absent or has that value.
    #[serde(default)]
    pub condition: Option<Value>,
    pub action: TypedConfig,
    /// Rhai code, for the `script` action. Writing, changing or removing code needs the `administration` scope; leave it out
    /// to keep the existing code of an automation.
    #[serde(default)]
    pub script: Option<String>,
}

/// An alert rule.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRuleInput {
    pub name: String,
    /// The device watched: its name or its numeric id.
    pub device: String,
    /// The key of its state (`temperature`, `humidity`...).
    pub key: String,
    /// `gt`, `gte`, `lt`, `lte`, `eq`, `neq` or `contains`.
    pub operator: String,
    /// The number to compare with (for a numeric key).
    #[serde(default)]
    pub threshold_num: Option<f64>,
    /// The text to compare with (for a text key).
    #[serde(default)]
    pub threshold_str: String,
    /// `info`, `warning` or `critical` (default `warning`).
    #[serde(default)]
    pub severity: Option<String>,
    /// Minutes between two alerts of this rule (default 10, 0 to 1440).
    #[serde(default)]
    pub cooldown_minutes: Option<i16>,
    /// Notify by e-mail (default `true`).
    #[serde(default)]
    pub notify_email: Option<bool>,
    /// Notify by SMS (default `false`).
    #[serde(default)]
    pub notify_sms: Option<bool>,
    /// Notify the Android phones (default `true`).
    #[serde(default)]
    pub notify_android: Option<bool>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
}

/// A group of devices.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceGroupInput {
    /// Name of the group. Unique.
    pub name: String,
    /// The place it belongs to: a name or a numeric id (optional).
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
}

/// The label of a state key.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceLabelInput {
    /// The state key (`temperature`). Unique.
    pub key: String,
    /// Readable name (`Temperature`).
    pub name: String,
    /// Icon name.
    pub icon_name: String,
    /// Unit (`°C`), optional.
    #[serde(default)]
    pub unit: String,
}

/// An MQTT source.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MqttSourceInput {
    pub name: String,
    /// MQTT topic filter (`maison/+/etat`).
    pub topic_pattern: String,
    /// 0, 1 or 2 (default 0).
    #[serde(default)]
    pub qos: u8,
    #[serde(default = "default_true")]
    pub active: bool,
    /// Keep the received messages in the logs.
    #[serde(default)]
    pub log_messages: bool,
    /// Create a device for every new topic.
    #[serde(default)]
    pub auto_device: bool,
    /// Type of the created devices (default `sensor`).
    #[serde(default)]
    pub auto_device_type: Option<String>,
    #[serde(default)]
    pub base_topic: String,
    #[serde(default)]
    pub description: String,
    /// Rhai code run on each message. Writing, changing or removing it needs the `administration` scope; leave it out to keep
    /// the existing code.
    #[serde(default)]
    pub handler_code: Option<String>,
}

/// A Prometheus alert rule.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrometheusRuleInput {
    /// Name of the alert. Unique.
    pub name: String,
    /// Group in `rules.yml`.
    pub group: String,
    /// PromQL expression.
    pub expr: String,
    pub summary: String,
    /// `info`, `warning` or `critical` (default `warning`).
    #[serde(default)]
    pub severity: Option<String>,
    /// How long the condition must hold (`5m`; default `5m`).
    #[serde(default)]
    pub for_duration: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Evaluation interval of the group (`1m`); empty = Prometheus' own.
    #[serde(default)]
    pub eval_interval: String,
    #[serde(default = "default_true")]
    pub active: bool,
}

/// A Grafana alert rule.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrafanaRuleInput {
    pub title: String,
    /// Stable identifier (lowercase letters, digits, `-`, `_`); derived from the title when absent.
    #[serde(default)]
    pub uid: String,
    pub folder: String,
    pub group: String,
    /// What is evaluated: PromQL for the `prometheus` datasource, **raw SQL** for any other. Writing a SQL query needs the
    /// `administration` scope.
    pub expr: String,
    pub summary: String,
    /// Datasource uid (default `prometheus`).
    #[serde(default)]
    pub datasource_uid: Option<String>,
    /// Evaluation interval of the group (default `1m`).
    #[serde(default)]
    pub group_interval: Option<String>,
    /// Seconds of data queried, 1 to 86400 (default 300).
    #[serde(default)]
    pub query_window: Option<i32>,
    #[serde(default)]
    pub for_duration: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub description: String,
    /// `NoData`, `Alerting` or `OK` (default `NoData`).
    #[serde(default)]
    pub no_data_state: Option<String>,
    /// `Error`, `Alerting` or `OK` (default `Error`).
    #[serde(default)]
    pub exec_err_state: Option<String>,
    #[serde(default = "default_true")]
    pub active: bool,
}

/// A dashboard.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DashboardInput {
    pub name: String,
    /// Username of the owner. Required to create; cannot be changed afterwards.
    #[serde(default)]
    pub owner: Option<String>,
    /// Make it the default dashboard of its owner (it can only be switched on: another one taking the place switches it off).
    #[serde(default)]
    pub is_default: bool,
}

/// A device's parameters. Its state, availability and what Zigbee2MQTT discovers are not written here.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceInput {
    /// Name used in the MQTT topic. Unique.
    pub friendly_name: String,
    #[serde(default)]
    pub description: String,
    /// `sensor`, `binary_sensor`, `switch`, `light`, `cover`, `climate`, `remote` or `generic`.
    pub device_type: String,
    /// Its group: a name or a numeric id (optional).
    #[serde(default)]
    pub group: Option<String>,
    /// Its place: a name or a numeric id (optional).
    #[serde(default)]
    pub location: Option<String>,
    /// MQTT prefix (`zigbee2mqtt`); empty = the bare name is the topic.
    #[serde(default)]
    pub base_topic: String,
    /// An inactive device ignores the messages it receives.
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default)]
    pub icon: String,
    /// Picture (http or https), optional.
    #[serde(default)]
    pub image_url: String,
    /// State keys kept in the history (empty = the usual ones).
    #[serde(default)]
    pub tracked_keys: Vec<String>,
    /// State keys that are not shown.
    #[serde(default)]
    pub hidden_keys: Vec<String>,
    /// Generate a Grafana dashboard for this device.
    #[serde(default)]
    pub grafana_dashboard: bool,
}

/// An entry of the navigation bar.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavbarInput {
    /// A Lucide icon name (`layout-grid`); one that does not exist is refused.
    pub icon_name: String,
    pub label: String,
    /// An internal path (`/devices`) or an http(s) address.
    pub url: String,
    /// `_self` (default) or `_blank`.
    #[serde(default = "default_self")]
    pub url_target: String,
    /// Shown to administrators only.
    #[serde(default)]
    pub admin_only: bool,
    /// `top` (default) or `bottom`.
    #[serde(default = "default_top")]
    pub position: String,
    /// Order inside its position, from 0.
    #[serde(default)]
    pub sort_order: i16,
}

/// A camera. The passwords and the API key are **write-only**: reading never shows them (`has_*` fields say whether one is
/// set), omitting one or sending it empty keeps the one in place.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct CameraConfigInput {
    pub name: String,
    /// Its place: a name or a numeric id (optional).
    #[serde(default)]
    pub location: Option<String>,
    /// Default true.
    pub active: Option<bool>,
    /// 1 (high) to 9 (low); default 1.
    pub priority: Option<i32>,
    #[serde(default)]
    pub notes: String,
    /// `rtsp` (default) or `rtmp`.
    pub proto: Option<String>,
    /// Address of the camera.
    #[serde(default)]
    pub ip: String,
    /// Stream port; default 554.
    pub port: Option<i32>,
    /// Path of the stream on the camera.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub rtsp_user: String,
    /// Stream password. Write-only.
    #[serde(default)]
    pub rtsp_pass: Option<String>,
    /// Frames per second; default 10.
    pub fps: Option<i32>,
    #[serde(default)]
    pub rtsp_any_port: bool,
    #[serde(default)]
    pub rtsp_transport_tcp: bool,
    /// `none` (default), `continuous`, `motion` or `manual`.
    pub recording_mode: Option<String>,
    /// Day / night method: `none` (default), `onvif`, `openipc`, `thingino` or `goform`.
    pub night_mode_api: Option<String>,
    /// Web interface port of the camera; default 80.
    pub web_port: Option<i32>,
    #[serde(default)]
    pub web_user: String,
    /// Web interface password. Write-only.
    #[serde(default)]
    pub web_pass: Option<String>,
    /// API key of the camera (night mode). Write-only.
    #[serde(default)]
    pub web_api_key: Option<String>,
    #[serde(default)]
    pub onvif: bool,
    /// ONVIF port; default 80.
    pub onvif_port: Option<i32>,
    /// Empty: the stream user is used.
    #[serde(default)]
    pub onvif_user: String,
    /// ONVIF password. Write-only.
    #[serde(default)]
    pub onvif_pass: Option<String>,
    /// Token of the main ONVIF profile; empty keeps the one learned by the ONVIF synchronisation.
    #[serde(default)]
    pub main_token: Option<String>,
    /// The camera is steered in PTZ.
    #[serde(default)]
    pub ptz: bool,
    #[serde(default)]
    pub yolo_enabled: bool,
    /// COCO classes kept; empty = all.
    #[serde(default)]
    pub yolo_classes: Vec<String>,
    /// Minimum confidence, 0 to 100; default 50.
    pub yolo_min_confidence_pct: Option<i32>,
}

/// The object detector setting: one item, `detector`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetectorInput {
    /// Run the detector on camera frames.
    pub enabled: bool,
    /// Keep the annotated frames.
    #[serde(default)]
    pub save_frames: bool,
}

fn default_self() -> String {
    "_self".to_string()
}

fn default_top() -> String {
    "top".to_string()
}

/// A widget, placed on a dashboard's grid (12 columns, rows 0 to 99).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WidgetInput {
    /// The dashboard: `owner/name` or its numeric id. A widget never changes dashboard.
    pub dashboard: String,
    /// `device`, `scene`, `camera`, `grafana`, `text` or `grille`.
    pub r#type: String,
    #[serde(default)]
    pub title: String,
    /// How a device is drawn (default `auto`; see the schema notes).
    #[serde(default)]
    pub presentation: Option<String>,
    /// The device (`device` widget, or `grafana` with a `key`): its name or numeric id.
    #[serde(default)]
    pub device: Option<String>,
    /// The scene (`scene` widget): its name or numeric id.
    #[serde(default)]
    pub scene: Option<String>,
    /// The camera (`camera` widget): its UUID.
    #[serde(default)]
    pub camera: Option<String>,
    /// Extra settings, an object: Grafana `panel_url`, or `dashboard_uid` + `panel_id`, or `key` with a device.
    #[serde(default)]
    pub config: Value,
    /// HTML of a `text` widget; it is sanitised when saved.
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub col: Option<i16>,
    #[serde(default)]
    pub row: Option<i16>,
    #[serde(default)]
    pub width: Option<i16>,
    #[serde(default)]
    pub height: Option<i16>,
}

/// A weather station.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeatherStationInput {
    pub name: String,
    /// -90 to 90.
    pub latitude: f64,
    /// -180 to 180.
    pub longitude: f64,
    /// Metres, -500 to 9000 (default 0).
    #[serde(default)]
    pub altitude: i32,
    /// IANA name (default `Europe/Paris`).
    #[serde(default)]
    pub timezone: Option<String>,
    /// Published on `meteo/<suffix>` (default `meteo_locale`).
    #[serde(default)]
    pub mqtt_topic_suffix: Option<String>,
    /// Minutes between two refreshes (default 10).
    #[serde(default)]
    pub refresh_minutes: Option<i16>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
}

fn parse_input<T: serde::de::DeserializeOwned>(mut data: Value) -> Result<T, ConfigError> {
    // `has_*` fields are what a read says about what cannot be read (`has_script`, `has_rtsp_pass`): not input, so a read item
    // can be written back as it is.
    if let Value::Object(map) = &mut data {
        map.retain(|key, _| !key.starts_with("has_"));
    }
    serde_json::from_value(data).map_err(|e| invalid(format!("data: {e}")))
}

// ------------------------------------------------------------------ schemas

/// The JSON Schema of what `config_upsert` accepts for a kind, with an example and notes.
#[must_use]
pub fn schema(kind: Kind) -> Value {
    let (schema, example, notes) = match kind {
        Kind::Location => (
            schemars::schema_for!(LocationInput),
            json!({"name": "Salon", "description": "Rez-de-chaussee", "active": true}),
            vec!["The name is unique."],
        ),
        Kind::Scene => (
            schemars::schema_for!(SceneInput),
            json!({"name": "Fermeture du soir", "actions": [
                {"device": "Volet-salon", "command": {"state": "CLOSE"}},
                {"device": "Lampe-salon", "command": {"state": "OFF"}}
            ]}),
            vec![
                "Commands go out in order; a failing command does not stop the following ones.",
                "A device can be given by its name or its number.",
            ],
        ),
        Kind::Automation => (
            schemars::schema_for!(AutomationInput),
            json!({
                "name": "Volets au coucher du soleil",
                "trigger": {"type": "solar", "config": {"event": "sunset", "latitude": 43.3, "longitude": 5.4}},
                "action": {"type": "mqtt_publish", "config": {"topic": "zigbee2mqtt/Volet-salon/set", "payload": {"state": "CLOSE"}}}
            }),
            vec![
                "Triggers: device_state {friendly_name, key, value}; mqtt_message {topic, payload_filter?}; schedule {cron}; solar {event, latitude, longitude} with event among sunrise, sunset, solar_noon, dawn_civil, dusk_civil, dawn_nautical, dusk_nautical, dawn_astronomical, dusk_astronomical.",
                "Actions: mqtt_publish {topic, payload}; scene {scene_id}; notification {message, sms?}; script (the code goes in `script`).",
                "Writing, changing or removing Rhai code requires the `administration` scope. An update that does not mention `script` keeps the existing code.",
                "A save with `id` REPLACES the automation: give all its fields (config_get returns them in this form).",
            ],
        ),
        Kind::AlertRule => (
            schemars::schema_for!(AlertRuleInput),
            json!({"name": "Canicule", "device": "Bureau", "key": "temperature", "operator": "gt",
                   "threshold_num": 30, "severity": "critical"}),
            vec!["A numeric threshold goes in threshold_num, a text threshold in threshold_str."],
        ),
        Kind::DeviceGroup => (
            schemars::schema_for!(DeviceGroupInput),
            json!({"name": "Volets", "location": "Salon", "icon": "blinds"}),
            vec!["Deleting a group leaves its devices without a group."],
        ),
        Kind::DeviceLabel => (
            schemars::schema_for!(DeviceLabelInput),
            json!({"key": "temperature", "name": "Temperature", "icon_name": "thermometer", "unit": "°C"}),
            vec!["A single label per state key."],
        ),
        Kind::MqttSource => (
            schemars::schema_for!(MqttSourceInput),
            json!({"name": "Capteurs maison", "topic_pattern": "maison/+/etat", "auto_device": true, "auto_device_type": "sensor"}),
            vec![
                "The handler code (handler_code) is Rhai: writing, changing or removing it requires the `administration` scope, it is never readable here and an update that does not mention it keeps it.",
                "Device types created: sensor, binary_sensor, switch, light, cover, climate...",
            ],
        ),
        Kind::WeatherStation => (
            schemars::schema_for!(WeatherStationInput),
            json!({"name": "Maison", "latitude": 43.3, "longitude": 5.4, "altitude": 20}),
            vec!["Published on meteo/<mqtt_topic_suffix>."],
        ),
        Kind::PrometheusRule => (
            schemars::schema_for!(PrometheusRuleInput),
            json!({"name": "CaveTropHumide", "group": "maison", "expr": "dominet_device_state{key=\"humidity\"} > 80", "summary": "Cave trop humide", "severity": "warning"}),
            vec![
                "Saving rewrites rules.yml then reloads Prometheus; if the reload fails the rule stays saved (the rules page shows the state).",
            ],
        ),
        Kind::GrafanaRule => (
            schemars::schema_for!(GrafanaRuleInput),
            json!({"title": "Cave trop humide", "folder": "Maison", "group": "cave", "expr": "dominet_device_state{key=\"humidity\"} > 80", "summary": "Cave trop humide"}),
            vec![
                "With a data source other than `prometheus`, `expr` is raw SQL run by Grafana: writing or changing it requires the `administration` scope.",
                "Saving rewrites rules.yaml then reloads Grafana.",
            ],
        ),
        Kind::Dashboard => (
            schemars::schema_for!(DashboardInput),
            json!({"name": "Maison", "owner": "admin", "is_default": true}),
            vec![
                "Deleting a dashboard deletes its widgets (config_delete with dry_run gives the count).",
                "The owner is mandatory at creation and no longer changes.",
            ],
        ),
        Kind::Widget => (
            schemars::schema_for!(WidgetInput),
            json!({"dashboard": "admin/Maison", "type": "device", "title": "Salon", "device": "Lampe-salon", "col": 0, "row": 0, "width": 2, "height": 2}),
            vec![
                "Types: device (device field), scene (scene field), camera (camera field), grafana (config), text (content, sanitised HTML), grille (grid).",
                "A widget can neither leave the grid nor overlap another one: the refusal is announced from the dry_run.",
                "Replacing a widget (with id) never changes its dashboard.",
            ],
        ),
        Kind::Device => (
            schemars::schema_for!(DeviceInput),
            json!({"friendly_name": "Lampe-salon", "device_type": "light", "group": "Salon", "location": "Salon", "base_topic": "zigbee2mqtt"}),
            vec![
                "Only the settings can be changed: the state, availability, manufacturer and IEEE address come from Zigbee2MQTT.",
                "Deleting a device deletes its history; its widgets stay, without a device.",
                "Changing friendly_name changes the listened topic: the device will no longer answer if it keeps its old name on the Zigbee2MQTT side.",
            ],
        ),
        Kind::Navbar => (
            schemars::schema_for!(NavbarInput),
            json!({"icon_name": "layout-grid", "label": "Tableaux", "url": "/dashboard", "position": "top", "sort_order": 1}),
            vec![
                "The icon must exist in Lucide; an address is internal (/...) or http(s), never javascript:.",
                "A label shared between the top and the bottom is designated by its id.",
            ],
        ),
        Kind::Camera => (
            schemars::schema_for!(CameraConfigInput),
            json!({"name": "Entree", "ip": "192.168.1.60", "path": "stream1", "rtsp_user": "viewer", "rtsp_pass": "(password)",
                   "recording_mode": "motion", "onvif": true}),
            vec![
                "Reserved to the `administration` scope, for reading as well as writing: it holds the address and user names.",
                "Passwords (rtsp_pass, onvif_pass, web_pass) and the API key (web_api_key) can be written but never read: config_get only says has_rtsp_pass...; an omitted or empty field keeps the secret in place.",
                "An omitted field takes its default value (not the current value): to modify, start from config_get.",
                "Deleting a camera deletes its recordings, snapshots and events (config_delete with dry_run says so).",
                "cameras_sync brings MediaMTX and the camera back in line after a change.",
            ],
        ),
        Kind::Detector => (
            schemars::schema_for!(DetectorInput),
            json!({"enabled": true, "save_frames": false}),
            vec![
                "A single element, `detector` (id 1): it can be modified, neither created nor deleted.",
                "The detection model and engine are not set here.",
            ],
        ),
    };
    json!({
        "kind": kind.name(),
        "description": kind.description(),
        "schema": serde_json::to_value(schema).unwrap_or(Value::Null),
        "example": example,
        "notes": notes,
    })
}

// ------------------------------------------------------------------ names

async fn all_devices(state: &AppState) -> Result<Vec<Device>, ConfigError> {
    state
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .map_err(|e| storage("unreadable devices", &e))
}

fn device_names(devices: &[Device]) -> HashMap<i64, String> {
    devices
        .iter()
        .map(|d| (d.id.0, d.friendly_name.clone()))
        .collect()
}

/// A device by numeric id or by exact name (case-insensitive).
fn find_device(devices: &[Device], text: &str) -> Result<DeviceId, ConfigError> {
    let text = text.trim();
    let by_id = text.parse::<i64>().ok();
    devices
        .iter()
        .find(|d| Some(d.id.0) == by_id || d.friendly_name.eq_ignore_ascii_case(text))
        .map(|d| d.id)
        .ok_or_else(|| invalid(format!("device not found: {text}")))
}

async fn all_locations(state: &AppState) -> Result<Vec<Location>, ConfigError> {
    state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| storage("unreadable places", &e))
}

async fn find_group(state: &AppState, text: &str) -> Result<DeviceGroupId, ConfigError> {
    let text = text.trim();
    let by_id = text.parse::<i64>().ok();
    state
        .store
        .device_groups()
        .list()
        .await
        .map_err(|e| storage("unreadable groups", &e))?
        .iter()
        .find(|g| Some(g.id.0) == by_id || g.name.eq_ignore_ascii_case(text))
        .map(|g| g.id)
        .ok_or_else(|| invalid(format!("group not found: {text}")))
}

fn find_location(locations: &[Location], text: &str) -> Result<LocationId, ConfigError> {
    let text = text.trim();
    let by_id = text.parse::<i64>().ok();
    locations
        .iter()
        .find(|l| Some(l.id.0) == by_id || l.name.eq_ignore_ascii_case(text))
        .map(|l| l.id)
        .ok_or_else(|| invalid(format!("place not found: {text}")))
}

// ------------------------------------------------------------------ shapes (what is read is what can be written)

/// The detector is a single item, with this fixed id.
const DETECTOR_ID: i64 = 1;

fn device_json(
    d: &DeviceDraft,
    locations: &[Location],
    groups: &[dominet_domain::device::DeviceGroup],
) -> Value {
    let location = d
        .location
        .and_then(|l| locations.iter().find(|x| x.id == l))
        .map(|l| l.name.clone());
    let group = d
        .group
        .and_then(|g| groups.iter().find(|x| x.id == g))
        .map(|g| g.name.clone());
    json!({
        "friendly_name": d.friendly_name, "description": d.description, "device_type": d.device_type.as_str(),
        "group": group, "location": location, "base_topic": d.base_topic, "active": d.active, "icon": d.icon,
        "image_url": d.image_url, "tracked_keys": d.tracked_keys, "hidden_keys": d.hidden_keys,
        "grafana_dashboard": d.grafana_dashboard,
    })
}

/// A camera in the shape `config_upsert` accepts; `has` = which secrets are set (stream, ONVIF, web password, API key).
#[allow(
    clippy::cast_possible_truncation,
    reason = "a confidence between 0 and 1, as a whole percentage"
)]
fn camera_json(d: &CameraDraft, has: [bool; 4], locations: &[Location]) -> Value {
    let location = d
        .location_id
        .and_then(|l| locations.iter().find(|x| x.id.0 == l))
        .map(|l| l.name.clone());
    json!({
        "name": d.name, "location": location, "active": d.active, "priority": d.priority, "notes": d.notes,
        "proto": d.proto.as_str(), "ip": d.ip, "port": d.port, "path": d.path,
        "rtsp_user": d.rtsp_user, "has_rtsp_pass": has[0], "fps": d.fps,
        "rtsp_any_port": d.rtsp_any_port, "rtsp_transport_tcp": d.rtsp_transport_tcp,
        "recording_mode": d.recording_mode.as_str(), "night_mode_api": d.night_mode_api.as_str(),
        "web_port": d.web_port, "web_user": d.web_user, "has_web_pass": has[2], "has_web_api_key": has[3],
        "onvif": d.onvif, "onvif_port": d.onvif_port, "onvif_user": d.onvif_user, "has_onvif_pass": has[1],
        "main_token": d.main_token, "ptz": d.ptz,
        "yolo_enabled": d.yolo_enabled, "yolo_classes": d.yolo_classes,
        "yolo_min_confidence_pct": (d.yolo_min_confidence * 100.0).round() as i64,
    })
}

fn navbar_json(d: &NavbarDraft) -> Value {
    json!({
        "icon_name": d.icon_name, "label": d.label, "url": d.url, "url_target": d.url_target,
        "admin_only": d.admin_only, "position": d.position.as_str(), "sort_order": d.sort_order,
    })
}

fn location_json(name: &str, description: &str, active: bool) -> Value {
    json!({"name": name, "description": description, "active": active})
}

fn scene_json(
    name: &str,
    icon: &str,
    description: &str,
    location: Option<&str>,
    active: bool,
    actions: Vec<(String, Value)>,
) -> Value {
    json!({
        "name": name, "icon": icon, "description": description, "location": location, "active": active,
        "actions": actions.into_iter().map(|(device, command)| json!({"device": device, "command": command})).collect::<Vec<_>>(),
    })
}

/// An automation in the shape `config_upsert` accepts. The script is never shown: `script` is `null`, `has_script` tells.
fn automation_json(d: &AutomationDraft) -> Value {
    let parts = d.parts();
    let condition = if parts.condition.as_object().is_some_and(|c| !c.is_empty()) {
        parts.condition
    } else {
        Value::Null
    };
    json!({
        "name": d.name, "enabled": d.enabled, "description": d.description,
        "trigger": {"type": parts.trigger_type, "config": parts.trigger_config},
        "condition": condition,
        "action": {"type": parts.action_type, "config": parts.action_config},
        "script": Value::Null,
        "has_script": matches!(d.action, Action::Script) && !d.script.is_empty(),
    })
}

fn alert_rule_json(d: &AlertRuleDraft, devices: &HashMap<i64, String>) -> Value {
    json!({
        "name": d.name,
        "device": devices.get(&d.device.0).cloned().unwrap_or_else(|| format!("#{}", d.device.0)),
        "key": d.key, "operator": d.operator.as_str(),
        "threshold_num": d.threshold_num, "threshold_str": d.threshold_str,
        "severity": d.severity.as_str(), "cooldown_minutes": d.cooldown_minutes,
        "notify_email": d.notify_email, "notify_sms": d.notify_sms, "notify_android": d.notify_android,
        "enabled": d.enabled, "description": d.description,
    })
}

fn mqtt_source_json(m: &MqttSource) -> Value {
    json!({
        "name": m.name, "topic_pattern": m.topic_pattern, "qos": m.qos, "active": m.active,
        "log_messages": m.log_messages, "auto_device": m.auto_device,
        "auto_device_type": m.auto_device_type.map(DeviceType::as_str),
        "base_topic": m.base_topic, "description": m.description,
        "handler_code": Value::Null,
        "has_handler_code": !m.handler_code.trim().is_empty(),
    })
}

fn weather_station_json(w: &WeatherStation) -> Value {
    json!({
        "name": w.name, "latitude": w.latitude, "longitude": w.longitude, "altitude": w.altitude,
        "timezone": w.timezone, "mqtt_topic_suffix": w.mqtt_topic_suffix,
        "refresh_minutes": w.refresh_minutes, "enabled": w.enabled, "description": w.description,
    })
}

fn prometheus_rule_json(d: &PrometheusRuleDraft) -> Value {
    json!({
        "name": d.name, "group": d.group, "expr": d.expr, "summary": d.summary,
        "severity": d.severity.as_str(), "for_duration": d.for_duration,
        "description": d.description, "eval_interval": d.eval_interval, "active": d.active,
    })
}

fn grafana_rule_json(d: &GrafanaRuleDraft) -> Value {
    json!({
        "title": d.title, "uid": d.uid, "folder": d.folder, "group": d.group, "expr": d.expr,
        "summary": d.summary, "datasource_uid": d.datasource_uid, "group_interval": d.group_interval,
        "query_window": d.query_window, "for_duration": d.for_duration,
        "severity": d.severity.as_str(), "description": d.description,
        "no_data_state": d.no_data_state, "exec_err_state": d.exec_err_state, "active": d.active,
    })
}

/// `owner/name`, the way a dashboard is named in the tools.
fn dashboard_label(owner: &str, name: &str) -> String {
    format!("{owner}/{name}")
}

async fn owner_names(state: &AppState) -> Result<HashMap<i64, String>, ConfigError> {
    let users = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| storage("unreadable accounts", &e))?;
    Ok(users.into_iter().map(|u| (u.id.0, u.username)).collect())
}

/// `(id, owner/name)` of every dashboard.
async fn dashboard_labels(state: &AppState) -> Result<Vec<(i64, String)>, ConfigError> {
    let owners = owner_names(state).await?;
    Ok(state
        .store
        .dashboards()
        .list_all()
        .await
        .map_err(|e| storage("unreadable dashboards", &e))?
        .into_iter()
        .map(|d| {
            let owner = owners.get(&d.owner.0).cloned().unwrap_or_default();
            (d.id.0, dashboard_label(&owner, &d.name))
        })
        .collect())
}

fn widget_json(
    w: &dominet_domain::dashboard::Widget,
    dashboard: &str,
    devices: &HashMap<i64, String>,
    scenes: &[(i64, String)],
) -> Value {
    json!({
        "dashboard": dashboard, "type": w.widget_type.as_str(), "title": w.title,
        "presentation": w.presentation.as_str(),
        "device": w.device_id.map(|id| devices.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))),
        "scene": w.scene_id.map(|id| scenes.iter().find(|(s, _)| *s == id).map_or_else(|| format!("#{id}"), |(_, n)| n.clone())),
        "camera": w.camera_uuid.map(|u| u.to_string()),
        "config": w.config, "content": w.content,
        "col": w.rect.col, "row": w.rect.row, "width": w.rect.width, "height": w.rect.height,
    })
}

fn location_value(l: &Location) -> Value {
    location_json(&l.name, &l.description, l.active)
}

fn scene_value(s: &Scene, locations: &[Location], devices: &HashMap<i64, String>) -> Value {
    let location = s
        .location
        .and_then(|id| locations.iter().find(|l| l.id == id))
        .map(|l| l.name.as_str());
    scene_json(
        &s.name,
        &s.icon,
        &s.description,
        location,
        s.active,
        s.actions
            .iter()
            .map(|a| {
                (
                    devices
                        .get(&a.device.0)
                        .cloned()
                        .unwrap_or_else(|| format!("#{}", a.device.0)),
                    a.payload.clone(),
                )
            })
            .collect(),
    )
}

// ------------------------------------------------------------------ reading

/// One item in the shape `config_upsert` accepts, with its id; `None` if it does not exist.
async fn read_one(state: &AppState, kind: Kind, id: i64) -> Result<Option<Value>, ConfigError> {
    Ok(match kind {
        Kind::Location => state
            .store
            .locations()
            .get(LocationId(id))
            .await
            .map_err(|e| storage("unreadable place", &e))?
            .map(|l| location_value(&l)),
        Kind::Scene => {
            let Some(scene) = state
                .store
                .scenes()
                .get(SceneId(id))
                .await
                .map_err(|e| storage("unreadable scene", &e))?
            else {
                return Ok(None);
            };
            let (locations, devices) = (all_locations(state).await?, all_devices(state).await?);
            Some(scene_value(&scene, &locations, &device_names(&devices)))
        }
        Kind::Automation => state
            .store
            .automations()
            .get(AutomationId(id))
            .await
            .map_err(|e| storage("unreadable automation", &e))?
            .map(|a| automation_json(&a.draft)),
        Kind::AlertRule => {
            let rules = state
                .store
                .alert_rules()
                .list()
                .await
                .map_err(|e| storage("unreadable rules", &e))?;
            let devices = device_names(&all_devices(state).await?);
            rules
                .into_iter()
                .find(|r| r.id.0 == id)
                .map(|r| alert_rule_json(&r.rule, &devices))
        }
        Kind::DeviceGroup => {
            let locations = all_locations(state).await?;
            state
                .store
                .device_groups()
                .get(DeviceGroupId(id))
                .await
                .map_err(|e| storage("unreadable group", &e))?
                .map(|g| {
                    let location = g
                        .location
                        .and_then(|l| locations.iter().find(|x| x.id == l))
                        .map(|l| l.name.clone());
                    json!({"name": g.name, "location": location, "description": g.description, "icon": g.icon})
                })
        }
        Kind::DeviceLabel => state
            .store
            .device_labels()
            .get(DeviceLabelId(id))
            .await
            .map_err(|e| storage("unreadable label", &e))?
            .map(
                |l| json!({"key": l.key, "name": l.name, "icon_name": l.icon_name, "unit": l.unit}),
            ),
        Kind::MqttSource => state
            .store
            .mqtt_sources()
            .get(MqttSourceId(id))
            .await
            .map_err(|e| storage("unreadable source", &e))?
            .map(|m| mqtt_source_json(&m)),
        Kind::WeatherStation => state
            .store
            .weather()
            .get(WeatherStationId(id))
            .await
            .map_err(|e| storage("unreadable station", &e))?
            .map(|w| weather_station_json(&w)),
        Kind::PrometheusRule => state
            .store
            .prometheus_rules()
            .get(PrometheusRuleId(id))
            .await
            .map_err(|e| storage("unreadable rule", &e))?
            .map(|r| prometheus_rule_json(&r.draft)),
        Kind::GrafanaRule => state
            .store
            .grafana_rules()
            .get(GrafanaRuleId(id))
            .await
            .map_err(|e| storage("unreadable rule", &e))?
            .map(|r| grafana_rule_json(&r.draft)),
        Kind::Dashboard => {
            let owners = owner_names(state).await?;
            state
                .store
                .dashboards()
                .get(DashboardId(id))
                .await
                .map_err(|e| storage("unreadable dashboard", &e))?
                .map(|d| {
                    json!({
                        "name": d.name,
                        "owner": owners.get(&d.owner.0).cloned().unwrap_or_default(),
                        "is_default": d.is_default,
                    })
                })
        }
        Kind::Widget => {
            let Some(w) = state
                .store
                .widgets()
                .get(WidgetId(id))
                .await
                .map_err(|e| storage("unreadable widget", &e))?
            else {
                return Ok(None);
            };
            let labels = dashboard_labels(state).await?;
            let label = labels
                .iter()
                .find(|(d, _)| *d == w.dashboard.0)
                .map(|(_, l)| l.clone())
                .unwrap_or_default();
            let devices = device_names(&all_devices(state).await?);
            let scenes = names_of(state, Kind::Scene).await?;
            Some(widget_json(&w, &label, &devices, &scenes))
        }
        Kind::Device => {
            let Some(d) = state
                .store
                .devices()
                .get(DeviceId(id))
                .await
                .map_err(|e| storage("unreadable device", &e))?
            else {
                return Ok(None);
            };
            let locations = all_locations(state).await?;
            let groups = state
                .store
                .device_groups()
                .list()
                .await
                .map_err(|e| storage("unreadable groups", &e))?;
            Some(device_json(
                &DeviceDraft {
                    friendly_name: d.friendly_name,
                    description: d.description,
                    device_type: d.device_type,
                    group: d.group,
                    location: d.location,
                    base_topic: d.base_topic,
                    active: d.active,
                    icon: d.icon,
                    image_url: d.image_url,
                    tracked_keys: d.tracked_keys,
                    hidden_keys: d.hidden_keys,
                    grafana_dashboard: d.grafana_dashboard,
                },
                &locations,
                &groups,
            ))
        }
        Kind::Navbar => state
            .store
            .navbar()
            .get(NavbarItemId(id))
            .await
            .map_err(|e| storage("unreadable navigation", &e))?
            .map(|i| {
                navbar_json(&NavbarDraft {
                    icon_name: i.icon_name,
                    label: i.label,
                    url: i.url,
                    url_target: i.url_target,
                    admin_only: i.admin_only,
                    position: i.position,
                    sort_order: i.sort_order,
                })
            }),
        Kind::Camera => {
            let Some(c) = state
                .cameras
                .get(CameraId(id))
                .await
                .map_err(|e| storage("unreadable camera", &e))?
            else {
                return Ok(None);
            };
            let locations = all_locations(state).await?;
            let d = &c.config;
            let has = [
                !d.rtsp_pass.is_empty(),
                !d.onvif_pass.is_empty(),
                !d.web_pass.is_empty(),
                !d.web_api_key.is_empty(),
            ];
            Some(camera_json(d, has, &locations))
        }
        Kind::Detector => {
            if id == DETECTOR_ID {
                let c = state
                    .store
                    .detector_config()
                    .get()
                    .await
                    .map_err(|e| storage("unreadable detector", &e))?;
                Some(json!({"enabled": c.enabled, "save_frames": c.save_frames}))
            } else {
                None
            }
        }
    })
}

/// The `(id, name)` of every item of a kind.
async fn names_of(state: &AppState, kind: Kind) -> Result<Vec<(i64, String)>, ConfigError> {
    Ok(match kind {
        Kind::Location => all_locations(state)
            .await?
            .into_iter()
            .map(|l| (l.id.0, l.name))
            .collect(),
        Kind::Scene => state
            .store
            .scenes()
            .list()
            .await
            .map_err(|e| storage("unreadable scenes", &e))?
            .into_iter()
            .map(|s| (s.id.0, s.name))
            .collect(),
        Kind::Automation => state
            .store
            .automations()
            .list()
            .await
            .map_err(|e| storage("unreadable automations", &e))?
            .into_iter()
            .map(|a| (a.id.0, a.draft.name))
            .collect(),
        Kind::AlertRule => state
            .store
            .alert_rules()
            .list()
            .await
            .map_err(|e| storage("unreadable rules", &e))?
            .into_iter()
            .map(|r| (r.id.0, r.rule.name))
            .collect(),
        Kind::DeviceGroup => state
            .store
            .device_groups()
            .list()
            .await
            .map_err(|e| storage("unreadable groups", &e))?
            .into_iter()
            .map(|g| (g.id.0, g.name))
            .collect(),
        Kind::DeviceLabel => state
            .store
            .device_labels()
            .list()
            .await
            .map_err(|e| storage("unreadable labels", &e))?
            .into_iter()
            .map(|l| (l.id.0, l.key))
            .collect(),
        Kind::MqttSource => state
            .store
            .mqtt_sources()
            .list()
            .await
            .map_err(|e| storage("unreadable sources", &e))?
            .into_iter()
            .map(|m| (m.id.0, m.name))
            .collect(),
        Kind::WeatherStation => state
            .store
            .weather()
            .list()
            .await
            .map_err(|e| storage("unreadable stations", &e))?
            .into_iter()
            .map(|w| (w.id.0, w.name))
            .collect(),
        Kind::PrometheusRule => state
            .store
            .prometheus_rules()
            .list()
            .await
            .map_err(|e| storage("unreadable rules", &e))?
            .into_iter()
            .map(|r| (r.id.0, r.draft.name))
            .collect(),
        Kind::GrafanaRule => state
            .store
            .grafana_rules()
            .list()
            .await
            .map_err(|e| storage("unreadable rules", &e))?
            .into_iter()
            .map(|r| (r.id.0, r.draft.title))
            .collect(),
        Kind::Dashboard => dashboard_labels(state).await?,
        Kind::Widget => {
            let mut out = vec![];
            for (dashboard, label) in dashboard_labels(state).await? {
                for w in state
                    .store
                    .widgets()
                    .list(DashboardId(dashboard))
                    .await
                    .map_err(|e| storage("unreadable widgets", &e))?
                {
                    out.push((w.id.0, format!("{label} #{} {}", w.id.0, w.title)));
                }
            }
            out
        }
        Kind::Device => all_devices(state)
            .await?
            .into_iter()
            .map(|d| (d.id.0, d.friendly_name))
            .collect(),
        Kind::Navbar => state
            .store
            .navbar()
            .list()
            .await
            .map_err(|e| storage("unreadable navigation", &e))?
            .into_iter()
            .map(|i| (i.id.0, i.label))
            .collect(),
        Kind::Camera => state
            .store
            .cameras()
            .list()
            .await
            .map_err(|e| storage("unreadable cameras", &e))?
            .into_iter()
            .map(|c| (c.id.0, c.config.name))
            .collect(),
        Kind::Detector => vec![(DETECTOR_ID, "detector".to_string())],
    })
}

/// Resolves an item given by numeric id or by name.
async fn resolve(state: &AppState, kind: Kind, item: &str) -> Result<(i64, String), ConfigError> {
    let item = item.trim();
    let by_id = item.parse::<i64>().ok();
    names_of(state, kind)
        .await?
        .into_iter()
        .find(|(id, name)| Some(*id) == by_id || name.eq_ignore_ascii_case(item))
        .ok_or_else(|| ConfigError::NotFound(format!("{} not found: {item}", kind.name())))
}

/// Every item of a kind: `{id, name, ...}` in the shape that can be written back.
///
/// # Errors
/// Storage failure.
pub async fn list(state: &AppState, kind: Kind) -> Result<Vec<Value>, ConfigError> {
    let mut out = vec![];
    for (id, name) in names_of(state, kind).await? {
        let data = read_one(state, kind, id).await?.unwrap_or(Value::Null);
        out.push(json!({"id": id, "name": name, "data": data}));
    }
    Ok(out)
}

/// One item (id or name), in the shape that can be written back.
///
/// # Errors
/// Not found, or storage failure.
pub async fn get(state: &AppState, kind: Kind, item: &str) -> Result<Value, ConfigError> {
    let (id, name) = resolve(state, kind, item).await?;
    let data = read_one(state, kind, id)
        .await?
        .ok_or_else(|| ConfigError::NotFound(format!("{} not found: {item}", kind.name())))?;
    Ok(json!({"kind": kind.name(), "id": id, "name": name, "data": data}))
}

// ------------------------------------------------------------------ writing

enum Draft {
    Location(LocationDraft),
    Scene(SceneDraft),
    // Boxed: an automation is by far the largest of the four.
    Automation(Box<AutomationDraft>),
    AlertRule(AlertRuleDraft),
    DeviceGroup(DeviceGroupDraft),
    DeviceLabel(DeviceLabelDraft),
    MqttSource(Box<MqttSource>),
    WeatherStation(WeatherStation),
    PrometheusRule(PrometheusRuleDraft),
    GrafanaRule(GrafanaRuleDraft),
    Dashboard {
        owner: Option<dominet_domain::user::UserId>,
        draft: DashboardDraft,
    },
    Widget {
        dashboard: DashboardId,
        draft: Box<WidgetDraft>,
    },
    Device(Box<DeviceDraft>),
    Navbar(NavbarDraft),
    Detector {
        enabled: bool,
        save_frames: bool,
    },
    Camera(Box<CameraPlan>),
}

/// A camera to save: its settings, and the secrets to write (`Keep` leaves the one in place).
struct CameraPlan {
    draft: CameraDraft,
    rtsp_pass: SecretInput,
    onvif_pass: SecretInput,
    web_pass: SecretInput,
    web_api_key: SecretInput,
}

/// What a `config_upsert` would do, ready to be applied.
pub struct Plan {
    pub kind: Kind,
    pub id: Option<i64>,
    pub name: String,
    /// The item as it is now (`None` for a creation).
    pub before: Option<Value>,
    /// The item as it would be.
    pub after: Value,
    /// Writes, changes or removes Rhai code: needs the `administration` scope.
    pub needs_admin: bool,
    draft: Draft,
}

impl Plan {
    /// Why this write needs the `administration` scope, for the refusal.
    #[must_use]
    pub fn admin_reason(&self) -> &'static str {
        match self.kind {
            Kind::Camera => "modify a camera (address, stream, credentials)",
            _ => "write, change or remove Rhai code",
        }
    }

    /// Top-level fields that differ, `[{field, from, to}]` (empty for a creation).
    #[must_use]
    pub fn changes(&self) -> Vec<Value> {
        let Some(Value::Object(before)) = &self.before else {
            return vec![];
        };
        let Value::Object(after) = &self.after else {
            return vec![];
        };
        let mut out = vec![];
        for (field, to) in after {
            let from = before.get(field).unwrap_or(&Value::Null);
            if from != to {
                out.push(json!({"field": field, "from": from, "to": to}));
            }
        }
        out
    }
}

/// Reads `data` into the draft the interface would build, checks everything a real save checks, and says what would change.
///
/// # Errors
/// [`ConfigError::Invalid`] with one line per problem, [`ConfigError::NotFound`] for an unknown `id`.
#[allow(clippy::too_many_lines)]
pub async fn plan_upsert(
    state: &AppState,
    kind: Kind,
    id: Option<i64>,
    data: Value,
) -> Result<Plan, ConfigError> {
    // The detector is one item: whatever the caller says, it is the one that exists.
    let id = if kind == Kind::Detector {
        Some(DETECTOR_ID)
    } else {
        id
    };
    let before =
        match id {
            Some(id) => Some(read_one(state, kind, id).await?.ok_or_else(|| {
                ConfigError::NotFound(format!("{} #{id} not found", kind.name()))
            })?),
            None => None,
        };
    match kind {
        Kind::Location => {
            let input: LocationInput = parse_input(data)?;
            let draft = LocationDraft {
                name: input.name.trim().to_string(),
                description: input.description.trim().to_string(),
                active: input.active,
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: location_json(&draft.name, &draft.description, draft.active),
                needs_admin: false,
                draft: Draft::Location(draft),
            })
        }
        Kind::Scene => {
            let input: SceneInput = parse_input(data)?;
            let (locations, devices) = (all_locations(state).await?, all_devices(state).await?);
            let mut draft = SceneDraft::new(input.name.trim());
            draft.icon = input.icon.trim().to_string();
            draft.description = input.description.trim().to_string();
            draft.active = input.active;
            if let Some(text) = input
                .location
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.location = Some(find_location(&locations, text)?);
            }
            for (n, action) in input.actions.iter().enumerate() {
                check_command(&action.command).map_err(|_| {
                    invalid(format!(
                        "actions[{n}].command: must be a non-empty JSON object ({{\"state\": \"CLOSE\"}}), 1 KB and 3 levels at most"
                    ))
                })?;
                draft.actions.push(SceneActionDraft {
                    device: find_device(&devices, &action.device).map_err(|_| {
                        invalid(format!(
                            "actions[{n}].device: device not found: {}",
                            action.device
                        ))
                    })?,
                    payload: action.command.clone(),
                });
            }
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            // A name is unique: said now, in a dry run, rather than at the write.
            let others = names_of(state, kind).await?;
            if others
                .iter()
                .any(|(other, name)| Some(*other) != id && name.eq_ignore_ascii_case(&draft.name))
            {
                return Err(invalid("name: already taken"));
            }
            let names = device_names(&devices);
            let location_name = draft
                .location
                .and_then(|l| locations.iter().find(|x| x.id == l))
                .map(|l| l.name.clone());
            let after = scene_json(
                &draft.name,
                &draft.icon,
                &draft.description,
                location_name.as_deref(),
                draft.active,
                draft
                    .actions
                    .iter()
                    .map(|a| {
                        (
                            names.get(&a.device.0).cloned().unwrap_or_default(),
                            a.payload.clone(),
                        )
                    })
                    .collect(),
            );
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after,
                needs_admin: false,
                draft: Draft::Scene(draft),
            })
        }
        Kind::Automation => {
            let input: AutomationInput = parse_input(data)?;
            // The code now saved, to keep it when the payload does not mention it.
            let existing_script = match id {
                Some(id) => state
                    .store
                    .automations()
                    .get(AutomationId(id))
                    .await
                    .map_err(|e| storage("unreadable automation", &e))?
                    .map(|a| a.draft.script),
                None => None,
            }
            .unwrap_or_default();
            let script = input
                .script
                .clone()
                .unwrap_or_else(|| existing_script.clone());
            let trigger_config = if input.trigger.config.is_null() {
                json!({})
            } else {
                input.trigger.config.clone()
            };
            let action_config = if input.action.config.is_null() {
                json!({})
            } else {
                input.action.config.clone()
            };
            let draft = AutomationDraft::from_parts(
                input.name.trim(),
                input.enabled,
                input.description.trim(),
                &input.trigger.r#type,
                &trigger_config,
                &input.condition.clone().unwrap_or(Value::Null),
                &input.action.r#type,
                &action_config,
                &script,
            )
            .map_err(|p| problems(&config_service::field_problems(p)))?;
            let found = config_service::check_automation(state, &draft)
                .await
                .map_err(|e| storage("checking an automation", &e))?;
            if !found.is_empty() {
                return Err(problems(&found));
            }
            let final_script = if matches!(draft.action, Action::Script) {
                draft.script.as_str()
            } else {
                ""
            };
            let old_script = if id.is_some() {
                existing_script.as_str()
            } else {
                ""
            };
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: automation_json(&draft),
                needs_admin: final_script != old_script,
                draft: Draft::Automation(Box::new(draft)),
            })
        }
        Kind::AlertRule => {
            let input: AlertRuleInput = parse_input(data)?;
            let devices = all_devices(state).await?;
            let device = find_device(&devices, &input.device)
                .map_err(|_| invalid(format!("device: device not found: {}", input.device)))?;
            let mut draft = AlertRuleDraft::new(input.name.trim(), device, input.key.trim());
            draft.operator = Operator::parse(input.operator.trim())
                .ok_or_else(|| invalid("operator : gt, gte, lt, lte, eq, neq ou contains"))?;
            if let Some(text) = input.severity.as_deref() {
                draft.severity = Severity::parse(text.trim())
                    .ok_or_else(|| invalid("severity : info, warning ou critical"))?;
            }
            draft.threshold_num = input.threshold_num;
            draft.threshold_str = input.threshold_str.trim().to_string();
            if let Some(v) = input.cooldown_minutes {
                draft.cooldown_minutes = v;
            }
            if let Some(v) = input.notify_email {
                draft.notify_email = v;
            }
            if let Some(v) = input.notify_sms {
                draft.notify_sms = v;
            }
            if let Some(v) = input.notify_android {
                draft.notify_android = v;
            }
            draft.enabled = input.enabled;
            draft.description = input.description.trim().to_string();
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let names = device_names(&devices);
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: alert_rule_json(&draft, &names),
                needs_admin: false,
                draft: Draft::AlertRule(draft),
            })
        }
        Kind::DeviceGroup => {
            let input: DeviceGroupInput = parse_input(data)?;
            let locations = all_locations(state).await?;
            let location = match input
                .location
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                Some(text) => Some(find_location(&locations, text)?),
                None => None,
            };
            let draft = DeviceGroupDraft {
                name: input.name.trim().to_string(),
                location,
                description: input.description.trim().to_string(),
                icon: input.icon.trim().to_string(),
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let taken = names_of(state, kind)
                .await?
                .into_iter()
                .any(|(other, name)| Some(other) != id && name.eq_ignore_ascii_case(&draft.name));
            if taken {
                return Err(invalid("name: already taken"));
            }
            let location_name = location
                .and_then(|l| locations.iter().find(|x| x.id == l))
                .map(|l| l.name.clone());
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: json!({"name": draft.name, "location": location_name, "description": draft.description, "icon": draft.icon}),
                needs_admin: false,
                draft: Draft::DeviceGroup(draft),
            })
        }
        Kind::DeviceLabel => {
            let input: DeviceLabelInput = parse_input(data)?;
            let draft = DeviceLabelDraft {
                key: input.key.trim().to_string(),
                name: input.name.trim().to_string(),
                icon_name: input.icon_name.trim().to_string(),
                unit: input.unit.trim().to_string(),
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let taken = names_of(state, kind)
                .await?
                .into_iter()
                .any(|(other, key)| Some(other) != id && key == draft.key);
            if taken {
                return Err(invalid("key: already taken"));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.key.clone(),
                before,
                after: json!({"key": draft.key, "name": draft.name, "icon_name": draft.icon_name, "unit": draft.unit}),
                needs_admin: false,
                draft: Draft::DeviceLabel(draft),
            })
        }
        Kind::MqttSource => {
            let input: MqttSourceInput = parse_input(data)?;
            let existing_code = match id {
                Some(id) => state
                    .store
                    .mqtt_sources()
                    .get(MqttSourceId(id))
                    .await
                    .map_err(|e| storage("unreadable source", &e))?
                    .map(|m| m.handler_code),
                None => None,
            }
            .unwrap_or_default();
            let mut draft = MqttSource::draft(input.name.trim(), input.topic_pattern.trim());
            draft.qos = input.qos;
            draft.active = input.active;
            draft.log_messages = input.log_messages;
            draft.auto_device = input.auto_device;
            draft.auto_device_type = Some(match input.auto_device_type.as_deref().map(str::trim) {
                Some(text) if !text.is_empty() => DeviceType::from_str(text).map_err(|_| {
                    invalid(format!("auto_device_type: unknown device type: {text}"))
                })?,
                _ => DeviceType::Sensor,
            });
            draft.base_topic = input.base_topic.trim().trim_end_matches('/').to_string();
            draft.description = input.description.trim().to_string();
            draft.handler_code = input
                .handler_code
                .clone()
                .unwrap_or_else(|| existing_code.clone());
            let found = config_service::check_mqtt_source(&draft);
            if !found.is_empty() {
                return Err(problems(&found));
            }
            let needs_admin = draft.handler_code != existing_code;
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: mqtt_source_json(&draft),
                needs_admin,
                draft: Draft::MqttSource(Box::new(draft)),
            })
        }
        Kind::WeatherStation => {
            let input: WeatherStationInput = parse_input(data)?;
            let mut draft =
                WeatherStation::draft(input.name.trim(), input.latitude, input.longitude);
            draft.altitude = input.altitude;
            if let Some(text) = input.timezone.as_deref() {
                draft.timezone = text.trim().to_string();
            }
            if let Some(text) = input.mqtt_topic_suffix.as_deref() {
                draft.mqtt_topic_suffix = text.trim().to_string();
            }
            if let Some(v) = input.refresh_minutes {
                draft.refresh_minutes = v;
            }
            draft.enabled = input.enabled;
            draft.description = input.description.trim().to_string();
            let found = config_service::field_problems(draft.problems());
            if !found.is_empty() {
                return Err(problems(&found));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: weather_station_json(&draft),
                needs_admin: false,
                draft: Draft::WeatherStation(draft),
            })
        }
        Kind::PrometheusRule => {
            let input: PrometheusRuleInput = parse_input(data)?;
            let mut draft = PrometheusRuleDraft {
                name: input.name.trim().to_string(),
                group: input.group.trim().to_string(),
                severity: Severity::Warning,
                expr: input.expr.trim().to_string(),
                for_duration: "5m".to_string(),
                summary: input.summary.trim().to_string(),
                description: input.description.trim().to_string(),
                eval_interval: input.eval_interval.trim().to_string(),
                active: input.active,
            };
            if let Some(text) = input.severity.as_deref() {
                draft.severity = Severity::parse(text.trim())
                    .ok_or_else(|| invalid("severity : info, warning ou critical"))?;
            }
            if let Some(text) = input.for_duration.as_deref() {
                draft.for_duration = text.trim().to_string();
            }
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let taken = names_of(state, kind)
                .await?
                .into_iter()
                .any(|(other, name)| Some(other) != id && name == draft.name);
            if taken {
                return Err(invalid("name: already taken"));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: prometheus_rule_json(&draft),
                needs_admin: false,
                draft: Draft::PrometheusRule(draft),
            })
        }
        Kind::GrafanaRule => {
            let input: GrafanaRuleInput = parse_input(data)?;
            let mut draft = GrafanaRuleDraft {
                uid: input.uid.trim().to_string(),
                title: input.title.trim().to_string(),
                folder: input.folder.trim().to_string(),
                group: input.group.trim().to_string(),
                group_interval: "1m".to_string(),
                datasource_uid: "prometheus".to_string(),
                expr: input.expr.trim().to_string(),
                query_window: 300,
                for_duration: "5m".to_string(),
                severity: Severity::Warning,
                summary: input.summary.trim().to_string(),
                description: input.description.trim().to_string(),
                no_data_state: NO_DATA_STATES[0].to_string(),
                exec_err_state: EXEC_ERR_STATES[0].to_string(),
                active: input.active,
            };
            if let Some(text) = input.datasource_uid.as_deref() {
                draft.datasource_uid = text.trim().to_string();
            }
            if let Some(text) = input.group_interval.as_deref() {
                draft.group_interval = text.trim().to_string();
            }
            if let Some(v) = input.query_window {
                draft.query_window = v;
            }
            if let Some(text) = input.for_duration.as_deref() {
                draft.for_duration = text.trim().to_string();
            }
            if let Some(text) = input.severity.as_deref() {
                draft.severity = Severity::parse(text.trim())
                    .ok_or_else(|| invalid("severity : info, warning ou critical"))?;
            }
            if let Some(text) = input.no_data_state.as_deref() {
                draft.no_data_state = text.trim().to_string();
            }
            if let Some(text) = input.exec_err_state.as_deref() {
                draft.exec_err_state = text.trim().to_string();
            }
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            // SQL run by Grafana is code: writing or changing it is an administration act.
            let old = match id {
                Some(id) => state
                    .store
                    .grafana_rules()
                    .get(GrafanaRuleId(id))
                    .await
                    .map_err(|e| storage("unreadable rule", &e))?
                    .map(|r| r.draft),
                None => None,
            };
            let is_sql = |d: &GrafanaRuleDraft| d.datasource_uid != "prometheus";
            let needs_admin = match &old {
                Some(o) => {
                    (is_sql(o) || is_sql(&draft))
                        && (o.expr != draft.expr || o.datasource_uid != draft.datasource_uid)
                }
                None => is_sql(&draft),
            };
            Ok(Plan {
                kind,
                id,
                name: draft.title.clone(),
                before,
                after: grafana_rule_json(&draft),
                needs_admin,
                draft: Draft::GrafanaRule(draft),
            })
        }
        Kind::Dashboard => {
            let input: DashboardInput = parse_input(data)?;
            let draft = DashboardDraft {
                name: input.name.trim().to_string(),
                is_default: input.is_default,
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let owner_text = input
                .owner
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let owners = owner_names(state).await?;
            let owner = match (id, owner_text) {
                (None, None) => return Err(invalid("owner: mandatory")),
                (None, Some(text)) => Some(
                    owners
                        .iter()
                        .find(|(_, name)| name.eq_ignore_ascii_case(text))
                        .map(|(id, _)| dominet_domain::user::UserId(*id))
                        .ok_or_else(|| invalid(format!("owner: account not found: {text}")))?,
                ),
                (Some(_), Some(text)) => {
                    let current = before
                        .as_ref()
                        .and_then(|b| b["owner"].as_str())
                        .unwrap_or_default();
                    if !current.eq_ignore_ascii_case(text) {
                        return Err(invalid("owner: the owner of a dashboard does not change"));
                    }
                    None
                }
                (Some(_), None) => None,
            };
            let owner_name = owner_text
                .map(str::to_string)
                .or_else(|| {
                    before
                        .as_ref()
                        .and_then(|b| b["owner"].as_str().map(str::to_string))
                })
                .unwrap_or_default();
            Ok(Plan {
                kind,
                id,
                name: dashboard_label(&owner_name, &draft.name),
                before,
                after: json!({"name": draft.name, "owner": owner_name, "is_default": draft.is_default}),
                needs_admin: false,
                draft: Draft::Dashboard { owner, draft },
            })
        }
        Kind::Widget => {
            let input: WidgetInput = parse_input(data)?;
            let (dashboard_id, dashboard_name) = resolve(state, Kind::Dashboard, &input.dashboard)
                .await
                .map_err(|_| {
                    invalid(format!(
                        "dashboard: dashboard not found: {}",
                        input.dashboard
                    ))
                })?;
            if let Some(Value::Object(old)) = &before {
                let current = old
                    .get("dashboard")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if current != dashboard_name {
                    return Err(invalid("dashboard: a widget does not change dashboard"));
                }
            }
            let widget_type = WidgetType::from_str(input.r#type.trim())
                .map_err(|_| invalid("type: device, scene, camera, grafana, text or grille"))?;
            let mut draft = WidgetDraft::new(widget_type);
            draft.title = input.title.trim().to_string();
            if let Some(text) = input
                .presentation
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.presentation = Presentation::from_str(text)
                    .map_err(|_| invalid(format!("presentation: unknown: {text}")))?;
            }
            if let Some(text) = input
                .device
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.device_id = Some(
                    find_device(&all_devices(state).await?, text)
                        .map_err(|_| invalid(format!("device: device not found: {text}")))?
                        .0,
                );
            }
            if let Some(text) = input
                .scene
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.scene_id = Some(
                    resolve(state, Kind::Scene, text)
                        .await
                        .map_err(|_| invalid(format!("scene: scene not found: {text}")))?
                        .0,
                );
            }
            if let Some(text) = input
                .camera
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.camera_uuid =
                    Some(uuid::Uuid::parse_str(text).map_err(|_| invalid("camera: invalid UUID"))?);
            }
            draft.config = if input.config.is_null() {
                json!({})
            } else {
                input.config.clone()
            };
            draft.content = if widget_type == WidgetType::Text {
                dominet_server::html::sanitize_widget_html(&input.content)
            } else {
                input.content.clone()
            };
            draft.rect = GridRect {
                col: input.col.unwrap_or(0),
                row: input.row.unwrap_or(0),
                width: input.width.unwrap_or(GridRect::DEFAULT.width),
                height: input.height.unwrap_or(GridRect::DEFAULT.height),
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            // The overlap is said now, in a dry run, rather than at the write.
            for other in state
                .store
                .widgets()
                .list(DashboardId(dashboard_id))
                .await
                .map_err(|e| storage("unreadable widgets", &e))?
            {
                let (a, b) = (draft.rect, other.rect);
                let overlaps = a.col < b.col + b.width
                    && b.col < a.col + a.width
                    && a.row < b.row + b.height
                    && b.row < a.row + a.height;
                if overlaps && Some(other.id.0) != id {
                    return Err(invalid(format!(
                        "col: overlaps widget #{} ({})",
                        other.id.0, other.title
                    )));
                }
            }
            let devices = device_names(&all_devices(state).await?);
            let scenes = names_of(state, Kind::Scene).await?;
            let preview = dominet_domain::dashboard::Widget {
                id: WidgetId(id.unwrap_or(0)),
                dashboard: DashboardId(dashboard_id),
                widget_type: draft.widget_type,
                title: draft.title.clone(),
                device_id: draft.device_id,
                scene_id: draft.scene_id,
                camera_uuid: draft.camera_uuid,
                presentation: draft.presentation,
                config: draft.config.clone(),
                content: draft.content.clone(),
                rect: draft.rect,
            };
            Ok(Plan {
                kind,
                id,
                name: format!("{} #{}", dashboard_name, id.unwrap_or(0)),
                before,
                after: widget_json(&preview, &dashboard_name, &devices, &scenes),
                needs_admin: false,
                draft: Draft::Widget {
                    dashboard: DashboardId(dashboard_id),
                    draft: Box::new(draft),
                },
            })
        }
        Kind::Device => {
            let input: DeviceInput = parse_input(data)?;
            let locations = all_locations(state).await?;
            let groups = state
                .store
                .device_groups()
                .list()
                .await
                .map_err(|e| storage("unreadable groups", &e))?;
            let text = |t: &Option<String>| {
                t.as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            };
            let group = match text(&input.group) {
                Some(t) => Some(find_group(state, &t).await?),
                None => None,
            };
            let location = match text(&input.location) {
                Some(t) => Some(find_location(&locations, &t)?),
                None => None,
            };
            let device_type = DeviceType::from_str(input.device_type.trim()).map_err(|_| {
                invalid(format!(
                    "device_type: unknown ({}); expected: {}",
                    input.device_type,
                    DeviceType::ALL.map(DeviceType::as_str).join(", ")
                ))
            })?;
            let keys = |list: &[String]| -> Vec<String> {
                list.iter()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect()
            };
            let draft = DeviceDraft {
                friendly_name: input.friendly_name.trim().to_string(),
                description: input.description.trim().to_string(),
                device_type,
                group,
                location,
                base_topic: input.base_topic.trim().trim_end_matches('/').to_string(),
                active: input.active,
                icon: input.icon.trim().to_string(),
                image_url: input.image_url.trim().to_string(),
                tracked_keys: keys(&input.tracked_keys),
                hidden_keys: keys(&input.hidden_keys),
                grafana_dashboard: input.grafana_dashboard,
            };
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let taken = names_of(state, kind)
                .await?
                .into_iter()
                .any(|(other, name)| Some(other) != id && name == draft.friendly_name);
            if taken {
                return Err(invalid("friendly_name: already taken"));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.friendly_name.clone(),
                before,
                after: device_json(&draft, &locations, &groups),
                needs_admin: false,
                draft: Draft::Device(Box::new(draft)),
            })
        }
        Kind::Navbar => {
            let input: NavbarInput = parse_input(data)?;
            let position = NavPosition::from_str(input.position.trim())
                .map_err(|_| invalid("position: expected top or bottom"))?;
            let draft = NavbarDraft {
                icon_name: input.icon_name.trim().to_string(),
                label: input.label.trim().to_string(),
                url: input.url.trim().to_string(),
                url_target: input.url_target.trim().to_string(),
                admin_only: input.admin_only,
                position,
                sort_order: input.sort_order,
            };
            let found = config_service::check_navbar(&draft);
            if !found.is_empty() {
                return Err(problems(&found));
            }
            Ok(Plan {
                kind,
                id,
                name: draft.label.clone(),
                before,
                after: navbar_json(&draft),
                needs_admin: false,
                draft: Draft::Navbar(draft),
            })
        }
        Kind::Camera => {
            let input: CameraConfigInput = parse_input(data)?;
            let locations = all_locations(state).await?;
            let existing = match id {
                Some(id) => state
                    .cameras
                    .get(CameraId(id))
                    .await
                    .map_err(|e| storage("unreadable camera", &e))?,
                None => None,
            };
            let mut draft = CameraDraft::new(input.name.trim());
            let mut bad: Vec<String> = vec![];
            if let Some(text) = input
                .location
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                draft.location_id = Some(find_location(&locations, text)?.0);
            }
            let mut choice =
                |field: &str, text: &Option<String>, set: &mut dyn FnMut(&str) -> bool| {
                    if let Some(t) = text.as_deref().map(str::trim)
                        && !set(t)
                    {
                        bad.push(format!("{field}: unknown value ({t})"));
                    }
                };
            choice("proto", &input.proto, &mut |t| {
                StreamProto::from_str(t).map(|v| draft.proto = v).is_ok()
            });
            choice("recording_mode", &input.recording_mode, &mut |t| {
                RecordingMode::from_str(t)
                    .map(|v| draft.recording_mode = v)
                    .is_ok()
            });
            choice("night_mode_api", &input.night_mode_api, &mut |t| {
                NightApi::from_str(t)
                    .map(|v| draft.night_mode_api = v)
                    .is_ok()
            });
            if let Some(pct) = input.yolo_min_confidence_pct {
                if (0..=100).contains(&pct) {
                    draft.yolo_min_confidence = f64::from(pct) / 100.0;
                } else {
                    bad.push("yolo_min_confidence_pct: between 0 and 100".to_string());
                }
            }
            if !bad.is_empty() {
                return Err(ConfigError::Invalid(bad));
            }
            draft.active = input.active.unwrap_or(true);
            draft.priority = input.priority.unwrap_or(draft.priority);
            draft.notes = input.notes.trim().to_string();
            draft.ip = input.ip.trim().to_string();
            draft.port = input.port.unwrap_or(draft.port);
            draft.path = input.path.trim().to_string();
            draft.rtsp_user = input.rtsp_user.trim().to_string();
            draft.fps = input.fps.unwrap_or(draft.fps);
            draft.rtsp_any_port = input.rtsp_any_port;
            draft.rtsp_transport_tcp = input.rtsp_transport_tcp;
            draft.web_port = input.web_port.unwrap_or(draft.web_port);
            draft.web_user = input.web_user.trim().to_string();
            draft.onvif = input.onvif;
            draft.onvif_port = input.onvif_port.unwrap_or(draft.onvif_port);
            draft.onvif_user = input.onvif_user.trim().to_string();
            draft.ptz = input.ptz;
            draft.yolo_enabled = input.yolo_enabled;
            draft.yolo_classes = input
                .yolo_classes
                .iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
            draft.main_token = input
                .main_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string();
            let found = draft.problems();
            if !found.is_empty() {
                return Err(problems(&config_service::field_problems(found)));
            }
            let secret = |t: &Option<String>| SecretInput::from_form(t.as_deref().unwrap_or(""));
            let secrets = (
                secret(&input.rtsp_pass),
                secret(&input.onvif_pass),
                secret(&input.web_pass),
                secret(&input.web_api_key),
            );
            let was = |f: fn(&CameraDraft) -> bool| existing.as_ref().is_some_and(|c| f(&c.config));
            let has = [
                matches!(secrets.0, SecretInput::Set(_)) || was(|d| !d.rtsp_pass.is_empty()),
                matches!(secrets.1, SecretInput::Set(_)) || was(|d| !d.onvif_pass.is_empty()),
                matches!(secrets.2, SecretInput::Set(_)) || was(|d| !d.web_pass.is_empty()),
                matches!(secrets.3, SecretInput::Set(_)) || was(|d| !d.web_api_key.is_empty()),
            ];
            Ok(Plan {
                kind,
                id,
                name: draft.name.clone(),
                before,
                after: camera_json(&draft, has, &locations),
                needs_admin: true,
                draft: Draft::Camera(Box::new(CameraPlan {
                    draft,
                    rtsp_pass: secrets.0,
                    onvif_pass: secrets.1,
                    web_pass: secrets.2,
                    web_api_key: secrets.3,
                })),
            })
        }
        Kind::Detector => {
            let input: DetectorInput = parse_input(data)?;
            Ok(Plan {
                kind,
                id,
                name: "detector".to_string(),
                before,
                after: json!({"enabled": input.enabled, "save_frames": input.save_frames}),
                needs_admin: false,
                draft: Draft::Detector {
                    enabled: input.enabled,
                    save_frames: input.save_frames,
                },
            })
        }
    }
}

/// Saves a plan through the service the interface uses. Returns the id of the item.
///
/// # Errors
/// [`ConfigError::Invalid`] if a check at write time refuses (name taken...), [`ConfigError::NotFound`], storage failure.
pub async fn apply(state: &AppState, plan: &Plan) -> Result<i64, ConfigError> {
    fn outcome<T>(
        out: Result<SaveOutcome<T>, dominet_domain::RepoError>,
        id_of: impl FnOnce(&T) -> i64,
        what: &str,
    ) -> Result<i64, ConfigError> {
        match out.map_err(|e| storage(what, &e))? {
            SaveOutcome::Saved(item) => Ok(id_of(&item)),
            SaveOutcome::Invalid(list) => Err(problems(&list)),
            SaveOutcome::NotFound => Err(ConfigError::NotFound("element not found".to_string())),
            SaveOutcome::Blocked(_) => Err(invalid("save blocked")),
        }
    }
    match &plan.draft {
        Draft::Location(d) => outcome(
            config_service::save_location(state, plan.id.map(LocationId), d).await,
            |l: &Location| l.id.0,
            "saving a place",
        ),
        Draft::Scene(d) => outcome(
            config_service::save_scene(state, plan.id.map(SceneId), d).await,
            |s: &Scene| s.id.0,
            "saving a scene",
        ),
        Draft::Automation(d) => outcome(
            config_service::save_automation(state, plan.id.map(AutomationId), d).await,
            |a: &Automation| a.id.0,
            "saving an automation",
        ),
        Draft::AlertRule(d) => outcome(
            config_service::save_alert_rule(state, plan.id.map(AlertRuleId), d).await,
            |r: &AlertRule| r.id.0,
            "saving an alert rule",
        ),
        Draft::DeviceGroup(d) => outcome(
            config_service::save_device_group(state, plan.id.map(DeviceGroupId), d).await,
            |g: &dominet_domain::device::DeviceGroup| g.id.0,
            "saving a group",
        ),
        Draft::DeviceLabel(d) => outcome(
            config_service::save_device_label(state, plan.id.map(DeviceLabelId), d).await,
            |l: &dominet_domain::device::DeviceLabel| l.id.0,
            "saving a label",
        ),
        Draft::MqttSource(d) => outcome(
            config_service::save_mqtt_source(state, plan.id.map(MqttSourceId), d).await,
            |m: &MqttSource| m.id.0,
            "saving an MQTT source",
        ),
        Draft::WeatherStation(d) => outcome(
            config_service::save_weather_station(state, plan.id.map(WeatherStationId), d).await,
            |w: &WeatherStation| w.id.0,
            "saving a weather station",
        ),
        Draft::PrometheusRule(d) => outcome(
            config_service::save_prometheus_rule(state, plan.id.map(PrometheusRuleId), d).await,
            |r: &PrometheusRule| r.id.0,
            "saving a Prometheus rule",
        ),
        Draft::GrafanaRule(d) => outcome(
            config_service::save_grafana_rule(state, plan.id.map(GrafanaRuleId), d).await,
            |r: &GrafanaRule| r.id.0,
            "saving a Grafana rule",
        ),
        Draft::Dashboard { owner, draft } => outcome(
            config_service::save_dashboard(state, plan.id.map(DashboardId), *owner, draft).await,
            |d: &dominet_domain::dashboard::Dashboard| d.id.0,
            "saving a dashboard",
        ),
        Draft::Widget { dashboard, draft } => outcome(
            config_service::save_widget(state, plan.id.map(WidgetId), *dashboard, draft).await,
            |w: &dominet_domain::dashboard::Widget| w.id.0,
            "saving a widget",
        ),
        Draft::Device(d) => outcome(
            config_service::save_device(state, plan.id.map(DeviceId), d).await,
            |d: &Device| d.id.0,
            "saving a device",
        ),
        Draft::Navbar(d) => outcome(
            config_service::save_navbar(state, plan.id.map(NavbarItemId), d).await,
            |i: &dominet_domain::navbar::NavbarItem| i.id.0,
            "saving a navigation entry",
        ),
        Draft::Detector {
            enabled,
            save_frames,
        } => {
            config_service::set_detector(state, *enabled, *save_frames)
                .await
                .map_err(|e| storage("saving the detector", &e))?;
            Ok(DETECTOR_ID)
        }
        Draft::Camera(c) => {
            let input = CameraSave {
                draft: c.draft.clone(),
                rtsp_pass: c.rtsp_pass.clone(),
                onvif_pass: c.onvif_pass.clone(),
                web_pass: c.web_pass.clone(),
                web_api_key: c.web_api_key.clone(),
            };
            match state.cameras.save(plan.id.map(CameraId), input).await {
                Ok((camera, _sync)) => Ok(camera.id.0),
                Err(CameraError::Invalid(found)) => {
                    Err(problems(&config_service::field_problems(found)))
                }
                Err(CameraError::NotFound) => {
                    Err(ConfigError::NotFound("camera not found".to_string()))
                }
                Err(e @ (CameraError::MasterKeyMissing | CameraError::Unreadable)) => {
                    Err(invalid(e.to_string()))
                }
                Err(e) => Err(storage("saving a camera", &e)),
            }
        }
    }
}

// ------------------------------------------------------------------ Rhai scripts

fn script_kind(text: &str) -> Result<Kind, ConfigError> {
    match Kind::parse(text) {
        Some(k @ (Kind::Automation | Kind::MqttSource)) => Ok(k),
        _ => Err(invalid(
            "kind: automation (an automation's script) or mqtt_source (handler code)",
        )),
    }
}

/// The Rhai code of an automation or of an MQTT source: `(id, name, code)`.
///
/// # Errors
/// Unknown kind or item, storage failure.
pub async fn script_get(
    state: &AppState,
    kind: &str,
    item: &str,
) -> Result<(i64, String, String), ConfigError> {
    let kind = script_kind(kind)?;
    let (id, name) = resolve(state, kind, item).await?;
    let code = match kind {
        Kind::Automation => state
            .store
            .automations()
            .get(AutomationId(id))
            .await
            .map_err(|e| storage("unreadable automation", &e))?
            .map(|a| a.draft.script),
        _ => state
            .store
            .mqtt_sources()
            .get(MqttSourceId(id))
            .await
            .map_err(|e| storage("unreadable source", &e))?
            .map(|m| m.handler_code),
    };
    Ok((id, name, code.unwrap_or_default()))
}

/// A plan that replaces the code of an automation or of an MQTT source and nothing else (same checks as `config_upsert`: the
/// code must compile, the action of the automation must be `script`).
///
/// # Errors
/// Unknown kind or item, invalid code, storage failure.
pub async fn plan_script_set(
    state: &AppState,
    kind: &str,
    item: &str,
    code: &str,
) -> Result<Plan, ConfigError> {
    let kind = script_kind(kind)?;
    let (id, _) = resolve(state, kind, item).await?;
    let mut data = read_one(state, kind, id)
        .await?
        .ok_or_else(|| ConfigError::NotFound(format!("{} #{id} not found", kind.name())))?;
    let field = if kind == Kind::Automation {
        if data["action"]["type"] != "script" {
            return Err(invalid(
                "action: this automation does not have the `script` action; change its action first with config_upsert",
            ));
        }
        "script"
    } else {
        "handler_code"
    };
    data[field] = json!(code);
    plan_upsert(state, kind, Some(id), data).await
}

// ------------------------------------------------------------------ deleting

/// What a deletion would take with it.
pub struct DeletePlan {
    pub id: i64,
    pub name: String,
    /// In words, one line each; empty when nothing else is affected.
    pub impact: Vec<String>,
}

/// Resolves the item and lists what its deletion would affect, without deleting.
///
/// # Errors
/// Not found, or storage failure.
pub async fn plan_delete(
    state: &AppState,
    kind: Kind,
    item: &str,
) -> Result<DeletePlan, ConfigError> {
    let (id, name) = resolve(state, kind, item).await?;
    let mut impact = vec![];
    match kind {
        Kind::Location => {
            let devices = all_devices(state)
                .await?
                .iter()
                .filter(|d| d.location == Some(LocationId(id)))
                .count();
            let scenes = state
                .store
                .scenes()
                .list()
                .await
                .map_err(|e| storage("unreadable scenes", &e))?
                .iter()
                .filter(|s| s.location == Some(LocationId(id)))
                .count();
            if devices + scenes > 0 {
                impact.push(format!(
                    "{devices} device(s) and {scenes} scene(s) are attached to it: deletion is refused while they are"
                ));
            }
        }
        Kind::Scene => {
            let using: Vec<String> = state
                .store
                .automations()
                .list()
                .await
                .map_err(|e| storage("unreadable automations", &e))?
                .into_iter()
                .filter(|a| matches!(&a.draft.action, Action::Scene { scene } if scene.0 == id))
                .map(|a| a.draft.name)
                .collect();
            if !using.is_empty() {
                impact.push(format!(
                    "{} automation(s) run this scene and will fail afterwards: {}",
                    using.len(),
                    using.join(", ")
                ));
            }
            impact.push("the widgets that pointed to it lose their scene".to_string());
        }
        Kind::Automation => {
            let runs = state
                .store
                .automation_runs()
                .recent(AutomationId(id), 1000)
                .await
                .map_err(|e| storage("unreadable log", &e))?
                .len();
            impact.push(format!(
                "its execution log ({runs} row(s)) is deleted with it"
            ));
        }
        Kind::AlertRule => {
            impact.push("alerts already created stay, without a rule".to_string());
        }
        Kind::DeviceGroup => {
            impact.push("its devices stay, without a group".to_string());
        }
        Kind::DeviceLabel => {
            impact.push("the key will be displayed again with its raw name".to_string());
        }
        Kind::MqttSource => {
            impact.push(
                "the devices it had created stay; its topics are no longer listened to".to_string(),
            );
        }
        Kind::WeatherStation => {
            impact.push("the device it fed stays, without new forecasts".to_string());
        }
        Kind::PrometheusRule => {
            impact.push("it disappears from rules.yml and Prometheus is reloaded".to_string());
        }
        Kind::GrafanaRule => {
            impact.push("it disappears from rules.yaml and Grafana is reloaded".to_string());
        }
        Kind::Dashboard => {
            let count = state
                .store
                .widgets()
                .list(DashboardId(id))
                .await
                .map_err(|e| storage("unreadable widgets", &e))?
                .len();
            impact.push(format!("its {count} widget(s) are deleted with it"));
        }
        Kind::Widget => {
            impact.push(
                "the widget disappears from the dashboard; nothing else depends on it".to_string(),
            );
        }
        Kind::Device => {
            let widgets = {
                let mut n = 0;
                for (dashboard, _) in dashboard_labels(state).await? {
                    n += state
                        .store
                        .widgets()
                        .list(DashboardId(dashboard))
                        .await
                        .map_err(|e| storage("unreadable widgets", &e))?
                        .iter()
                        .filter(|w| w.device_id == Some(id))
                        .count();
                }
                n
            };
            impact.push(
                "its history and state are deleted with it; if it is still published by Zigbee2MQTT it will reappear at the next discovery"
                    .to_string(),
            );
            if widgets > 0 {
                impact.push(format!("{widgets} widget(s) stay, without a device"));
            }
        }
        Kind::Navbar => {
            impact.push("the entry disappears from the navigation bar".to_string());
        }
        Kind::Camera => {
            impact.push(
                "its recordings, snapshots and events are deleted with it (files included), as well as its MediaMTX stream"
                    .to_string(),
            );
        }
        Kind::Detector => {
            return Err(invalid(
                "the detector cannot be deleted: turn it off with config_upsert (enabled: false)",
            ));
        }
    }
    Ok(DeletePlan { id, name, impact })
}

/// Deletes an item through the service the interface uses.
///
/// # Errors
/// [`ConfigError::NotFound`], [`ConfigError::Invalid`] when still in use, storage failure.
pub async fn apply_delete(state: &AppState, kind: Kind, id: i64) -> Result<(), ConfigError> {
    if kind == Kind::Camera {
        return match state.cameras.delete(CameraId(id)).await {
            Ok(_sync) => Ok(()),
            Err(CameraError::NotFound) => {
                Err(ConfigError::NotFound(format!("camera #{id} not found")))
            }
            Err(e) => Err(storage("deleting a camera", &e)),
        };
    }
    let outcome = match kind {
        Kind::Location => config_service::delete_location(state, LocationId(id)).await,
        Kind::Scene => config_service::delete_scene(state, SceneId(id)).await,
        Kind::Automation => config_service::delete_automation(state, AutomationId(id)).await,
        Kind::AlertRule => config_service::delete_alert_rule(state, AlertRuleId(id)).await,
        Kind::DeviceGroup => config_service::delete_device_group(state, DeviceGroupId(id)).await,
        Kind::DeviceLabel => config_service::delete_device_label(state, DeviceLabelId(id)).await,
        Kind::MqttSource => config_service::delete_mqtt_source(state, MqttSourceId(id)).await,
        Kind::WeatherStation => {
            config_service::delete_weather_station(state, WeatherStationId(id)).await
        }
        Kind::PrometheusRule => {
            config_service::delete_prometheus_rule(state, PrometheusRuleId(id)).await
        }
        Kind::GrafanaRule => config_service::delete_grafana_rule(state, GrafanaRuleId(id)).await,
        Kind::Dashboard => config_service::delete_dashboard(state, DashboardId(id)).await,
        Kind::Widget => config_service::delete_widget(state, WidgetId(id)).await,
        Kind::Device => config_service::delete_device(state, DeviceId(id)).await,
        Kind::Navbar => config_service::delete_navbar(state, NavbarItemId(id)).await,
        Kind::Detector => return Err(invalid("the detector cannot be deleted")),
        Kind::Camera => unreachable!("handled above"),
    }
    .map_err(|e| storage("suppression", &e))?;
    match outcome {
        DeleteOutcome::Deleted => Ok(()),
        DeleteOutcome::NotFound => Err(ConfigError::NotFound(format!(
            "{} #{id} not found",
            kind.name()
        ))),
        DeleteOutcome::InUse => Err(invalid("still used elsewhere: detach it first")),
        DeleteOutcome::Blocked(_) => Err(invalid("deletion blocked")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_kind_is_found_by_its_identifier_or_by_the_usual_french_word() {
        use super::Kind;
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k), "{}", k.name());
        }
        for (word, kind) in [
            ("emplacement", Kind::Location),
            (" Emplacements ", Kind::Location),
            ("scène", Kind::Scene),
            ("Règle d'alerte", Kind::AlertRule),
            ("tableau de bord", Kind::Dashboard),
            ("caméra", Kind::Camera),
            ("appareil", Kind::Device),
        ] {
            assert_eq!(Kind::parse(word), Some(kind), "{word}");
        }
        assert_eq!(Kind::parse("n'importe quoi"), None);
    }

    use super::*;

    #[test]
    fn kinds_round_trip_and_the_schema_of_each_describes_its_payload() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
            let s = schema(k);
            assert_eq!(s["kind"], k.name());
            assert!(s["schema"]["properties"].is_object(), "{}: {s}", k.name());
            assert!(s["example"].is_object());
            assert!(!s["notes"].as_array().unwrap().is_empty());
        }
        assert_eq!(Kind::parse("  scene "), Some(Kind::Scene));
        assert_eq!(Kind::parse("frigo"), None);
    }

    #[test]
    fn unknown_fields_are_refused_so_a_typo_is_not_silently_dropped() {
        let bad = parse_input::<LocationInput>(json!({"name": "Salon", "actif": true}));
        let ConfigError::Invalid(lines) = bad.unwrap_err() else {
            panic!("expected Invalid")
        };
        assert!(lines[0].contains("actif"), "{lines:?}");
        let ok: LocationInput = parse_input(json!({"name": "Salon"})).unwrap();
        assert!(ok.active, "active defaults to true");
    }

    #[test]
    fn problems_read_as_sentences() {
        let p = |field: &str, kind| FieldProblem {
            field: field.into(),
            kind,
        };
        assert_eq!(
            problem_text(&p("name", ProblemKind::Required)),
            "name : mandatory"
        );
        assert_eq!(
            problem_text(&p("name", ProblemKind::Taken)),
            "name : already taken"
        );
        assert!(
            problem_text(&p("script", ProblemKind::Syntax("ligne 2".into()))).contains("ligne 2")
        );
        assert!(
            problem_text(&p("x", ProblemKind::OutOfRange { min: 0, max: 9 })).contains("0 to 9")
        );
    }

    #[test]
    fn a_plan_lists_only_the_top_level_fields_that_differ() {
        let plan = Plan {
            kind: Kind::Location,
            id: Some(1),
            name: "Salon".into(),
            before: Some(json!({"name": "Salon", "description": "", "active": true})),
            after: json!({"name": "Salon", "description": "RDC", "active": true}),
            needs_admin: false,
            draft: Draft::Location(LocationDraft {
                name: "Salon".into(),
                description: "RDC".into(),
                active: true,
            }),
        };
        assert_eq!(
            plan.changes(),
            [json!({"field": "description", "from": "", "to": "RDC"})]
        );
        let created = Plan {
            before: None,
            ..plan
        };
        assert!(created.changes().is_empty());
    }
}
