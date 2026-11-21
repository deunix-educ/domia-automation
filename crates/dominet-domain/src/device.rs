// SPDX-License-Identifier: AGPL-3.0-or-later
//! Appareils, groupes, libelles, historique d'etat, sources MQTT et stations meteo (R3, ADR 0025).
//!
//! Les regles qui decident ce que devient un message MQTT (type deduit des `exposes`, lignes d'historique d'un etat) sont
//! ici, pures, pour etre appliquees de la meme facon par l'ecoute MQTT, les tests de rejeu et les pages. Elles reprennent
//! **a l'identique** l'implementation de reference, y compris ses particularites, parce que le critere
//! d'acceptation de R3 est un rejeu qui donne les memes resultats ; chaque particularite est signalee.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{Problem, ValidationError};
use crate::location::LocationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceGroupId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceLabelId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MqttSourceId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeatherStationId(pub i64);

/// Topic de base de Zigbee2MQTT.
pub const Z2M_BASE_TOPIC: &str = "zigbee2mqtt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceType {
    Sensor,
    BinarySensor,
    Switch,
    Light,
    Cover,
    Climate,
    Remote,
    Generic,
}

impl DeviceType {
    pub const ALL: [Self; 8] = [
        Self::Sensor,
        Self::BinarySensor,
        Self::Switch,
        Self::Light,
        Self::Cover,
        Self::Climate,
        Self::Remote,
        Self::Generic,
    ];

    /// Valeur stockee, identique a l'implementation de reference.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sensor => "sensor",
            Self::BinarySensor => "binary_sensor",
            Self::Switch => "switch",
            Self::Light => "light",
            Self::Cover => "cover",
            Self::Climate => "climate",
            Self::Remote => "remote",
            Self::Generic => "generic",
        }
    }
}

impl FromStr for DeviceType {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| ValidationError::new(format!("unknown device type: \"{s}\"")))
    }
}

// ------------------------------------------------------------------ exposes Zigbee2MQTT

/// Aplatit les `exposes` **comme l'implementation de reference pour deduire le type** : seuls les elements `composite` sont
/// remplaces par leurs `features`. Particularite conservee : un `light`, un `switch` ou un `cover` Zigbee2MQTT (qui portent
/// aussi des `features`) ne sont **pas** aplatis, donc `brightness` ou `state` n'y sont pas vus et l'appareil reste
/// `generic`. Le rejeu contre l'implementation de reference verifie cette equivalence ; la corriger est une amelioration a decider a part.
#[must_use]
pub fn flatten_composites(exposes: &[Value]) -> Vec<&Value> {
    let mut out = vec![];
    for e in exposes {
        match (
            e.get("type").and_then(Value::as_str),
            e.get("features").and_then(Value::as_array),
        ) {
            (Some("composite"), Some(features)) => out.extend(flatten_composites(features)),
            _ => out.push(e),
        }
    }
    out
}

/// Aplatit **toutes** les `features` (pour lister les proprietes et leurs unites).
#[must_use]
pub fn flatten_features(exposes: &[Value]) -> Vec<&Value> {
    let mut out = vec![];
    for e in exposes {
        match e.get("features").and_then(Value::as_array) {
            Some(features) => out.extend(flatten_features(features)),
            None => out.push(e),
        }
    }
    out
}

/// Type d'un appareil deduit de ses `exposes`.
#[must_use]
pub fn infer_device_type(exposes: &[Value]) -> DeviceType {
    let flat = flatten_composites(exposes);
    let has = |p: &str| {
        flat.iter()
            .any(|f| f.get("property").and_then(Value::as_str) == Some(p))
    };
    // Noms des elements binaires de premier niveau (non aplatis), comme `feature_names` en reference.
    let binary_names: Vec<&str> = exposes
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("binary"))
        .filter_map(|e| e.get("name").and_then(Value::as_str))
        .collect();
    if has("cover_position") {
        DeviceType::Cover
    } else if has("temperature") {
        DeviceType::Sensor
    } else if has("brightness") {
        DeviceType::Light
    } else if has("state") {
        DeviceType::Switch
    } else if binary_names
        .iter()
        .any(|n| matches!(*n, "occupancy" | "contact"))
    {
        DeviceType::BinarySensor
    } else {
        DeviceType::Generic
    }
}

