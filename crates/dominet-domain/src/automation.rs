// SPDX-License-Identifier: AGPL-3.0-or-later
//! Automations (R4, ADR 0031): a trigger, an optional condition, an action.
//!
//! **Strong** types in memory, the **reference implementation's** JSON in the database (`trigger_type` + `trigger_config`, `action_type` +
//! `action_config`, `condition`): the production automations load as they are. The plugins become `script`s
//! (Rhai, code in `script`). The validation rules are here, applied the same way by the pages, the CLI and the tests.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use crate::command::check_command;
use crate::cron::CronSpec;
use crate::device::{
    is_state_key, is_valid_friendly_name, is_valid_publish_topic, is_valid_topic_filter,
};
use crate::error::Problem;
use crate::scene::SceneId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AutomationId(pub i64);

/// Field-by-field problems (the names are those of the form fields).
pub type Problems = Vec<(&'static str, Problem)>;

/// The nine solar events of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolarEvent {
    Sunrise,
    Sunset,
    SolarNoon,
    DawnCivil,
    DuskCivil,
    DawnNautical,
    DuskNautical,
    DawnAstronomical,
    DuskAstronomical,
}

impl SolarEvent {
    pub const ALL: [Self; 9] = [
        Self::Sunrise,
        Self::Sunset,
        Self::SolarNoon,
        Self::DawnCivil,
        Self::DuskCivil,
        Self::DawnNautical,
        Self::DuskNautical,
        Self::DawnAstronomical,
        Self::DuskAstronomical,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sunrise => "sunrise",
            Self::Sunset => "sunset",
            Self::SolarNoon => "solar_noon",
            Self::DawnCivil => "dawn_civil",
            Self::DuskCivil => "dusk_civil",
            Self::DawnNautical => "dawn_nautical",
            Self::DuskNautical => "dusk_nautical",
            Self::DawnAstronomical => "dawn_astronomical",
            Self::DuskAstronomical => "dusk_astronomical",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == s)
    }
}

/// What triggers an automation.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// A message whose topic matches the filter (wildcards `+` and `#`) and whose payload contains `payload_filter`.
    MqttMessage {
        topic: String,
        payload_filter: Map<String, Value>,
    },
    /// A cron expression with five fields, in local time.
    Schedule { cron: String },
    /// A solar event at the given place.
    Solar {
        event: SolarEvent,
        latitude: f64,
        longitude: f64,
    },
    /// A device state message whose `state[key]` equals `value` (the meaning is that of ADR 0031, not the reference's).
    DeviceState {
        friendly_name: String,
        key: String,
        value: Value,
    },
}

fn text<'a>(cfg: &'a Value, key: &str) -> Option<&'a str> {
    cfg.get(key).and_then(Value::as_str)
}

fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

