// SPDX-License-Identifier: AGPL-3.0-or-later
//! Initial data: the known application settings and the default navigation.
//!
//! Settings registry and default navigation. Sensitive keys are secret: their value is encrypted at rest (ADR 0011).

use crate::navbar::{NavPosition, NavbarDraft};
use crate::setting::{SettingDraft, SettingKey, SettingType, is_sensitive_key};

/// A known setting: its shape and default value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingSpec {
    pub key: &'static str,
    pub value_type: SettingType,
    pub category: &'static str,
    pub description: &'static str,
    pub default: &'static str,
}

impl SettingSpec {
    /// Secret if the key designates an API key, a password or a token.
    #[must_use]
    pub fn is_secret(&self) -> bool {
        is_sensitive_key(self.key)
    }

    /// Write draft with the default value or the supplied value.
    ///
    /// # Panics
    /// Never for the entries of [`SETTINGS`] (keys checked by the tests).
    #[must_use]
    pub fn draft(&self, value: &str) -> SettingDraft {
        SettingDraft {
            key: SettingKey::parse(self.key).expect("valid registry key"),
            value: value.to_string(),
            value_type: self.value_type,
            category: self.category.to_string(),
            description: self.description.to_string(),
            is_secret: self.is_secret(),
        }
    }
}

const fn spec(
    key: &'static str,
    value_type: SettingType,
    category: &'static str,
    description: &'static str,
    default: &'static str,
) -> SettingSpec {
    SettingSpec {
        key,
        value_type,
        category,
        description,
        default,
    }
}

use SettingType::{Bool, Float, Int, Str};

/// The 17 application settings.
pub const SETTINGS: [SettingSpec; 17] = [
    spec(
        "EMAIL_RECIPIENT",
        Str,
        "Notifications",
        "Alert e-mail recipient",
        "dominet@localhost",
    ),
    spec(
        "DEFAULT_FROM_EMAIL",
        Str,
        "Notifications",
        "E-mail sender address",
        "dominet@localhost",
    ),
    spec(
        "SMS_FREE_USER",
        Str,
        "Notifications",
        "Free Mobile identifier (SMS)",
        "",
    ),
    spec(
        "SMS_FREE_KEY",
        Str,
        "Notifications",
        "Cle API Free Mobile (SMS)",
        "",
    ),
    spec(
        "PROMETHEUS_ALERT_COOLDOWN",
        Int,
        "Alerts",
        "Prometheus alert cooldown (s)",
        "3600",
    ),
    spec(
        "MOTION_ALERT_COOLDOWN",
        Int,
        "Alerts",
        "Motion detection cooldown per camera (s)",
        "600",
    ),
    spec(
        "MOTION_THRESHOLD",
        Float,
        "Surveillance",
        "Motion detection threshold (0.0-1.0, e.g. 0.01)",
        "0.01",
    ),
    spec(
        "RECORDINGS_SEGMENT_DURATION",
        Int,
        "Surveillance",
        "Video segment duration (s)",
        "60",
    ),
    spec(
        "GRAFANA_PUBLIC_URL",
        Str,
        "Interface",
        "Public Grafana URL (browser)",
        "http://localhost:3000",
    ),
    spec(
        "PROMETHEUS_PUBLIC_URL",
        Str,
        "Interface",
        "Public Prometheus URL (browser)",
        "http://localhost:9090",
    ),
    spec(
        "MEDIAMTX_WEBRTC_DOMAIN",
        Str,
        "Interface",
        "MediaMTX WebRTC domain",
        "localhost:8889",
    ),
    spec(
        "MEDIAMTX_WEBRTC_SCHEME",
        Str,
        "Interface",
        "Schema WebRTC (http ou https)",
        "http",
    ),
    spec(
        "Z2M_BASE_TOPIC",
        Str,
        "MQTT",
        "Topic de base Zigbee2MQTT",
        "zigbee2mqtt",
    ),
    spec(
        "AGENT_ENABLED",
        Bool,
        "Assistant",
        "Active l'assistant local (page Assistant)",
        "false",
    ),
    spec(
        "AGENT_LLM_URL",
        Str,
        "Assistant",
        "Model server address (llama-server), e.g. http://192.168.1.20:8081",
        "",
    ),
    spec(
        "AGENT_MODEL",
        Str,
        "Assistant",
        "Model for everyday requests (read, control)",
        "Qwen3-8B-Q4_K_M",
    ),
    spec(
        "AGENT_MODEL_ADMIN",
        Str,
        "Assistant",
        "Model for configuration requests (empty = same as everyday requests)",
        "",
    ),
];

