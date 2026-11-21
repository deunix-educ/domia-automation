// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instantane `context.yaml` de l'installation (R3 lot 6, ADR 0030) : aide-memoire pour ecrire automatisations, alertes, scenes et
//! requetes Grafana sans relire le code. Le schema d'etat d'un appareil est deduit des **valeurs
//! courantes** de son etat. Les tests comparent a l'execution de l'implementation de reference.
//!
//! Le YAML est ecrit par un petit emetteur (le projet n'embarque pas de bibliotheque YAML) : mappings et listes en style bloc,
//! **toutes les chaines entre guillemets** (la syntaxe JSON est du YAML valide), cles nues seulement quand aucun lecteur YAML 1.1
//! ne peut les confondre avec un booleen ou un nombre.

use serde_json::Value;

/// Type d'une valeur, avec les noms de la reference.
#[must_use]
pub fn infer_type(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Object(_) => "dict",
        Value::Array(_) => "list",
        Value::Null => "null",
    }
}

/// `{cle: type}` d'un etat, dans l'ordre de l'etat (celui de la base, comme en reference) : sans `update` ni `linkquality`, sans objets ni listes ; une valeur nulle est « `float|null` ».
#[must_use]
pub fn state_schema(state: &Value) -> Vec<(String, &'static str)> {
    let Some(object) = state.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "update" | "linkquality"))
        .filter_map(|(k, v)| match infer_type(v) {
            "dict" | "list" => None,
            "null" => Some((k.clone(), "float|null")),
            t => Some((k.clone(), t)),
        })
        .collect()
}