/// Unite de chaque propriete exposee.
#[must_use]
pub fn exposed_units(exposes: &[Value]) -> Vec<(String, String)> {
    flatten_features(exposes)
        .into_iter()
        .filter_map(|f| {
            let prop = f
                .get("property")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())?;
            let unit = f
                .get("unit")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())?;
            Some((prop.to_string(), unit.to_string()))
        })
        .collect()
}

/// Un appareil decrit par `zigbee2mqtt/bridge/devices`.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub friendly_name: String,
    pub ieee: String,
    pub vendor: String,
    pub model_id: String,
    pub description: String,
    pub exposes: Vec<Value>,
}

fn text(v: &Value, key: &str, max: usize) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(max)
        .collect()
}

impl Discovered {
    /// Lit un element de `bridge/devices` ; `None` pour le coordinateur ou un nom vide (ignores, comme en reference).
    /// Les textes sont tronques aux longueurs des colonnes (reference aurait leve une erreur de base).
    #[must_use]
    pub fn from_bridge_entry(entry: &Value) -> Option<Self> {
        if entry.get("type").and_then(Value::as_str) == Some("Coordinator") {
            return None;
        }
        let friendly_name = entry
            .get("friendly_name")
            .and_then(Value::as_str)?
            .trim()
            .to_string();
        if !is_valid_friendly_name(&friendly_name) {
            return None;
        }
        let empty = Value::Null;
        let definition = entry
            .get("definition")
            .filter(|d| d.is_object())
            .unwrap_or(&empty);
        Some(Self {
            ieee: text(entry, "ieee_address", 64),
            vendor: text(definition, "vendor", 64),
            model_id: text(definition, "model", 64),
            description: text(definition, "description", 256),
            exposes: definition
                .get("exposes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            friendly_name,
        })
    }

    /// Type d'un appareil **nouveau** (un appareil existant garde le sien).
    #[must_use]
    pub fn device_type(&self) -> DeviceType {
        infer_device_type(&self.exposes)
    }
}

// ------------------------------------------------------------------ noms et topics

/// Un nom d'appareil sert a construire des topics : ni caractere de controle, ni joker MQTT, ni '/' de tete ou de queue.
#[must_use]
pub fn is_valid_friendly_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 128
        && !name.chars().any(|c| c.is_control() || c == '+' || c == '#')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.contains("//")
}

/// Prefixe de topic : comme un nom, mais peut etre vide.
#[must_use]
pub fn is_valid_base_topic(base: &str) -> bool {
    base.is_empty() || (is_valid_friendly_name(base) && base.chars().count() <= 128)
}

/// Filtre d'abonnement MQTT valide : `+` occupe un niveau entier, `#` est le dernier niveau et l'occupe entier.
#[must_use]
pub fn is_valid_topic_filter(filter: &str) -> bool {
    if filter.is_empty()
        || filter.chars().count() > 255
        || filter.chars().any(|c| c == '\0' || c.is_control())
    {
        return false;
    }
    let levels: Vec<&str> = filter.split('/').collect();
    levels.iter().enumerate().all(|(i, l)| match *l {
        "#" => i + 1 == levels.len(),
        "+" => true,
        other => !other.contains('#') && !other.contains('+'),
    })
}

/// Topic de **publication** : non vide, 256 caracteres au plus, ni joker MQTT ni caractere de controle.
#[must_use]
pub fn is_valid_publish_topic(topic: &str) -> bool {
    !topic.is_empty()
        && topic.chars().count() <= 256
        && !topic
            .chars()
            .any(|c| c == '+' || c == '#' || c.is_control())
}

// ------------------------------------------------------------------ appareil

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: DeviceId,
    pub friendly_name: String,
    pub ieee: String,
    pub vendor: String,
    pub model_id: String,
    pub description: String,
    pub device_type: DeviceType,
    pub group: Option<DeviceGroupId>,
    pub location: Option<LocationId>,
    pub base_topic: String,
    pub state: Value,
    pub last_seen: Option<DateTime<Utc>>,
    pub available: bool,
    pub exposes: Vec<Value>,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: Vec<String>,
    pub hidden_keys: Vec<String>,
    pub grafana_dashboard: bool,
}

