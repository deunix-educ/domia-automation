// SPDX-License-Identifier: AGPL-3.0-or-later
//! Protocole TCP des onduleurs SolarMax (port 12345), partie pure : construction des requetes, lecture des reponses, grandeurs.
//! Issu de `solarmax.py` de Bernd Wurst (GPL-3.0-or-later, "Developed 2009-2010 by Bernd Wurst", publie en 2012, python-solarmax) ; les tests rejouent des vecteurs de reference figes
//! (`tests/fixtures/solarmax/vectors.json`, sans acces reseau).

use std::collections::BTreeMap;

/// Grandeurs que l'on peut demander a l'onduleur.
pub const QUERY_TYPES: [&str; 20] = [
    "KDY", "KYR", "KMT", "KT0", "IL1", "IDC", "PAC", "PRL", "SYS", "SAL", "TNF", "TKK", "UL1",
    "UDC", "ADR", "TYP", "PIN", "MAC", "CAC", "KHR",
];

/// Type de requete du protocole (`100` en decimal = `64` en hexadecimal).
const QUERY_KIND: u32 = 100;

/// Modeles connus : (code `TYP`, nom, puissance maximale en W).
pub const INVERTER_TYPES: [(i64, &str, i64); 4] = [
    (20010, "SolarMax 2000S", 2000),
    (20020, "SolarMax 3000S", 3000),
    (20030, "SolarMax 4200S", 4200),
    (20040, "SolarMax 6000S", 6000),
];

/// Etats de fonctionnement (premier nombre de `SYS`).
#[must_use]
pub fn status_text(code: i64) -> &'static str {
    match code {
        20000 => "No communication",
        20001 => "In use",
        20002 => "Too little irradiation",
        20003 => "Starting",
        20004 => "Operating at MPP",
        20005 => "The fan is running",
        20006 => "Operating at maximum power",
        20007 => "Temperature limit",
        20008 => "Operating on grid",
        _ => "Unknown",
    }
}

/// Alarmes (bits de `SAL`).
pub const ALARM_CODES: [(i64, &str); 11] = [
    (1, "External error 1"),
    (2, "DC-side insulation error"),
    (4, "Ground fault current too high"),
    (8, "Earth protection fuse blown"),
    (16, "External alarm 2"),
    (32, "Long-term temperature limitation"),
    (64, "AC supply error"),
    (128, "External alarm 4"),
    (256, "Faulty fan"),
    (512, "Fuse blown"),
    (1024, "Temperature sensor failure"),
];

/// Pourquoi une requete ou une reponse est refusee.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("grandeur inconnue : {0}")]
    UnknownType(String),
    #[error("reponse malformee")]
    Malformed,
    #[error("erreur de checksum")]
    Checksum,
    #[error("longueur incoherente")]
    Length,
}

/// Valeur lue, deja mise a l'echelle.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Float(f64),
    Int(i64),
    /// `SYS` : (etat, sous-etat).
    Pair(i64, i64),
}

impl Value {
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => i32::try_from(*i).ok().map(f64::from),
            Self::Pair(..) => None,
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }
}

/// Somme des caracteres, en hexadecimal majuscule sur 4 chiffres.
#[must_use]
pub fn checksum(s: &str) -> String {
    let total: u32 = s.chars().map(u32::from).sum();
    format!("{total:04X}")
}

/// Requete pour l'onduleur `inverter` : `{FB;NN;LL|64:K1;K2|CCCC}`.
///
/// # Errors
/// [`ProtocolError::UnknownType`] pour une grandeur hors de [`QUERY_TYPES`].
pub fn build_query(inverter: u8, keys: &[&str]) -> Result<String, ProtocolError> {
    if let Some(bad) = keys.iter().find(|k| !QUERY_TYPES.contains(k)) {
        return Err(ProtocolError::UnknownType((*bad).to_string()));
    }
    let payload = format!("|{QUERY_KIND:X}:{}|", keys.join(";"));
    let length = payload.len() + 2 + 2 + 2 + 2 + 2 + 4;
    let mut message = format!("FB;{inverter:02};{length:X}{payload}");
    message.push_str(&checksum(&message));
    Ok(format!("{{{message}}}"))
}