/// Cles numeriques d'un schema.
#[must_use]
pub fn numeric_keys(schema: &[(String, &'static str)]) -> Vec<String> {
    schema
        .iter()
        .filter(|(_, t)| matches!(*t, "int" | "float" | "float|null"))
        .map(|(k, _)| k.clone())
        .collect()
}

fn plain_key(key: &str) -> bool {
    let mut chars = key.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "y" | "n" | "yes" | "no" | "on" | "off" | "true" | "false" | "null" | "~"
        )
}

fn key_text(key: &str) -> String {
    if plain_key(key) {
        key.to_string()
    } else {
        Value::String(key.to_string()).to_string()
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn put(out: &mut String, args: std::fmt::Arguments<'_>) {
    // Ecrire dans une `String` ne peut pas echouer.
    let _ = std::fmt::Write::write_fmt(out, args);
}

fn emit(value: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                match v {
                    Value::Object(m) if !m.is_empty() => {
                        put(out, format_args!("{pad}{}:\n", key_text(k)));
                        emit(v, indent + 2, out);
                    }
                    Value::Array(a) if !a.is_empty() => {
                        put(out, format_args!("{pad}{}:\n", key_text(k)));
                        emit(v, indent + 2, out);
                    }
                    Value::Object(_) => put(out, format_args!("{pad}{}: {{}}\n", key_text(k))),
                    Value::Array(_) => put(out, format_args!("{pad}{}: []\n", key_text(k))),
                    other => put(
                        out,
                        format_args!("{pad}{}: {}\n", key_text(k), scalar(other)),
                    ),
                }
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for item in items {
                match item {
                    Value::Object(m) if !m.is_empty() => {
                        // Premier couple sur la ligne du tiret, le reste aligne dessous.
                        let mut inner = String::new();
                        emit(item, indent + 2, &mut inner);
                        put(out, format_args!("{pad}- {}", inner.trim_start()));
                    }
                    Value::Array(a) if !a.is_empty() => {
                        let mut inner = String::new();
                        emit(item, indent + 2, &mut inner);
                        put(out, format_args!("{pad}- {}", inner.trim_start()));
                    }
                    Value::Object(_) => put(out, format_args!("{pad}- {{}}\n")),
                    Value::Array(_) => put(out, format_args!("{pad}- []\n")),
                    other => put(out, format_args!("{pad}- {}\n", scalar(other))),
                }
            }
        }
        Value::Object(_) => put(out, format_args!("{pad}{{}}\n")),
        Value::Array(_) => put(out, format_args!("{pad}[]\n")),
        other => put(out, format_args!("{pad}{}\n", scalar(other))),
    }
}

/// Document YAML d'une valeur JSON (ordre des cles = ordre de l'objet).
#[must_use]
pub fn to_yaml(value: &Value) -> String {
    let mut out = String::new();
    emit(value, 0, &mut out);
    out
}

/// Chaine de reference des requetes Grafana sur l'etat courant (la table s'appelle `device`).
pub const GRAFANA_SQL_PATTERN: &str = "SELECT (state->>'<key>')::float AS <key> FROM device WHERE friendly_name = '<friendly_name>' AND (state->>'<key>')::float <op> <seuil>";

/// Schemas de reference des declencheurs et actions (les memes que l'implementation de reference).
#[must_use]
pub fn reference_schemas() -> Value {
    serde_json::json!({
        "trigger_configs": {
            "device_state": {
                "friendly_name": "str — friendly_name du device source",
                "key": "str — key in device.state",
                "value": "str — value that triggers",
            },
            "schedule": {"cron": "str — 5 fields: minute hour day_of_month month day_of_week"},
            "solar": {
                "event": "sunrise|sunset|dawn_civil|dusk_civil|solar_noon|dawn_nautical|dusk_nautical|dawn_astronomical|dusk_astronomical",
                "latitude": "float",
                "longitude": "float",
            },
            "mqtt_message": {
                "topic": "str — MQTT pattern (wildcards + and # allowed)",
                "payload_filter": "dict — optional, filter on the payload",
            },
        },
        "action_configs": {
            "scene": {"scene_id": "int — primary key of the scene"},
            "mqtt_publish": {
                "topic": "str — target MQTT topic",
                "payload": "dict — ex: {state: ON} ou {position: 50}",
            },
            "notification": {"message": "str", "sms": "bool — optional, default false"},
            "script": {"_note": "Rhai script — code in the automation's `script` field"},
        },
        "cover_payloads": {
            "open": {"state": "OPEN"},
            "close": {"state": "CLOSE"},
            "stop": {"state": "STOP"},
            "position": {"position": "int(0-100)"},
        },
        "switch_payloads": {"on": {"state": "ON"}, "off": {"state": "OFF"}},
        "alert_operators": ["gt", "gte", "lt", "lte", "eq", "neq", "contains"],
        "grafana_sql_pattern": GRAFANA_SQL_PATTERN,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_state_schema_follows_the_reference_inference_in_the_same_order() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/context/schemas.json"))
                .unwrap();
        for case in fixtures.as_array().unwrap() {
            let schema = state_schema(&case["state"]);
            let want: Vec<(&str, &str)> = case["schema"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
                .collect();
            let got: Vec<(&str, &str)> = schema.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            assert_eq!(got, want, "etat {}", case["state"]);
            let numeric: Vec<&str> = case["numeric_keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap())
                .collect();
            assert_eq!(
                numeric_keys(&schema),
                numeric,
                "cles numeriques de {}",
                case["state"]
            );
        }
    }

    #[test]
    fn yaml_blocks_lists_empty_collections_and_quoting() {
        let doc = json!({
            "devices": {
                "Sonde salon": {"type": "sensor", "state_schema": {"temperature": "float"}, "numeric_keys": ["temperature", "humidity"]},
                "on": {"type": "light", "state_schema": {}},
            },
            "scenes": [],
            "rules": [{"name": "a: b", "notify": {"email": true}}, {"name": "c"}],
            "note": "ligne \"citée\"\net suite",
            "nothing": null,
            "count": 3,
        });
        let yaml = to_yaml(&doc);
        assert_eq!(
            yaml,
            "devices:\n  \"Sonde salon\":\n    type: \"sensor\"\n    state_schema:\n      temperature: \"float\"\n    numeric_keys:\n      - \"temperature\"\n      - \"humidity\"\n  \"on\":\n    type: \"light\"\n    state_schema: {}\nscenes: []\nrules:\n  - name: \"a: b\"\n    notify:\n      email: true\n  - name: \"c\"\nnote: \"ligne \\\"citée\\\"\\net suite\"\nnothing: null\ncount: 3\n"
        );
    }

    #[test]
    fn reserved_words_and_odd_keys_are_quoted() {
        for key in [
            "on", "Off", "yes", "null", "true", "1abc", "a b", "a/b", "é", "",
        ] {
            assert!(key_text(key).starts_with('"'), "{key}");
        }
        for key in ["temperature", "_x", "state-schema", "A1"] {
            assert_eq!(key_text(key), key);
        }
    }

    #[test]
    fn the_reference_schemas_are_plain_data() {
        let yaml = to_yaml(&reference_schemas());
        assert!(
            yaml.contains("trigger_configs:") && yaml.contains("grafana_sql_pattern: \"SELECT")
        );
        assert!(
            !yaml.contains("automation_device"),
            "la table s'appelle device"
        );
    }
}