impl Trigger {
    pub const KINDS: [&'static str; 4] = ["mqtt_message", "schedule", "solar", "device_state"];

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MqttMessage { .. } => "mqtt_message",
            Self::Schedule { .. } => "schedule",
            Self::Solar { .. } => "solar",
            Self::DeviceState { .. } => "device_state",
        }
    }

    /// JSON configuration, in the reference's shape.
    #[must_use]
    pub fn config(&self) -> Value {
        match self {
            Self::MqttMessage {
                topic,
                payload_filter,
            } if payload_filter.is_empty() => json!({"topic": topic}),
            Self::MqttMessage {
                topic,
                payload_filter,
            } => json!({"topic": topic, "payload_filter": payload_filter}),
            Self::Schedule { cron } => json!({"cron": cron}),
            Self::Solar {
                event,
                latitude,
                longitude,
            } => {
                json!({"event": event.as_str(), "latitude": latitude, "longitude": longitude})
            }
            Self::DeviceState {
                friendly_name,
                key,
                value,
            } => {
                json!({"friendly_name": friendly_name, "key": key, "value": value})
            }
        }
    }

    /// Reads a trigger from `trigger_type` + `trigger_config`.
    ///
    /// # Errors
    /// Shape problems (absent field, wrong type) then value problems.
    pub fn from_parts(kind: &str, cfg: &Value) -> Result<Self, Problems> {
        let missing = |field: &'static str| vec![(field, Problem::Required)];
        let trigger = match kind {
            "mqtt_message" => {
                let topic = text(cfg, "topic")
                    .ok_or_else(|| missing("topic"))?
                    .to_string();
                let payload_filter = match cfg.get("payload_filter") {
                    None | Some(Value::Null) => Map::new(),
                    Some(Value::Object(o)) => o.clone(),
                    Some(_) => return Err(vec![("payload_filter", Problem::Invalid)]),
                };
                Self::MqttMessage {
                    topic,
                    payload_filter,
                }
            }
            "schedule" => Self::Schedule {
                cron: text(cfg, "cron")
                    .ok_or_else(|| missing("cron"))?
                    .to_string(),
            },
            "solar" => {
                let event = SolarEvent::parse(text(cfg, "event").ok_or_else(|| missing("event"))?)
                    .ok_or_else(|| vec![("event", Problem::Invalid)])?;
                let number = |field: &'static str| {
                    cfg.get(field)
                        .and_then(Value::as_f64)
                        .ok_or_else(|| missing(field))
                };
                Self::Solar {
                    event,
                    latitude: number("latitude")?,
                    longitude: number("longitude")?,
                }
            }
            "device_state" => Self::DeviceState {
                friendly_name: text(cfg, "friendly_name")
                    .ok_or_else(|| missing("friendly_name"))?
                    .to_string(),
                key: text(cfg, "key").ok_or_else(|| missing("key"))?.to_string(),
                value: cfg.get("value").cloned().ok_or_else(|| missing("value"))?,
            },
            _ => return Err(vec![("trigger_type", Problem::Invalid)]),
        };
        let problems = trigger.problems();
        if problems.is_empty() {
            Ok(trigger)
        } else {
            Err(problems)
        }
    }

    #[must_use]
    pub fn problems(&self) -> Problems {
        let mut out = vec![];
        match self {
            Self::MqttMessage {
                topic,
                payload_filter,
            } => {
                if topic.is_empty() {
                    out.push(("topic", Problem::Required));
                } else if !is_valid_topic_filter(topic) {
                    out.push(("topic", Problem::Invalid));
                }
                if payload_filter.len() > 20 {
                    out.push(("payload_filter", Problem::Invalid));
                }
            }
            Self::Schedule { cron } => {
                if cron.trim().is_empty() {
                    out.push(("cron", Problem::Required));
                } else if let Err(e) = CronSpec::parse(cron) {
                    out.push(("cron", e.problem()));
                }
            }
            Self::Solar {
                latitude,
                longitude,
                ..
            } => {
                if !(-90.0..=90.0).contains(latitude) {
                    out.push(("latitude", Problem::OutOfRange { min: -90, max: 90 }));
                }
                if !(-180.0..=180.0).contains(longitude) {
                    out.push((
                        "longitude",
                        Problem::OutOfRange {
                            min: -180,
                            max: 180,
                        },
                    ));
                }
            }
            Self::DeviceState {
                friendly_name,
                key,
                value,
            } => {
                if friendly_name.is_empty() {
                    out.push(("friendly_name", Problem::Required));
                } else if !is_valid_friendly_name(friendly_name) {
                    out.push(("friendly_name", Problem::Invalid));
                }
                if key.is_empty() {
                    out.push(("key", Problem::Required));
                } else if !is_state_key(key) {
                    out.push(("key", Problem::Invalid));
                }
                if !is_scalar(value) {
                    out.push(("value", Problem::Invalid));
                }
            }
        }
        out
    }
}

/// Optional condition: if `key` is in the payload, it must equal `value`; otherwise the condition is true (like the reference).
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub key: String,
    pub value: Value,
}