/// Nombre hexadecimal de 32 bits au plus (les valeurs de l'onduleur tiennent sur 16 bits).
fn hex32(value: &str) -> Result<u32, ProtocolError> {
    u32::from_str_radix(value, 16).map_err(|_| ProtocolError::Malformed)
}

fn hex(value: &str) -> Result<i64, ProtocolError> {
    hex32(value).map(i64::from)
}

/// Mise a l'echelle d'une valeur lue (les memes regles que la reference).
fn normalize(key: &str, value: &str) -> Result<Value, ProtocolError> {
    Ok(match key {
        "KDY" | "UL1" | "UDC" => Value::Float(f64::from(hex32(value)?) / 10.0),
        "IL1" | "IDC" | "TNF" => Value::Float(f64::from(hex32(value)?) / 100.0),
        "PAC" | "PIN" => Value::Float(f64::from(hex32(value)?) / 2.0),
        "SYS" => {
            let (a, b) = value.split_once(',').ok_or(ProtocolError::Malformed)?;
            Value::Pair(hex(a)?, hex(b)?)
        }
        _ => Value::Int(hex(value)?),
    })
}

/// Lit une reponse `{NN;FB;LL|64:K=V;K=V|CCCC}` : numero d'onduleur et valeurs.
///
/// # Errors
/// [`ProtocolError`] si la reponse est mal formee, de mauvaise longueur ou de mauvais checksum, ou porte une grandeur inconnue.
pub fn parse_answer(answer: &str) -> Result<(u8, BTreeMap<String, Value>), ProtocolError> {
    let inner = answer
        .strip_prefix('{')
        .and_then(|a| a.strip_suffix('}'))
        .ok_or(ProtocolError::Malformed)?;
    if inner.len() < 5 || !inner.is_ascii() {
        return Err(ProtocolError::Malformed);
    }
    let (content, sum) = inner.split_at(inner.len() - 4);
    if sum != checksum(content) {
        return Err(ProtocolError::Checksum);
    }
    let content = content.strip_suffix('|').ok_or(ProtocolError::Malformed)?;
    let mut parts = content.splitn(2, '|');
    let (header, body) = (
        parts.next().ok_or(ProtocolError::Malformed)?,
        parts.next().ok_or(ProtocolError::Malformed)?,
    );
    let mut fields = header.split(';');
    let (inverter, fb, length) = (
        fields.next().ok_or(ProtocolError::Malformed)?,
        fields.next().ok_or(ProtocolError::Malformed)?,
        fields.next().ok_or(ProtocolError::Malformed)?,
    );
    if fb != "FB" || fields.next().is_some() {
        return Err(ProtocolError::Malformed);
    }
    if usize::try_from(hex(length)?).ok() != Some(answer.len()) {
        return Err(ProtocolError::Length);
    }
    let inverter: u8 = inverter.parse().map_err(|_| ProtocolError::Malformed)?;
    let body = body.get(3..).ok_or(ProtocolError::Malformed)?; // « 64: »
    let mut data = BTreeMap::new();
    for item in body.split(';') {
        let (key, value) = item.split_once('=').ok_or(ProtocolError::Malformed)?;
        if !QUERY_TYPES.contains(&key) {
            return Err(ProtocolError::UnknownType(key.to_string()));
        }
        data.insert(key.to_string(), normalize(key, value)?);
    }
    Ok((inverter, data))
}

