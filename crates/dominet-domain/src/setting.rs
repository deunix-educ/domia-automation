// SPDX-License-Identifier: AGPL-3.0-or-later
//! Typed application settings.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::error::ValidationError;

/// Fragments that designate a sensitive key (sensitive settings).
const SENSITIVE_MARKERS: [&str; 4] = ["KEY", "PASSWORD", "TOKEN", "SECRET"];

/// Text displayed in place of a sensitive value.
pub const MASK: &str = "********";

/// True if the key designates a secret (API key, password, token...).
#[must_use]
pub fn is_sensitive_key(key: &str) -> bool {
    let up = key.to_ascii_uppercase();
    SENSITIVE_MARKERS.iter().any(|m| up.contains(m))
}

/// Setting key: 1 to 64 characters, upper case, digits and `_`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SettingKey(String);

impl SettingKey {
    /// # Errors
    /// Refuses an empty key, one that is too long or outside `[A-Z0-9_]`.
    pub fn parse(raw: &str) -> Result<Self, ValidationError> {
        if raw.is_empty()
            || raw.len() > 64
            || !raw
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(ValidationError::new(
                "invalid setting key (1 to 64 characters, A-Z, 0-9 and _)",
            ));
        }
        Ok(Self(raw.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingType {
    Str,
    Int,
    Float,
    Bool,
}

impl SettingType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
        }
    }
}

impl FromStr for SettingType {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "str" => Ok(Self::Str),
            "int" => Ok(Self::Int),
            "float" => Ok(Self::Float),
            "bool" => Ok(Self::Bool),
            other => Err(ValidationError::new(format!(
                "unknown setting type: \"{other}\""
            ))),
        }
    }
}

/// Typed value of a setting.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl SettingValue {
    /// Interprets a text according to the type. Accepted booleans: 1/true/yes/oui and 0/false/no/non.
    ///
    /// # Errors
    /// Refuses a text incompatible with the type (the message says which one is expected).
    pub fn parse(raw: &str, ty: SettingType) -> Result<Self, ValidationError> {
        let raw = raw.trim();
        match ty {
            SettingType::Str => Ok(Self::Str(raw.to_string())),
            SettingType::Int => raw
                .parse()
                .map(Self::Int)
                .map_err(|_| ValidationError::new(format!("integer expected, got \"{raw}\""))),
            SettingType::Float => match raw.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok(Self::Float(v)),
                _ => Err(ValidationError::new(format!(
                    "decimal number expected with a dot (e.g. 0.01), got \"{raw}\""
                ))),
            },
            SettingType::Bool => match raw.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "oui" => Ok(Self::Bool(true)),
                "0" | "false" | "no" | "non" => Ok(Self::Bool(false)),
                _ => Err(ValidationError::new(format!(
                    "boolean expected (true/false), got \"{raw}\""
                ))),
            },
        }
    }

    /// Text form stored in the database.
    #[must_use]
    pub fn to_raw(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Bool(b) => b.to_string(),
        }
    }
}

/// Setting as stored. If `is_secret`, `value` is a cipher text (ADR 0011), never a clear value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSetting {
    pub key: String,
    pub value: String,
    pub value_type: SettingType,
    pub category: String,
    pub description: String,
    pub is_secret: bool,
    pub updated_at: DateTime<Utc>,
}

impl AppSetting {
    /// Value to display: masked for a non-empty secret.
    #[must_use]
    pub fn display_value(&self) -> &str {
        if self.is_secret && !self.value.is_empty() {
            MASK
        } else {
            &self.value
        }
    }

    /// Typed value (invalid for an encrypted secret: it must be decrypted first).
    ///
    /// # Errors
    /// Returns the interpretation error if the value does not match the type.
    pub fn typed(&self) -> Result<SettingValue, ValidationError> {
        SettingValue::parse(&self.value, self.value_type)
    }
}

/// Setting to create or write; `value` is here the **clear** value (the repository or the
/// encryption layer transforms it if `is_secret`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingDraft {
    pub key: SettingKey,
    pub value: String,
    pub value_type: SettingType,
    pub category: String,
    pub description: String,
    pub is_secret: bool,
}