impl Device {
    /// Topic de l'etat : `{base_topic}/{friendly_name}` (ou le seul nom si le prefixe est vide).
    #[must_use]
    pub fn state_topic(&self) -> String {
        if self.base_topic.is_empty() {
            self.friendly_name.clone()
        } else {
            format!("{}/{}", self.base_topic, self.friendly_name)
        }
    }

    /// Topic des commandes.
    #[must_use]
    pub fn command_topic(&self) -> String {
        format!("{}/set", self.state_topic())
    }

    /// Identifiant du tableau de bord Grafana de l'appareil (deterministe, jamais stocke).
    #[must_use]
    pub fn grafana_uid(&self) -> String {
        format!("dominet-device-{}", self.id.0)
    }
}

/// Parametres modifiables d'un appareil (l'etat, la disponibilite et ce que decouvre Zigbee2MQTT ne le sont pas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDraft {
    pub friendly_name: String,
    pub description: String,
    pub device_type: DeviceType,
    pub group: Option<DeviceGroupId>,
    pub location: Option<LocationId>,
    pub base_topic: String,
    pub active: bool,
    pub icon: String,
    pub image_url: String,
    pub tracked_keys: Vec<String>,
    pub hidden_keys: Vec<String>,
    pub grafana_dashboard: bool,
}

pub(crate) fn is_state_key(k: &str) -> bool {
    !k.is_empty()
        && k.chars().count() <= 64
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn is_http_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && s.len() <= 512
        && !s.contains(char::is_whitespace)
}

fn key_list_problem(keys: &[String]) -> bool {
    keys.len() > 64
        || keys.iter().any(|k| !is_state_key(k))
        || keys.iter().enumerate().any(|(i, k)| keys[..i].contains(k))
}

impl DeviceDraft {
    pub const DESCRIPTION_MAX: usize = 256;
    pub const ICON_MAX: usize = 32;

    /// Brouillon minimal (appareil cree a la main).
    #[must_use]
    pub fn new(friendly_name: &str) -> Self {
        Self {
            friendly_name: friendly_name.to_string(),
            description: String::new(),
            device_type: DeviceType::Generic,
            group: None,
            location: None,
            base_topic: Z2M_BASE_TOPIC.to_string(),
            active: true,
            icon: String::new(),
            image_url: String::new(),
            tracked_keys: vec![],
            hidden_keys: vec!["update".to_string()],
            grafana_dashboard: false,
        }
    }

    /// Problemes champ par champ.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.friendly_name.trim().is_empty() {
            out.push(("friendly_name", Problem::Required));
        } else if self.friendly_name.chars().count() > 128 {
            out.push(("friendly_name", Problem::TooLong { max: 128 }));
        } else if !is_valid_friendly_name(&self.friendly_name) {
            out.push(("friendly_name", Problem::Invalid));
        }
        if self.description.chars().count() > Self::DESCRIPTION_MAX {
            out.push((
                "description",
                Problem::TooLong {
                    max: Self::DESCRIPTION_MAX,
                },
            ));
        }
        if self.base_topic.chars().count() > 128 {
            out.push(("base_topic", Problem::TooLong { max: 128 }));
        } else if !is_valid_base_topic(&self.base_topic) {
            out.push(("base_topic", Problem::Invalid));
        }
        if self.icon.chars().count() > Self::ICON_MAX {
            out.push((
                "icon",
                Problem::TooLong {
                    max: Self::ICON_MAX,
                },
            ));
        }
        if !self.image_url.is_empty() && !is_http_url(&self.image_url) {
            out.push(("image_url", Problem::Invalid));
        }
        if key_list_problem(&self.tracked_keys) {
            out.push(("tracked_keys", Problem::Invalid));
        }
        if key_list_problem(&self.hidden_keys) {
            out.push(("hidden_keys", Problem::Invalid));
        }
        out
    }
}

// ------------------------------------------------------------------ historique

/// Une ligne d'historique d'une cle suivie.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub key: String,
    pub value_num: Option<f64>,
    pub value_str: String,
}