impl Condition {
    /// `{}` (or `null`) = no condition.
    ///
    /// # Errors
    /// Absent or invalid key.
    pub fn from_json(v: &Value) -> Result<Option<Self>, Problems> {
        match v {
            Value::Null => Ok(None),
            Value::Object(o) if o.is_empty() => Ok(None),
            Value::Object(o) => {
                let key = o.get("key").and_then(Value::as_str).unwrap_or_default();
                if key.is_empty() || !is_state_key(key) {
                    return Err(vec![("condition", Problem::Invalid)]);
                }
                Ok(Some(Self {
                    key: key.to_string(),
                    value: o.get("value").cloned().unwrap_or(Value::Null),
                }))
            }
            _ => Err(vec![("condition", Problem::Invalid)]),
        }
    }

    #[must_use]
    pub fn to_json(c: &Option<Self>) -> Value {
        c.as_ref()
            .map_or_else(|| json!({}), |c| json!({"key": c.key, "value": c.value}))
    }

    /// True if the payload does not contain the key, or if it equals `value` there (the reference's equality, see [`loose_equal`]). A payload
    /// that is not an object does not have the key: the condition is true.
    #[must_use]
    pub fn holds(&self, payload: &Value) -> bool {
        payload
            .get(&self.key)
            .is_none_or(|v| loose_equal(v, &self.value))
    }
}

/// Equality of two JSON values like `==` in the reference: a number equals the same number whatever its form (`1` = `1.0`),
/// and a boolean equals `0` / `1` (`True == 1`), including inside lists and objects.
#[must_use]
pub fn loose_equal(a: &Value, b: &Value) -> bool {
    fn number(v: &Value) -> Option<f64> {
        match v {
            Value::Number(n) => n.as_f64(),
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            _ => None,
        }
    }
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| loose_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| loose_equal(v, w)))
        }
        _ => match (number(a), number(b)) {
            #[allow(clippy::float_cmp)] // like the reference: exact comparison
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
}

/// Payload filter of an `mqtt_message` trigger: each pair of the filter must equal the payload's value. A key
/// absent from the payload is `null` (`payload.get(k) == v` in the reference: a `null` filter lets it through). A payload that is
/// not an object only passes the empty filter.
#[must_use]
pub fn payload_filter_holds(filter: &Map<String, Value>, payload: &Value) -> bool {
    let Some(object) = payload.as_object() else {
        return filter.is_empty();
    };
    filter
        .iter()
        .all(|(k, v)| loose_equal(object.get(k).unwrap_or(&Value::Null), v))
}

/// Text form of a scalar (`28` -> `28`, `true` -> `true`, `"28"` -> `28`).
fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Meaning of `device_state` (ADR 0031, D2): the state contains `key` and its value equals `value`, by the text form of scalars
/// (`28` = `"28"`) or numerically (`28.0` = `28`). Non-scalar values never match.
#[must_use]
pub fn device_state_holds(key: &str, value: &Value, state: &Value) -> bool {
    let Some(seen) = state.get(key) else {
        return false;
    };
    match (scalar_text(seen), scalar_text(value)) {
        (Some(a), Some(b)) => {
            a == b
                || matches!((seen, value), (Value::Number(_), Value::Number(_)) if loose_equal(seen, value))
        }
        _ => false,
    }
}

/// Value entered in a form: JSON if it is some (`28`, `true`, `"28"`, `[1]`), otherwise the text as is (`ON`). To
/// force a text that looks like JSON (`28`), surround it with quotes.
#[must_use]
pub fn value_from_text(text: &str) -> Value {
    let t = text.trim();
    serde_json::from_str::<Value>(t).unwrap_or_else(|_| Value::String(t.to_string()))
}

/// Inverse of [`value_from_text`]: a text is displayed as is, in quotes only if it would be read back differently.
#[must_use]
pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) if value_from_text(s) == *v && s.trim() == s => s.clone(),
        other => other.to_string(),
    }
}

/// What an automation does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    MqttPublish {
        topic: String,
        payload: Value,
    },
    /// The Rhai code is in [`AutomationDraft::script`].
    Script,
    /// Log and banner in the interface (sending by SMS / e-mail / Android comes with R5).
    Notification {
        message: String,
        sms: bool,
    },
    Scene {
        scene: SceneId,
    },
}