impl SettingDraft {
    /// Checks that the value matches the type and that sensitive keys are indeed marked secret.
    ///
    /// # Errors
    /// Returns the first rule violated.
    pub fn validate(&self) -> Result<(), ValidationError> {
        SettingValue::parse(&self.value, self.value_type)?;
        if is_sensitive_key(self.key.as_str()) && !self.is_secret {
            return Err(ValidationError::new(format!(
                "{} is a sensitive key: it must be marked secret",
                self.key
            )));
        }
        if self.category.trim().is_empty() || self.category.len() > 32 {
            return Err(ValidationError::new("category: 1 to 32 characters"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_keys_are_detected_like_reference() {
        for k in [
            "SMS_FREE_KEY",
            "EMAIL_HOST_PASSWORD",
            "REDUCTSTORE_TOKEN",
            "WG_SECRET",
            "api_key",
        ] {
            assert!(is_sensitive_key(k), "{k}");
        }
        for k in [
            "EMAIL_RECIPIENT",
            "MOTION_THRESHOLD",
            "Z2M_BASE_TOPIC",
            "GRAFANA_PUBLIC_URL",
        ] {
            assert!(!is_sensitive_key(k), "{k}");
        }
    }

    #[test]
    fn keys_follow_the_reference_naming() {
        assert!(SettingKey::parse("MOTION_ALERT_COOLDOWN").is_ok());
        for bad in ["", "motion", "A B", "A-B", &"A".repeat(65)] {
            assert!(SettingKey::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn values_are_typed_strictly() {
        assert_eq!(
            SettingValue::parse(" 42 ", SettingType::Int).unwrap(),
            SettingValue::Int(42)
        );
        assert_eq!(
            SettingValue::parse("0.01", SettingType::Float).unwrap(),
            SettingValue::Float(0.01)
        );
        assert_eq!(
            SettingValue::parse("Oui", SettingType::Bool).unwrap(),
            SettingValue::Bool(true)
        );
        assert_eq!(
            SettingValue::parse("non", SettingType::Bool).unwrap(),
            SettingValue::Bool(false)
        );
        for (raw, ty) in [
            ("abc", SettingType::Int),
            ("1.5", SettingType::Int),
            ("0,01", SettingType::Float),
            ("NaN", SettingType::Float),
            ("peut-etre", SettingType::Bool),
        ] {
            assert!(SettingValue::parse(raw, ty).is_err(), "{raw} comme {ty:?}");
        }
    }

    #[test]
    fn raw_form_round_trips() {
        for (raw, ty) in [
            ("12", SettingType::Int),
            ("0.5", SettingType::Float),
            ("true", SettingType::Bool),
            ("texte", SettingType::Str),
        ] {
            assert_eq!(SettingValue::parse(raw, ty).unwrap().to_raw(), raw);
        }
    }

    fn setting(value: &str, secret: bool) -> AppSetting {
        AppSetting {
            key: "SMS_FREE_KEY".into(),
            value: value.into(),
            value_type: SettingType::Str,
            category: "Notifications".into(),
            description: String::new(),
            is_secret: secret,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn secrets_are_masked_for_display_but_empty_stays_empty() {
        assert_eq!(setting("v1:chiffre", true).display_value(), MASK);
        assert_eq!(setting("", true).display_value(), "");
        assert_eq!(setting("visible", false).display_value(), "visible");
    }

    #[test]
    fn draft_validation_enforces_secret_flag_and_type() {
        let mut d = SettingDraft {
            key: SettingKey::parse("SMS_FREE_KEY").unwrap(),
            value: "abc".into(),
            value_type: SettingType::Str,
            category: "Notifications".into(),
            description: String::new(),
            is_secret: false,
        };
        assert!(d.validate().is_err(), "une cle sensible doit etre secrete");
        d.is_secret = true;
        assert!(d.validate().is_ok());
        d.key = SettingKey::parse("MOTION_THRESHOLD").unwrap();
        d.is_secret = false;
        d.value_type = SettingType::Float;
        assert!(d.validate().is_err(), "abc n'est pas un decimal");
        d.value = "0.01".into();
        assert!(d.validate().is_ok());
        d.category = String::new();
        assert!(d.validate().is_err());
    }
}
