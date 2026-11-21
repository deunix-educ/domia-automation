// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decodage d'une charge utile MQTT, comme l'implementation de reference : UTF-8 (a defaut Latin-1), puis JSON (a defaut
//! le texte tel quel).

use serde_json::Value;

/// Charge utile decodee.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    Json(Value),
    Text(String),
}

impl Payload {
    /// L'objet JSON, si la charge en est un (les etats d'appareils sont toujours des objets).
    #[must_use]
    pub fn as_object(&self) -> Option<&serde_json::Map<String, Value>> {
        match self {
            Self::Json(Value::Object(o)) => Some(o),
            _ => None,
        }
    }

    /// Le texte, qu'il vienne d'une chaine JSON (`"online"`) ou d'un texte brut (`online`).
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Json(Value::String(s)) | Self::Text(s) => Some(s),
            Self::Json(_) => None,
        }
    }
}

/// Decode des octets : UTF-8, sinon Latin-1 (chaque octet est un caractere) ; JSON si possible, sinon texte.
#[must_use]
pub fn decode_payload(raw: &[u8]) -> Payload {
    let text = match std::str::from_utf8(raw) {
        Ok(t) => t.to_string(),
        Err(_) => raw.iter().map(|&b| char::from(b)).collect(),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Payload::Json(v),
        Err(_) => Payload::Text(text),
    }
}

/// Un message recu du broker.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub topic: String,
    pub payload: Payload,
    pub retain: bool,
    /// QoS de livraison (0, 1 ou 2).
    pub qos: u8,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_objects_arrays_numbers_and_strings_are_json() {
        assert_eq!(
            decode_payload(br#"{"temperature": 21.5}"#),
            Payload::Json(json!({"temperature": 21.5}))
        );
        assert_eq!(decode_payload(b"[1,2]"), Payload::Json(json!([1, 2])));
        assert_eq!(decode_payload(b"42"), Payload::Json(json!(42)));
        assert_eq!(decode_payload(b"true"), Payload::Json(json!(true)));
        assert_eq!(
            decode_payload(br#""online""#),
            Payload::Json(json!("online"))
        );
    }

    #[test]
    fn other_text_stays_text() {
        assert_eq!(decode_payload(b"online"), Payload::Text("online".into()));
        assert_eq!(decode_payload(b""), Payload::Text(String::new()));
        assert_eq!(
            decode_payload(b"{pas du json"),
            Payload::Text("{pas du json".into())
        );
    }

    #[test]
    fn invalid_utf8_falls_back_to_latin_1() {
        // « é » en Latin-1 (un seul octet 0xE9) n'est pas de l'UTF-8 valide.
        assert_eq!(
            decode_payload(&[b'c', b'a', b'f', 0xE9]),
            Payload::Text("café".into())
        );
        assert_eq!(
            decode_payload(&[b'"', 0xE9, b'"']),
            Payload::Json(json!("é"))
        );
    }

    #[test]
    fn accessors() {
        let obj = decode_payload(br#"{"state":"ON"}"#);
        assert_eq!(obj.as_object().unwrap()["state"], "ON");
        assert_eq!(decode_payload(b"online").as_text(), Some("online"));
        assert_eq!(decode_payload(br#""online""#).as_text(), Some("online"));
        assert_eq!(decode_payload(b"12").as_text(), None);
        assert!(decode_payload(b"[1]").as_object().is_none());
    }
}