impl Action {
    pub const KINDS: [&'static str; 4] = ["mqtt_publish", "script", "notification", "scene"];

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MqttPublish { .. } => "mqtt_publish",
            Self::Script => "script",
            Self::Notification { .. } => "notification",
            Self::Scene { .. } => "scene",
        }
    }

    #[must_use]
    pub fn config(&self) -> Value {
        match self {
            Self::MqttPublish { topic, payload } => json!({"topic": topic, "payload": payload}),
            Self::Script => json!({}),
            Self::Notification { message, sms } => json!({"message": message, "sms": sms}),
            Self::Scene { scene } => json!({"scene_id": scene.0}),
        }
    }

    /// Reads an action from `action_type` + `action_config` (`script`: the code is in the automation's `script` field).
    ///
    /// # Errors
    /// Shape problems then value problems.
    pub fn from_parts(kind: &str, cfg: &Value) -> Result<Self, Problems> {
        let action = match kind {
            "mqtt_publish" => Self::MqttPublish {
                topic: text(cfg, "topic").unwrap_or_default().to_string(),
                payload: cfg.get("payload").cloned().unwrap_or_else(|| json!({})),
            },
            "script" => Self::Script,
            "notification" => Self::Notification {
                message: text(cfg, "message").unwrap_or_default().to_string(),
                sms: cfg.get("sms").and_then(Value::as_bool).unwrap_or(false),
            },
            "scene" => Self::Scene {
                scene: SceneId(cfg.get("scene_id").and_then(Value::as_i64).unwrap_or(0)),
            },
            _ => return Err(vec![("action_type", Problem::Invalid)]),
        };
        let problems = action.problems();
        if problems.is_empty() {
            Ok(action)
        } else {
            Err(problems)
        }
    }

    #[must_use]
    pub fn problems(&self) -> Problems {
        let mut out = vec![];
        match self {
            Self::MqttPublish { topic, payload } => {
                if topic.is_empty() {
                    out.push(("action_topic", Problem::Required));
                } else if !is_valid_publish_topic(topic) {
                    out.push(("action_topic", Problem::Invalid));
                }
                if check_command(payload).is_err() {
                    out.push(("action_payload", Problem::Invalid));
                }
            }
            Self::Script => {}
            Self::Notification { message, .. } => {
                if message.chars().count() > 500 {
                    out.push(("message", Problem::TooLong { max: 500 }));
                }
            }
            Self::Scene { scene } => {
                if scene.0 <= 0 {
                    out.push(("scene_id", Problem::Required));
                }
            }
        }
        out
    }
}

/// An automation to save.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationDraft {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    pub trigger: Trigger,
    pub condition: Option<Condition>,
    pub action: Action,
    /// Rhai code of the `script` action.
    pub script: String,
}

/// The five JSON / text columns of an automation in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct Parts {
    pub trigger_type: &'static str,
    pub trigger_config: Value,
    pub condition: Value,
    pub action_type: &'static str,
    pub action_config: Value,
}

impl AutomationDraft {
    pub const SCRIPT_MAX: usize = 20_000;

    #[must_use]
    pub fn new(name: &str, trigger: Trigger, action: Action) -> Self {
        Self {
            name: name.to_string(),
            enabled: true,
            description: String::new(),
            trigger,
            condition: None,
            action,
            script: String::new(),
        }
    }

    #[must_use]
    pub fn parts(&self) -> Parts {
        Parts {
            trigger_type: self.trigger.kind(),
            trigger_config: self.trigger.config(),
            condition: Condition::to_json(&self.condition),
            action_type: self.action.kind(),
            action_config: self.action.config(),
        }
    }