/// Reponse d'onduleur, telle que le simulateur de test la fabrique (valeurs deja en hexadecimal brut).
#[must_use]
pub fn build_answer(inverter: u8, items: &[(&str, &str)]) -> String {
    let content = items
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";");
    let mut length = 0usize;
    for _ in 0..3 {
        // La longueur est elle-meme dans le message : on la cherche par approximations successives.
        length = format!("{inverter:02};FB;{length:X}|64:{content}|").len() + 4 + 2;
    }
    let body = format!("{inverter:02};FB;{length:X}|64:{content}|");
    format!("{{{body}{}}}", checksum(&body))
}

/// Les grandeurs publiees (`solarmax/<ieee>/prod`), calculees comme la reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Production {
    pub tmpr: i64,
    pub pac: f64,
    pub eac: i64,
    pub pdc: f64,
    pub edc: i64,
    pub qdy: f64,
    pub qt0: i64,
    pub status: String,
}

/// Rapport en pour cent, tronque vers zero comme `int()` de la reference (les rapports d'un onduleur restent tres loin des bornes de `i64`).
#[allow(clippy::cast_possible_truncation)]
fn percent(ratio: f64) -> i64 {
    (ratio * 100.0) as i64
}

/// Arrondi a une decimale comme `round(x, 1)` de la reference (demi-pair sur la valeur binaire exacte).
#[must_use]
pub fn round1(x: f64) -> f64 {
    format!("{x:.1}").parse().unwrap_or(x)
}

impl Production {
    /// `current` : valeurs lues (`UL1`, `IL1`, `UDC`, `IDC`, `TKK`, `KDY`, `KT0`) ; `installed` : puissance installee (`PIN`).
    ///
    /// # Errors
    /// [`ProtocolError::Malformed`] si une grandeur manque.
    pub fn from_values(
        current: &BTreeMap<String, Value>,
        installed: f64,
        status: &str,
    ) -> Result<Self, ProtocolError> {
        let f = |k: &str| {
            current
                .get(k)
                .and_then(Value::as_f64)
                .ok_or(ProtocolError::Malformed)
        };
        let (uac, iac, udc, idc) = (f("UL1")?, f("IL1")?, f("UDC")?, f("IDC")?);
        let (pac, pdc) = (uac * iac, udc * idc);
        // `int()` de la reference tronque vers zero ; `as i64` aussi.
        Ok(Self {
            tmpr: current
                .get("TKK")
                .and_then(Value::as_i64)
                .ok_or(ProtocolError::Malformed)?,
            pac: round1(pac),
            eac: if installed == 0.0 {
                0
            } else {
                percent(pac / installed)
            },
            pdc: round1(pdc),
            edc: if pdc == 0.0 { 0 } else { percent(pac / pdc) },
            qdy: f("KDY")?,
            qt0: current
                .get("KT0")
                .and_then(Value::as_i64)
                .ok_or(ProtocolError::Malformed)?,
            status: status.to_string(),
        })
    }

    /// Charge utile JSON (memes cles que reference).
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "tmpr": self.tmpr, "pac": self.pac, "eac": self.eac, "pdc": self.pdc, "edc": self.edc,
            "qdy": self.qdy, "qt0": self.qt0, "status": self.status,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value as Json;

    use super::*;