/// The setting of that name, if it is known.
#[must_use]
pub fn setting_spec(key: &str) -> Option<&'static SettingSpec> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// Default navigation: top then bottom. The Zigbee2MQTT icon and label were swapped in the reference seed.
#[must_use]
pub fn default_navbar() -> Vec<NavbarDraft> {
    let item =
        |position, sort_order, icon: &str, label: &str, url: &str, target: &str, admin_only| {
            NavbarDraft {
                icon_name: icon.to_string(),
                label: label.to_string(),
                url: url.to_string(),
                url_target: target.to_string(),
                admin_only,
                position,
                sort_order,
            }
        };
    vec![
        item(
            NavPosition::Top,
            1,
            "video",
            "Surveillance",
            "/surveillance/",
            "_self",
            false,
        ),
        item(
            NavPosition::Top,
            2,
            "settings-2",
            "Automation",
            "/automation/",
            "_self",
            false,
        ),
        item(
            NavPosition::Top,
            10,
            "activity",
            "Observabilite",
            // The reference had an `/observability/` page; here these are the Grafana dashboards (the alerts have their own link).
            "/grafana/",
            "_self",
            false,
        ),
        item(
            NavPosition::Top,
            20,
            "book-open",
            "Documentation",
            "/docs/",
            "_self",
            false,
        ),
        item(
            NavPosition::Bottom,
            5,
            "wifi-sync",
            "Zigbee2MQTT",
            "http://dominet.local:8080",
            "_blank",
            true,
        ),
        item(
            NavPosition::Bottom,
            10,
            "smartphone",
            "Android",
            "/android/",
            "_self",
            true,
        ),
        item(
            NavPosition::Bottom,
            20,
            "mail",
            "Messages",
            "/messaging/",
            "_self",
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::setting::SettingValue;

    #[test]
    fn there_are_seventeen_unique_valid_settings() {
        assert_eq!(SETTINGS.len(), 17);
        let keys: HashSet<_> = SETTINGS.iter().map(|s| s.key).collect();
        assert_eq!(keys.len(), 17, "cles uniques");
        for s in &SETTINGS {
            assert!(SettingKey::parse(s.key).is_ok(), "{}", s.key);
            assert!(
                s.category.len() <= 32 && !s.category.is_empty(),
                "{}",
                s.key
            );
            assert!(!s.description.is_empty(), "{}", s.key);
        }
    }

    #[test]
    fn every_default_matches_its_declared_type_and_validates() {
        for s in &SETTINGS {
            if s.default.is_empty() {
                continue; // a setting not set (SMS identifiers)
            }
            assert!(
                SettingValue::parse(s.default, s.value_type).is_ok(),
                "{} = {:?}",
                s.key,
                s.default
            );
            assert!(s.draft(s.default).validate().is_ok(), "{}", s.key);
        }
        for s in SETTINGS.iter().filter(|s| s.default.is_empty()) {
            // An empty value is only valid for text.
            assert_eq!(s.value_type, SettingType::Str, "{}", s.key);
        }
    }

    #[test]
    fn only_the_sms_key_is_secret() {
        let secrets: Vec<_> = SETTINGS
            .iter()
            .filter(|s| s.is_secret())
            .map(|s| s.key)
            .collect();
        assert_eq!(secrets, ["SMS_FREE_KEY"]);
        assert!(setting_spec("SMS_FREE_KEY").unwrap().draft("x").is_secret);
    }

    #[test]
    fn lookup_by_key() {
        assert_eq!(setting_spec("MOTION_THRESHOLD").unwrap().default, "0.01");
        assert!(setting_spec("INCONNU").is_none());
    }

    #[test]
    fn the_default_navbar_is_valid_and_unique() {
        let items = default_navbar();
        assert_eq!(items.len(), 7);
        let mut seen = HashSet::new();
        for i in &items {
            assert!(i.validate().is_ok(), "{}: {:?}", i.label, i.validate());
            assert!(
                seen.insert((i.position.as_str(), i.label.clone())),
                "doublon {}",
                i.label
            );
        }
        let z2m = items.iter().find(|i| i.label == "Zigbee2MQTT").unwrap();
        assert_eq!(
            (
                z2m.icon_name.as_str(),
                z2m.url_target.as_str(),
                z2m.admin_only
            ),
            ("wifi-sync", "_blank", true)
        );
        assert!(
            items
                .iter()
                .filter(|i| i.position == NavPosition::Top)
                .all(|i| !i.admin_only)
        );
    }
}