    /// Rebuilds a draft from the database columns or from a `context.yaml`.
    ///
    /// # Errors
    /// The problems of the trigger, the condition and the action, together.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        name: &str,
        enabled: bool,
        description: &str,
        trigger_type: &str,
        trigger_config: &Value,
        condition: &Value,
        action_type: &str,
        action_config: &Value,
        script: &str,
    ) -> Result<Self, Problems> {
        let trigger = Trigger::from_parts(trigger_type, trigger_config);
        let condition = Condition::from_json(condition);
        let action = Action::from_parts(action_type, action_config);
        let (Ok(trigger), Ok(condition), Ok(action)) = (&trigger, &condition, &action) else {
            let mut all = vec![];
            for p in [trigger.err(), condition.err(), action.err()]
                .into_iter()
                .flatten()
            {
                all.extend(p);
            }
            return Err(all);
        };
        let draft = Self {
            name: name.to_string(),
            enabled,
            description: description.to_string(),
            trigger: trigger.clone(),
            condition: condition.clone(),
            action: action.clone(),
            script: script.to_string(),
        };
        let problems = draft.problems();
        if problems.is_empty() {
            Ok(draft)
        } else {
            Err(problems)
        }
    }

    #[must_use]
    pub fn problems(&self) -> Problems {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 128 {
            out.push(("name", Problem::TooLong { max: 128 }));
        }
        if self.description.chars().count() > 256 {
            out.push(("description", Problem::TooLong { max: 256 }));
        }
        out.extend(self.trigger.problems());
        if let Some(c) = &self.condition
            && !is_state_key(&c.key)
        {
            out.push(("condition", Problem::Invalid));
        }
        out.extend(self.action.problems());
        if self.action == Action::Script && self.script.trim().is_empty() {
            out.push(("script", Problem::Required));
        }
        if self.script.chars().count() > Self::SCRIPT_MAX {
            out.push((
                "script",
                Problem::TooLong {
                    max: Self::SCRIPT_MAX,
                },
            ));
        }
        out
    }
}

/// A saved automation.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: AutomationId,
    pub draft: AutomationDraft,
    pub last_triggered: Option<DateTime<Utc>>,
    pub trigger_count: i32,
}

/// Outcome of a trigger, written to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Error,
    /// False condition or action with no effect.
    Skipped,
    /// Ignored by the anti-loop safeguard.
    Limited,
}

impl RunStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::Limited => "limited",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Ok, Self::Error, Self::Skipped, Self::Limited]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

