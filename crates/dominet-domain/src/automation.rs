// SPDX-License-Identifier: AGPL-3.0-or-later
//! Automations (R4, ADR 0031) : un declencheur, une condition facultative, une action.
//!
//! Types **forts** en memoire, JSON **de l'implementation de reference** en base (`trigger_type` + `trigger_config`, `action_type` +
//! `action_config`, `condition`) : les automations de la production se chargent telles quelles. les plugins deviennent des `script`
//! (Rhai, code dans `script`). Les regles de validation sont ici, appliquees pareil par les pages, la CLI et les tests.

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

/// Problemes champ par champ (les noms sont ceux des champs du formulaire).
pub type Problems = Vec<(&'static str, Problem)>;

/// Les neuf evenements solaires de reference (les evenements solaires de reference).
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

/// Ce qui declenche une automation.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Un message dont le topic correspond au filtre (jokers `+` et `#`) et dont le payload contient `payload_filter`.
    MqttMessage {
        topic: String,
        payload_filter: Map<String, Value>,
    },
    /// Une expression cron a cinq champs, a l'heure locale.
    Schedule { cron: String },
    /// Un evenement solaire au lieu donne.
    Solar {
        event: SolarEvent,
        latitude: f64,
        longitude: f64,
    },
    /// Un message d'etat de l'appareil dont `etat[key]` vaut `value` (le sens est celui de l'ADR 0031, pas celui de la reference).
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

    /// Configuration JSON, de la forme de la reference.
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

    /// Lit un declencheur depuis `trigger_type` + `trigger_config`.
    ///
    /// # Errors
    /// Les problemes de forme (champ absent, mauvais type) puis de valeur.
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

/// Condition facultative : si `key` est dans le payload, il doit valoir `value` ; sinon la condition est vraie (comme la reference).
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub key: String,
    pub value: Value,
}

impl Condition {
    /// `{}` (ou `null`) = aucune condition.
    ///
    /// # Errors
    /// Cle absente ou invalide.
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

    /// Vrai si le payload ne contient pas la cle, ou si elle y vaut `value` (egalite de la reference, voir [`loose_equal`]). Un payload
    /// qui n'est pas un objet n'a pas la cle : la condition est vraie.
    #[must_use]
    pub fn holds(&self, payload: &Value) -> bool {
        payload
            .get(&self.key)
            .is_none_or(|v| loose_equal(v, &self.value))
    }
}

/// Egalite de deux valeurs JSON comme `==` en Reference : un nombre vaut le meme nombre quelle que soit sa forme (`1` = `1.0`),
/// et un booleen vaut `0` / `1` (`True == 1`), y compris dans les listes et objets.
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
            #[allow(clippy::float_cmp)] // comme la reference : comparaison exacte
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
}

/// Filtre de payload d'un declencheur `mqtt_message` : chaque couple du filtre doit etre egal a la valeur du payload. Une cle
/// absente du payload vaut `null` (`payload.get(k) == v` en Reference : un filtre `null` la laisse passer). Un payload qui n'est
/// pas un objet ne passe que le filtre vide.
#[must_use]
pub fn payload_filter_holds(filter: &Map<String, Value>, payload: &Value) -> bool {
    let Some(object) = payload.as_object() else {
        return filter.is_empty();
    };
    filter
        .iter()
        .all(|(k, v)| loose_equal(object.get(k).unwrap_or(&Value::Null), v))
}

/// Forme texte d'un scalaire (`28` -> `28`, `true` -> `true`, `"28"` -> `28`).
fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Sens de `device_state` (ADR 0031, D2) : l'etat contient `key` et sa valeur egale `value`, par la forme texte des scalaires
/// (`28` = `"28"`) ou numeriquement (`28.0` = `28`). Les valeurs non scalaires ne correspondent jamais.
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

/// Valeur saisie dans un formulaire : du JSON s'il en est (`28`, `true`, `"28"`, `[1]`), sinon le texte tel quel (`ON`). Pour
/// forcer un texte qui ressemble a du JSON (`28`), l'entourer de guillemets.
#[must_use]
pub fn value_from_text(text: &str) -> Value {
    let t = text.trim();
    serde_json::from_str::<Value>(t).unwrap_or_else(|_| Value::String(t.to_string()))
}

/// Inverse de [`value_from_text`] : un texte s'affiche tel quel, entre guillemets seulement s'il serait relu autrement.
#[must_use]
pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) if value_from_text(s) == *v && s.trim() == s => s.clone(),
        other => other.to_string(),
    }
}

/// Ce que fait une automation.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    MqttPublish {
        topic: String,
        payload: Value,
    },
    /// Le code Rhai est dans [`AutomationDraft::script`].
    Script,
    /// Journal et bandeau dans l'interface (l'envoi par SMS / e-mail / Android arrive avec R5).
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

    /// Lit une action depuis `action_type` + `action_config` (`script` : le code est dans le champ `script` de l'automation).
    ///
    /// # Errors
    /// Problemes de forme puis de valeur.
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

/// Une automation a enregistrer.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationDraft {
    pub name: String,
    pub enabled: bool,
    pub description: String,
    pub trigger: Trigger,
    pub condition: Option<Condition>,
    pub action: Action,
    /// Code Rhai de l'action `script`.
    pub script: String,
}

/// Les cinq colonnes JSON / texte d'une automation en base.
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

    /// Reconstruit un brouillon depuis les colonnes de la base ou depuis un `context.yaml`.
    ///
    /// # Errors
    /// Les problemes du declencheur, de la condition et de l'action, ensemble.
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

/// Une automation enregistree.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: AutomationId,
    pub draft: AutomationDraft,
    pub last_triggered: Option<DateTime<Utc>>,
    pub trigger_count: i32,
}

/// Issue d'un declenchement, inscrite au journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Error,
    /// Condition fausse ou action sans effet.
    Skipped,
    /// Ignore par le garde-fou anti-boucle.
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

/// Une ligne du journal d'execution.
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

    #[allow(clippy::needless_pass_by_value)] // les cas de test sont ecrits en `json!` temporaires
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
        // Reference : une cle absente vaut None, qui egale un filtre `null`.
        assert!(payload_filter_holds(&f(json!({"absent": null})), &p));
        // Un payload texte ne passe que le filtre vide.
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