    fn vectors() -> Json {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/solarmax/vectors.json"
        ))
        .unwrap()
    }

    #[test]
    fn requests_are_byte_identical_to_the_reference_ones() {
        for q in vectors()["queries"].as_array().unwrap() {
            let keys: Vec<&str> = q["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap())
                .collect();
            let inverter = u8::try_from(q["inverter"].as_u64().unwrap()).unwrap();
            assert_eq!(
                build_query(inverter, &keys).unwrap(),
                q["query"].as_str().unwrap(),
                "{keys:?}"
            );
        }
    }

    #[test]
    fn answers_are_read_and_scaled_like_reference() {
        for a in vectors()["answers"].as_array().unwrap() {
            let (inverter, data) = parse_answer(a["raw"].as_str().unwrap()).unwrap();
            assert_eq!(u64::from(inverter), a["inverter"].as_u64().unwrap());
            let expected = a["data"].as_object().unwrap();
            assert_eq!(data.len(), expected.len());
            for (key, want) in expected {
                match (&data[key], want) {
                    (Value::Pair(x, y), Json::Array(p)) => {
                        assert_eq!(
                            (*x, *y),
                            (p[0].as_i64().unwrap(), p[1].as_i64().unwrap()),
                            "{key}"
                        );
                    }
                    (got, want) => {
                        assert_eq!(got.as_f64().unwrap(), want.as_f64().unwrap(), "{key}");
                    }
                }
            }
        }
    }

    #[test]
    fn malformed_answers_are_refused_like_reference_refuses_them() {
        for b in vectors()["bad_answers"].as_array().unwrap() {
            let error = parse_answer(b["raw"].as_str().unwrap()).unwrap_err();
            let label = b["label"].as_str().unwrap();
            assert_eq!(
                matches!(error, ProtocolError::UnknownType(_)),
                b["expected_error"] == "NotImplementedError",
                "{label}: {error:?}"
            );
        }
        assert_eq!(parse_answer("").unwrap_err(), ProtocolError::Malformed);
        assert_eq!(parse_answer("{}").unwrap_err(), ProtocolError::Malformed);
        assert_eq!(parse_answer("{é}").unwrap_err(), ProtocolError::Malformed);
    }

    #[test]
    fn unknown_types_are_not_queried() {
        assert_eq!(
            build_query(1, &["PAC", "XXX"]),
            Err(ProtocolError::UnknownType("XXX".into()))
        );
    }

    #[test]
    fn the_answers_we_fabricate_are_the_answers_we_read() {
        let raw = build_answer(1, &[("PAC", "0640"), ("SYS", "4E24,0")]);
        let (inverter, data) = parse_answer(&raw).unwrap();
        assert_eq!(inverter, 1);
        assert_eq!(data["PAC"], Value::Float(800.0));
        assert_eq!(data["SYS"], Value::Pair(0x4E24, 0));
        // Octet pour octet la meme reponse que celle que fabrique le code de l'oracle reference.
        let python = vectors()["answers"][2]["raw"].as_str().unwrap().to_string();
        assert_eq!(build_answer(1, &[("SYS", "4E24,0"), ("SAL", "0")]), python);
    }

    #[test]
    fn derived_values_follow_the_reference_publisher_including_its_rounding() {
        for case in vectors()["production"].as_array().unwrap() {
            let f = |k: &str| case["current"][k].as_f64().unwrap();
            let current: BTreeMap<String, Value> = BTreeMap::from([
                ("UL1".into(), Value::Float(f("UL1"))),
                ("IL1".into(), Value::Float(f("IL1"))),
                ("UDC".into(), Value::Float(f("UDC"))),
                ("IDC".into(), Value::Float(f("IDC"))),
                (
                    "TKK".into(),
                    Value::Int(case["current"]["TKK"].as_i64().unwrap()),
                ),
                ("KDY".into(), Value::Float(f("KDY"))),
                (
                    "KT0".into(),
                    Value::Int(case["current"]["KT0"].as_i64().unwrap()),
                ),
            ]);
            let got = Production::from_values(&current, case["ivmax"].as_f64().unwrap(), "ok")
                .unwrap()
                .to_json();
            let want = &case["expected"];
            for key in ["tmpr", "pac", "eac", "pdc", "edc", "qdy", "qt0"] {
                assert_eq!(got[key].as_f64(), want[key].as_f64(), "{key} pour {case}");
            }
        }
    }

    #[test]
    fn rounding_ties_go_to_the_even_digit_like_reference() {
        assert_eq!(round1(115.25), 115.2);
        assert_eq!(
            round1(0.35),
            0.3,
            "0.35 est un peu en dessous de 0.35 en binaire"
        );
        assert_eq!(round1(2.675), 2.7);
    }
}