/// A row of the execution log.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationRun {
    pub automation: AutomationId,
    pub at: DateTime<Utc>,
    pub trigger: String,
    pub status: RunStatus,
    pub duration_ms: i32,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value)] // the test cases are written in temporary `json!`
    fn parse(
        trigger_type: &str,
        tc: Value,
        action_type: &str,
        ac: Value,
    ) -> Result<AutomationDraft, Problems> {
        AutomationDraft::from_parts(
            "A",
            true,
            "",
            trigger_type,
            &tc,
            &json!({}),
            action_type,
            &ac,
            "",
        )
    }

    #[test]
    fn form_values_read_json_or_plain_text_and_round_trip() {
        for (text, expected) in [
            ("ON", json!("ON")),
            ("28", json!(28)),
            ("28.5", json!(28.5)),
            ("true", json!(true)),
            ("\"28\"", json!("28")),
            ("  ON  ", json!("ON")),
            ("[1, 2]", json!([1, 2])),
            ("3_single", json!("3_single")),
            ("", json!("")),
        ] {
            assert_eq!(value_from_text(text), expected, "{text:?}");
        }
        for v in [
            json!("ON"),
            json!(28),
            json!("28"),
            json!(true),
            json!("true"),
            json!(""),
            json!(" x "),
            json!([1]),
            json!("3_single"),
        ] {
            assert_eq!(value_from_text(&value_to_text(&v)), v, "{v}");
        }
        assert_eq!(value_to_text(&json!("28")), "\"28\"");
        assert_eq!(value_to_text(&json!("ON")), "ON");
    }

    #[test]
    fn equality_follows_reference() {
        assert!(loose_equal(&json!(1), &json!(1.0)));
        assert!(loose_equal(&json!(true), &json!(1)));
        assert!(loose_equal(&json!(false), &json!(0.0)));
        assert!(loose_equal(
            &json!([1, {"a": true}]),
            &json!([1.0, {"a": 1}])
        ));
        assert!(
            !loose_equal(&json!("1"), &json!(1)),
            "un texte n'est pas un nombre"
        );
        assert!(!loose_equal(&json!(null), &json!(0)));
        assert!(!loose_equal(&json!([1]), &json!([1, 2])));
        assert!(!loose_equal(&json!({"a": 1}), &json!({"b": 1})));
        assert!(
            !loose_equal(&json!(0.1 + 0.2), &json!(0.3)),
            "comparaison exacte, comme la reference"
        );
    }

    #[test]
    fn the_payload_filter_is_the_reference_all_get_equal() {
        let f = |v: Value| v.as_object().unwrap().clone();
        let p = json!({"action": "single", "n": 2});
        assert!(payload_filter_holds(&Map::new(), &p));
        assert!(payload_filter_holds(&f(json!({"action": "single"})), &p));
        assert!(payload_filter_holds(&f(json!({"n": 2.0})), &p));
        assert!(!payload_filter_holds(&f(json!({"action": "double"})), &p));
        assert!(!payload_filter_holds(&f(json!({"absent": 1})), &p));
        // Reference: an absent key is None, which equals a `null` filter.
        assert!(payload_filter_holds(&f(json!({"absent": null})), &p));
        // A text payload only passes the empty filter.
        assert!(payload_filter_holds(&Map::new(), &json!("online")));
        assert!(!payload_filter_holds(&f(json!({"a": 1})), &json!("online")));
    }

    #[test]
    fn device_state_compares_scalars_by_their_text_form() {
        let st =
            json!({"action": "3_single", "n": 28, "f": 28.5, "on": true, "list": [1], "nul": null});
        for (key, value, expected) in [
            ("action", json!("3_single"), true),
            ("action", json!("1_single"), false),
            ("n", json!(28), true),
            ("n", json!("28"), true),
            ("n", json!(28.0), true),
            ("n", json!(29), false),
            ("f", json!(28.5), true),
            ("f", json!("28.5"), true),
            ("on", json!(true), true),
            ("on", json!("true"), true),
            ("on", json!(1), false),
            ("list", json!("[1]"), false),
            ("nul", json!("null"), false),
            ("absent", json!("x"), false),
        ] {
            assert_eq!(
                device_state_holds(key, &value, &st),
                expected,
                "{key} {value}"
            );
        }
    }

    #[test]
    fn the_production_shapes_round_trip_to_the_same_json() {
        let cases = [
            (
                "device_state",
                json!({"key": "action", "value": "3_single", "friendly_name": "Tele-commande"}),
                "mqtt_publish",
                json!({"topic": "zigbee2mqtt/Volet-salon/set", "payload": {"state": "CLOSE"}}),
            ),
            (
                "solar",
                json!({"event": "sunset", "latitude": 43.3, "longitude": 5.4}),
                "scene",
                json!({"scene_id": 1}),
            ),
            (
                "schedule",
                json!({"cron": "5 0 * * *"}),
                "notification",
                json!({"message": "x", "sms": true}),
            ),
            (
                "mqtt_message",
                json!({"topic": "linky/+/papp", "payload_filter": {"a": 1}}),
                "mqtt_publish",
                json!({"topic": "a/b", "payload": {"x": 1}}),
            ),
        ];
        for (tt, tc, at, ac) in cases {
            let d = parse(tt, tc.clone(), at, ac.clone()).unwrap();
            let p = d.parts();
            assert_eq!((p.trigger_type, &p.trigger_config), (tt, &tc));
            assert_eq!((p.action_type, &p.action_config), (at, &ac));
        }
    }

    #[test]
    fn a_script_action_needs_code() {
        let err = parse(
            "device_state",
            json!({"key": "action", "value": "x", "friendly_name": "V"}),
            "script",
            json!({}),
        )
        .unwrap_err();
        assert_eq!(err, [("script", Problem::Required)]);
        let ok = AutomationDraft::from_parts(
            "A",
            true,
            "",
            "device_state",
            &json!({"key": "action", "value": "x", "friendly_name": "V"}),
            &json!({}),
            "script",
            &json!({}),
            "// a porter",
        )
        .unwrap();
        assert_eq!(ok.action, Action::Script);
        assert_eq!(ok.parts().action_type, "script");
    }

    #[test]
    fn invalid_triggers_are_reported_field_by_field() {
        let p = |tt, tc: Value| Trigger::from_parts(tt, &tc).unwrap_err();
        assert_eq!(
            p("mqtt_message", json!({"topic": "a/#/b"})),
            [("topic", Problem::Invalid)]
        );
        assert_eq!(p("mqtt_message", json!({})), [("topic", Problem::Required)]);
        assert_eq!(
            p("mqtt_message", json!({"topic": "a", "payload_filter": [1]})),
            [("payload_filter", Problem::Invalid)]
        );
        assert_eq!(
            p("schedule", json!({"cron": "61 * * * *"})),
            [("cron", Problem::Invalid)]
        );
        assert_eq!(
            p("schedule", json!({"cron": "  "})),
            [("cron", Problem::Required)]
        );
        assert_eq!(
            p(
                "solar",
                json!({"event": "midi", "latitude": 1, "longitude": 1})
            ),
            [("event", Problem::Invalid)]
        );
        assert_eq!(
            p(
                "solar",
                json!({"event": "sunset", "latitude": 91.0, "longitude": 5.4})
            ),
            [("latitude", Problem::OutOfRange { min: -90, max: 90 })]
        );
        assert_eq!(
            p("solar", json!({"event": "sunset", "latitude": 1})),
            [("longitude", Problem::Required)]
        );
        assert_eq!(
            p(
                "device_state",
                json!({"friendly_name": "A", "key": "k w", "value": 1})
            ),
            [("key", Problem::Invalid)]
        );
        assert_eq!(
            p(
                "device_state",
                json!({"friendly_name": "A", "key": "k", "value": [1]})
            ),
            [("value", Problem::Invalid)]
        );
        assert_eq!(
            p("inconnu", json!({})),
            [("trigger_type", Problem::Invalid)]
        );
    }

    #[test]
    fn invalid_actions_and_names_are_reported() {
        let a = |at, ac: Value| Action::from_parts(at, &ac).unwrap_err();
        assert_eq!(
            a(
                "mqtt_publish",
                json!({"topic": "a/+/b", "payload": {"state": "ON"}})
            ),
            [("action_topic", Problem::Invalid)]
        );
        assert_eq!(
            a("mqtt_publish", json!({"topic": "a/b", "payload": []})),
            [("action_payload", Problem::Invalid)]
        );
        assert_eq!(
            a("mqtt_publish", json!({})),
            [
                ("action_topic", Problem::Required),
                ("action_payload", Problem::Invalid)
            ]
        );
        assert_eq!(a("scene", json!({})), [("scene_id", Problem::Required)]);
        assert_eq!(
            a("notification", json!({"message": "x".repeat(501)})),
            [("message", Problem::TooLong { max: 500 })]
        );
        let mut d = parse(
            "schedule",
            json!({"cron": "* * * * *"}),
            "scene",
            json!({"scene_id": 1}),
        )
        .unwrap();
        d.name = " ".into();
        d.description = "x".repeat(257);
        assert_eq!(
            d.problems(),
            [
                ("name", Problem::Required),
                ("description", Problem::TooLong { max: 256 })
            ]
        );
    }

    #[test]
    fn the_condition_follows_reference_even_when_the_key_is_absent() {
        let c = Condition::from_json(&json!({"key": "state", "value": "ON"}))
            .unwrap()
            .unwrap();
        assert!(c.holds(&json!({"state": "ON"})));
        assert!(!c.holds(&json!({"state": "OFF"})));
        assert!(
            c.holds(&json!({"autre": 1})),
            "cle absente : la condition est vraie, comme en reference"
        );
        assert!(Condition::from_json(&json!({})).unwrap().is_none());
        assert!(Condition::from_json(&json!({"value": 1})).is_err());
        assert!(Condition::from_json(&json!("x")).is_err());
    }

    #[test]
    fn the_nine_solar_events_round_trip() {
        for e in SolarEvent::ALL {
            assert_eq!(SolarEvent::parse(e.as_str()), Some(e));
        }
        assert_eq!(SolarEvent::parse("midi"), None);
    }
}