/// Lignes d'historique d'un etat pour les cles suivies, dans l'ordre de `tracked_keys` :
/// booleen -> `value_num` 0/1 et `value_str` `True`/`False` ; nombre -> `value_num` ; texte -> `value_str` (tronque a
/// 128 caracteres) ; tout autre type (objet, tableau, null) et les cles absentes sont ignores.
#[must_use]
pub fn log_entries(tracked_keys: &[String], state: &Value) -> Vec<LogEntry> {
    tracked_keys
        .iter()
        .filter_map(|key| {
            let value = state.get(key)?;
            let (value_num, value_str) = match value {
                Value::Bool(b) => (
                    Some(f64::from(u8::from(*b))),
                    if *b { "True" } else { "False" }.to_string(),
                ),
                Value::Number(n) => (Some(n.as_f64()?), String::new()),
                Value::String(s) => (None, s.chars().take(128).collect()),
                _ => return None,
            };
            Some(LogEntry {
                key: key.clone(),
                value_num,
                value_str,
            })
        })
        .collect()
}

/// Un point d'historique lu.
#[derive(Debug, Clone, PartialEq)]
pub struct LogPoint {
    pub ts: DateTime<Utc>,
    pub value_num: Option<f64>,
    pub value_str: String,
}

// ------------------------------------------------------------------ groupes et libelles

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceGroup {
    pub id: DeviceGroupId,
    pub name: String,
    pub location: Option<LocationId>,
    pub description: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceGroupDraft {
    pub name: String,
    pub location: Option<LocationId>,
    pub description: String,
    pub icon: String,
}

impl DeviceGroupDraft {
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 64 {
            out.push(("name", Problem::TooLong { max: 64 }));
        }
        if self.description.chars().count() > 256 {
            out.push(("description", Problem::TooLong { max: 256 }));
        }
        if self.icon.chars().count() > 32 {
            out.push(("icon", Problem::TooLong { max: 32 }));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLabel {
    pub id: DeviceLabelId,
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLabelDraft {
    pub key: String,
    pub name: String,
    pub icon_name: String,
    pub unit: String,
}

impl DeviceLabelDraft {
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.key.is_empty() {
            out.push(("key", Problem::Required));
        } else if !is_state_key(&self.key) {
            out.push(("key", Problem::Invalid));
        }
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 128 {
            out.push(("name", Problem::TooLong { max: 128 }));
        }
        if self.icon_name.is_empty() {
            out.push(("icon_name", Problem::Required));
        } else if self.icon_name.chars().count() > 64 {
            out.push(("icon_name", Problem::TooLong { max: 64 }));
        }
        if self.unit.chars().count() > 32 {
            out.push(("unit", Problem::TooLong { max: 32 }));
        }
        out
    }
}

/// Libelles connus des cles Zigbee2MQTT usuelles : (cle, icone Lucide, nom, unite). Reprend `seed_device_labels`.
pub const KNOWN_LABELS: [(&str, &str, &str, &str); 25] = [
    ("action", "mouse-pointer-click", "Action", ""),
    ("battery", "battery", "Battery", "%"),
    ("battery_low", "battery-warning", "Low battery", ""),
    ("brightness", "sun", "Brightness", ""),
    ("color_temp", "thermometer-sun", "Color temperature", "K"),
    ("contact", "door-open", "Contact", ""),
    ("cpu_temperature", "cpu", "Température CPU", "°C"),
    ("current", "zap", "Current", "A"),
    ("energy", "zap", "Energy", "kWh"),
    ("humidity", "droplets", "Humidity", "%"),
    ("illuminance", "sun", "Brightness", "lx"),
    ("illuminance_lux", "sun", "Brightness", "lx"),
    ("linkquality", "wifi", "Signal quality", "lqi"),
    ("local_temperature", "thermometer", "Température", "°C"),
    ("occupancy", "person-standing", "Présence", ""),
    ("position", "move-horizontal", "Position", "%"),
    ("power", "zap", "Power", "W"),
    ("power_factor", "activity", "Power factor", ""),
    ("pressure", "gauge", "Pressure", "hPa"),
    ("smoke", "flame", "Smoke", ""),
    ("state", "toggle-right", "State", ""),
    ("tamper", "shield-alert", "Sabotage", ""),
    ("temperature", "thermometer", "Température", "°C"),
    ("voltage", "zap", "Tension", "V"),
    ("water_leak", "droplets", "Water leak", ""),
];

/// Libelle d'une cle : celui du referentiel `KNOWN_LABELS`, sinon icone neutre, la cle comme nom et aucune unite.
#[must_use]
pub fn known_label(key: &str) -> DeviceLabelDraft {
    let (icon, name, unit) = KNOWN_LABELS
        .iter()
        .find(|(k, ..)| *k == key)
        .map_or(("circle-dot", key, ""), |(_, icon, name, unit)| {
            (*icon, *name, *unit)
        });
    DeviceLabelDraft {
        key: key.to_string(),
        name: name.to_string(),
        icon_name: icon.to_string(),
        unit: unit.to_string(),
    }
}

// ------------------------------------------------------------------ sources MQTT

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttSource {
    pub id: MqttSourceId,
    pub name: String,
    pub topic_pattern: String,
    pub qos: u8,
    pub active: bool,
    pub log_messages: bool,
    pub auto_device: bool,
    /// Type des appareils crees automatiquement (`None` = type par defaut, capteur).
    pub auto_device_type: Option<DeviceType>,
    pub base_topic: String,
    /// Code la reference de la version precedente : conserve et affiche, execute a partir de R4.
    pub handler_code: String,
    pub description: String,
}

pub type MqttSourceDraft = MqttSource;

impl MqttSource {
    /// Brouillon d'une source (l'identifiant est ignore a l'enregistrement).
    #[must_use]
    pub fn draft(name: &str, topic_pattern: &str) -> Self {
        Self {
            id: MqttSourceId(0),
            name: name.to_string(),
            topic_pattern: topic_pattern.to_string(),
            qos: 0,
            active: true,
            log_messages: false,
            auto_device: false,
            auto_device_type: Some(DeviceType::Sensor),
            base_topic: String::new(),
            handler_code: String::new(),
            description: String::new(),
        }
    }

    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 64 {
            out.push(("name", Problem::TooLong { max: 64 }));
        }
        if self.topic_pattern.trim().is_empty() {
            out.push(("topic_pattern", Problem::Required));
        } else if self.topic_pattern.chars().count() > 255 {
            out.push(("topic_pattern", Problem::TooLong { max: 255 }));
        } else if !is_valid_topic_filter(&self.topic_pattern) {
            out.push(("topic_pattern", Problem::Invalid));
        }
        if self.qos > 2 {
            out.push(("qos", Problem::OutOfRange { min: 0, max: 2 }));
        }
        if self.base_topic.chars().count() > 128 {
            out.push(("base_topic", Problem::TooLong { max: 128 }));
        } else if !is_valid_base_topic(self.base_topic.trim_end_matches('/')) {
            out.push(("base_topic", Problem::Invalid));
        }
        if self.handler_code.chars().count() > 20_000 {
            out.push(("handler_code", Problem::TooLong { max: 20_000 }));
        }
        if self.description.chars().count() > 256 {
            out.push(("description", Problem::TooLong { max: 256 }));
        }
        out
    }
}

// ------------------------------------------------------------------ stations meteo

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherStation {
    pub id: WeatherStationId,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: i32,
    pub timezone: String,
    pub mqtt_topic_suffix: String,
    pub refresh_minutes: i16,
    pub enabled: bool,
    pub description: String,
}

pub type WeatherStationDraft = WeatherStation;

impl WeatherStation {
    /// Brouillon d'une station.
    #[must_use]
    pub fn draft(name: &str, latitude: f64, longitude: f64) -> Self {
        Self {
            id: WeatherStationId(0),
            name: name.to_string(),
            latitude,
            longitude,
            altitude: 0,
            timezone: "Europe/Paris".to_string(),
            mqtt_topic_suffix: "meteo_locale".to_string(),
            refresh_minutes: 10,
            enabled: true,
            description: String::new(),
        }
    }

    /// Topic publie : `meteo/<suffixe>` (annexe A de la ROADMAP, inchange).
    #[must_use]
    pub fn mqtt_topic(&self) -> String {
        format!("meteo/{}", self.mqtt_topic_suffix)
    }

    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 100 {
            out.push(("name", Problem::TooLong { max: 100 }));
        }
        if !(-90.0..=90.0).contains(&self.latitude) {
            out.push(("latitude", Problem::OutOfRange { min: -90, max: 90 }));
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            out.push((
                "longitude",
                Problem::OutOfRange {
                    min: -180,
                    max: 180,
                },
            ));
        }
        if !(-500..=9000).contains(&self.altitude) {
            out.push((
                "altitude",
                Problem::OutOfRange {
                    min: -500,
                    max: 9000,
                },
            ));
        }
        if self.timezone.trim().is_empty() {
            out.push(("timezone", Problem::Required));
        } else if self.timezone.chars().count() > 50
            || self
                .timezone
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            out.push(("timezone", Problem::Invalid));
        }
        let suffix = &self.mqtt_topic_suffix;
        if suffix.is_empty() {
            out.push(("mqtt_topic_suffix", Problem::Required));
        } else if suffix.chars().count() > 100
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            out.push(("mqtt_topic_suffix", Problem::Invalid));
        }
        if !(1..=1440).contains(&self.refresh_minutes) {
            out.push(("refresh_minutes", Problem::OutOfRange { min: 1, max: 1440 }));
        }
        if self.description.chars().count() > 255 {
            out.push(("description", Problem::TooLong { max: 255 }));
        }
        out
    }
}

/// Image televersee d'un appareil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceImage {
    pub content_type: String,
    pub data: Vec<u8>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
// `assert!(x.is_empty())` sur des listes de problemes typees : plus lisible que la comparaison a un tableau vide annote.
#[allow(clippy::assert_is_empty)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tracked(keys: &[&str]) -> Vec<String> {
        keys.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn device_types_round_trip_with_the_reference_names() {
        for t in DeviceType::ALL {
            assert_eq!(t.as_str().parse::<DeviceType>().unwrap(), t);
        }
        assert!("fan".parse::<DeviceType>().is_err());
    }

    #[test]
    fn the_type_is_inferred_from_the_exposes_in_the_reference_order() {
        let t = |v: Value| infer_device_type(v.as_array().unwrap());
        assert_eq!(
            t(json!([{"property": "cover_position"}, {"property": "temperature"}])),
            DeviceType::Cover
        );
        assert_eq!(
            t(json!([{"property": "temperature"}, {"property": "brightness"}])),
            DeviceType::Sensor
        );
        assert_eq!(
            t(json!([{"property": "brightness"}, {"property": "state"}])),
            DeviceType::Light
        );
        assert_eq!(t(json!([{"property": "state"}])), DeviceType::Switch);
        assert_eq!(
            t(json!([{"type": "binary", "name": "occupancy", "property": "occupancy"}])),
            DeviceType::BinarySensor
        );
        assert_eq!(
            t(json!([{"type": "binary", "name": "contact", "property": "contact"}])),
            DeviceType::BinarySensor
        );
        assert_eq!(
            t(json!([{"type": "numeric", "property": "battery"}])),
            DeviceType::Generic
        );
        assert_eq!(t(json!([])), DeviceType::Generic);
    }

    #[test]
    fn only_composites_are_flattened_to_infer_the_type_like_reference() {
        // Composite : ses features comptent.
        let composite = json!([{"type": "composite", "features": [{"property": "brightness"}]}]);
        assert_eq!(
            infer_device_type(composite.as_array().unwrap()),
            DeviceType::Light
        );
        // `light` Zigbee2MQTT (a des features mais n'est pas `composite`) : non aplati, l'appareil reste generique.
        let light = json!([{"type": "light", "features": [{"property": "state"}, {"property": "brightness"}]}]);
        assert_eq!(
            infer_device_type(light.as_array().unwrap()),
            DeviceType::Generic
        );
        // En revanche les unites voient toutes les features.
        let units = exposed_units(light.as_array().unwrap());
        assert!(units.is_empty());
        let with_unit = json!([{"type": "light", "features": [{"property": "color_temp", "unit": "mired"}]}, {"property": "temperature", "unit": "°C"}]);
        assert_eq!(
            exposed_units(with_unit.as_array().unwrap()),
            [
                ("color_temp".to_string(), "mired".to_string()),
                ("temperature".to_string(), "°C".to_string())
            ]
        );
    }

    #[test]
    fn bridge_entries_become_discovered_devices() {
        let entry = json!({
            "ieee_address": "0x00124b0000000001", "friendly_name": "  Bureau ", "type": "EndDevice",
            "definition": {"vendor": "Test", "model": "TH-01", "description": "Capteur", "exposes": [{"property": "temperature"}]}
        });
        let d = Discovered::from_bridge_entry(&entry).unwrap();
        assert_eq!(
            (
                d.friendly_name.as_str(),
                d.ieee.as_str(),
                d.vendor.as_str(),
                d.model_id.as_str()
            ),
            ("Bureau", "0x00124b0000000001", "Test", "TH-01")
        );
        assert_eq!(d.device_type(), DeviceType::Sensor);
        // Coordinateur, nom vide, nom a jokers : ignores.
        assert!(
            Discovered::from_bridge_entry(
                &json!({"type": "Coordinator", "friendly_name": "Coordinator"})
            )
            .is_none()
        );
        assert!(Discovered::from_bridge_entry(&json!({"friendly_name": "  "})).is_none());
        assert!(Discovered::from_bridge_entry(&json!({"friendly_name": "a/#"})).is_none());
        assert!(Discovered::from_bridge_entry(&json!("pas un objet")).is_none());
        // Sans definition (appareil non supporte) : valeurs vides, type generique.
        let bare =
            Discovered::from_bridge_entry(&json!({"friendly_name": "Unknown", "definition": null}))
                .unwrap();
        assert!(bare.exposes.is_empty() && bare.vendor.is_empty());
        assert_eq!(bare.device_type(), DeviceType::Generic);
        // Textes tronques aux colonnes.
        let long = Discovered::from_bridge_entry(&json!({"friendly_name": "x", "ieee_address": "i".repeat(200), "definition": {"vendor": "v".repeat(100)}})).unwrap();
        assert_eq!((long.ieee.len(), long.vendor.len()), (64, 64));
    }

    #[test]
    fn names_topics_and_filters_are_validated() {
        for ok in ["Bureau", "salon/lampe", "0x6f09/prod", "Cuisine (volet)"] {
            assert!(is_valid_friendly_name(ok), "{ok}");
        }
        for bad in [
            "",
            "a+b",
            "a#",
            "/a",
            "a/",
            "a//b",
            "a\nb",
            &"x".repeat(129),
        ] {
            assert!(!is_valid_friendly_name(bad), "{bad:?}");
        }
        assert!(
            is_valid_base_topic("")
                && is_valid_base_topic("zigbee2mqtt")
                && !is_valid_base_topic("a/#")
        );
        for ok in [
            "linky/#",
            "solarmax/+/prod",
            "a/b",
            "#",
            "+",
            "+/+/#",
            "a//b",
        ] {
            assert!(is_valid_topic_filter(ok), "{ok}");
        }
        for bad in ["", "a/#/b", "a#", "a+/b", "#/a", "a/b#", "a\0"] {
            assert!(!is_valid_topic_filter(bad), "{bad:?}");
        }
    }

    #[test]
    fn device_topics_follow_base_and_name() {
        let mut d = Device {
            id: DeviceId(7),
            friendly_name: "salon/lampe".into(),
            ieee: String::new(),
            vendor: String::new(),
            model_id: String::new(),
            description: String::new(),
            device_type: DeviceType::Light,
            group: None,
            location: None,
            base_topic: "zigbee2mqtt".into(),
            state: json!({}),
            last_seen: None,
            available: false,
            exposes: vec![],
            active: true,
            icon: String::new(),
            image_url: String::new(),
            tracked_keys: vec![],
            hidden_keys: vec![],
            grafana_dashboard: false,
        };
        assert_eq!(d.state_topic(), "zigbee2mqtt/salon/lampe");
        assert_eq!(d.command_topic(), "zigbee2mqtt/salon/lampe/set");
        assert_eq!(d.grafana_uid(), "dominet-device-7");
        d.base_topic = String::new();
        assert_eq!(d.state_topic(), "salon/lampe");
    }

    #[test]
    fn state_log_entries_follow_the_reference_rules() {
        let state = json!({"temperature": 21.5, "humidity": 40, "contact": true, "mode": "eco", "nested": {"a": 1}, "none": null, "off": false});
        let keys = tracked(&[
            "temperature",
            "humidity",
            "contact",
            "mode",
            "nested",
            "none",
            "off",
            "absent",
        ]);
        let e = log_entries(&keys, &state);
        let got: Vec<_> = e
            .iter()
            .map(|x| (x.key.as_str(), x.value_num, x.value_str.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("temperature", Some(21.5), ""),
                ("humidity", Some(40.0), ""),
                ("contact", Some(1.0), "True"),
                ("mode", None, "eco"),
                ("off", Some(0.0), "False"),
            ]
        );
        // Texte long tronque ; rien de suivi, rien d'ecrit.
        let long = log_entries(&tracked(&["s"]), &json!({"s": "é".repeat(300)}));
        assert_eq!(long[0].value_str.chars().count(), 128);
        assert!(log_entries(&[], &state).is_empty());
        assert!(log_entries(&tracked(&["a"]), &json!("pas un objet")).is_empty());
    }

    #[test]
    fn known_labels_cover_the_reference_table_and_default_for_the_rest() {
        let t = known_label("temperature");
        assert_eq!(
            (t.icon_name.as_str(), t.name.as_str(), t.unit.as_str()),
            ("thermometer", "Température", "°C")
        );
        let u = known_label("co2_ppm");
        assert_eq!(
            (u.icon_name.as_str(), u.name.as_str(), u.unit.as_str()),
            ("circle-dot", "co2_ppm", "")
        );
        assert_eq!(KNOWN_LABELS.len(), 25);
        assert!(
            KNOWN_LABELS.windows(2).all(|w| w[0].0 < w[1].0),
            "table triee et sans doublon"
        );
        assert!(KNOWN_LABELS.iter().all(|(k, ..)| is_state_key(k)));
    }

    #[test]
    fn device_drafts_report_problems_per_field() {
        assert!(DeviceDraft::new("Bureau").problems().is_empty());
        let mut d = DeviceDraft::new(" ");
        d.base_topic = "a/#".into();
        d.image_url = "ftp://x".into();
        d.tracked_keys = tracked(&["temperature", "temperature"]);
        d.hidden_keys = tracked(&["bad key"]);
        d.description = "d".repeat(257);
        let fields: Vec<_> = d.problems().into_iter().map(|(f, _)| f).collect();
        assert_eq!(
            fields,
            [
                "friendly_name",
                "description",
                "base_topic",
                "image_url",
                "tracked_keys",
                "hidden_keys"
            ]
        );
        assert!(
            DeviceDraft {
                friendly_name: "x".repeat(129),
                ..DeviceDraft::new("x")
            }
            .problems()
            .contains(&("friendly_name", Problem::TooLong { max: 128 }))
        );
    }

    #[test]
    fn sources_stations_groups_and_labels_validate() {
        assert!(MqttSource::draft("Linky", "linky/#").problems().is_empty());
        let mut s = MqttSource::draft("", "a/#/b");
        s.qos = 3;
        s.base_topic = "x#".into();
        let f: Vec<_> = s.problems().into_iter().map(|(f, _)| f).collect();
        assert_eq!(f, ["name", "topic_pattern", "qos", "base_topic"]);
        assert!(MqttSource::draft("S", "solarmax").problems().is_empty());

        assert!(
            WeatherStation::draft("Maison", 48.85, 2.35)
                .problems()
                .is_empty()
        );
        let mut w = WeatherStation::draft("", 91.0, -181.0);
        w.altitude = 10_000;
        w.mqtt_topic_suffix = "a b".into();
        w.refresh_minutes = 0;
        let f: Vec<_> = w.problems().into_iter().map(|(f, _)| f).collect();
        assert_eq!(
            f,
            [
                "name",
                "latitude",
                "longitude",
                "altitude",
                "mqtt_topic_suffix",
                "refresh_minutes"
            ]
        );
        assert_eq!(
            WeatherStation::draft("M", 0.0, 0.0).mqtt_topic(),
            "meteo/meteo_locale"
        );

        assert!(
            DeviceGroupDraft {
                name: "Salon".into(),
                location: None,
                description: String::new(),
                icon: String::new()
            }
            .problems()
            .is_empty()
        );
        assert_eq!(
            DeviceGroupDraft {
                name: String::new(),
                location: None,
                description: String::new(),
                icon: String::new()
            }
            .problems(),
            [("name", Problem::Required)]
        );
        assert!(known_label("battery").problems().is_empty());
        assert_eq!(
            DeviceLabelDraft {
                key: "a b".into(),
                name: String::new(),
                icon_name: String::new(),
                unit: String::new()
            }
            .problems()
            .len(),
            3
        );
    }
}
