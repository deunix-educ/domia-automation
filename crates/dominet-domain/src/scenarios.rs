// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test scenarios drawn from the `context.yaml` of the **reference production** (R4): real devices, scenes, automations and alert rule,
//! used as test data. No import into a production database: the JSON file (`tests/fixtures/automation/
//! prod-context.json`) comes from `tools/context-to-json.sh`.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::alert::{AlertRuleDraft, Operator, Severity};
use crate::automation::{AutomationDraft, Problems};
use crate::device::{DeviceId, DeviceType};
use crate::scene::SceneId;

/// The production's JSON, for the tests of the other crates (`scenarios` feature).
#[cfg(any(test, feature = "scenarios"))]
pub const PROD_CONTEXT_JSON: &str =
    include_str!("../../../tests/fixtures/automation/prod-context.json");

/// The production's two plugins, ported to Rhai (R4 lot 5) according to their name: the original reference implementation is not in the
/// `context.yaml`.
pub const PROD_SCRIPT_LEFT: &str = include_str!("../data/volet-sb-est-gauche.rhai");
pub const PROD_SCRIPT_RIGHT: &str = include_str!("../data/volet-sb-est-droite.rhai");

#[derive(Debug, Clone, PartialEq)]
pub struct ProdDevice {
    pub name: String,
    pub device_type: DeviceType,
    /// Deduced from the command topic (`<base>/<name>/set`).
    pub base_topic: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProdScene {
    /// Number in the production database (the automations refer to it).
    pub pk: i64,
    pub name: String,
    pub icon: String,
    /// (device name, command), in order.
    pub actions: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProdAutomation {
    pub name: String,
    pub trigger_type: String,
    pub trigger_config: Value,
    pub action_type: String,
    pub action_config: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProdAlertRule {
    pub name: String,
    pub device: String,
    pub key: String,
    pub operator: Operator,
    pub threshold_num: Option<f64>,
    pub threshold_str: String,
    pub severity: Severity,
    pub cooldown_minutes: i16,
    pub notify: (bool, bool, bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProdContext {
    pub devices: Vec<ProdDevice>,
    pub scenes: Vec<ProdScene>,
    pub automations: Vec<ProdAutomation>,
    pub alert_rules: Vec<ProdAlertRule>,
}

fn s(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("field `{key}` missing"))
}

impl ProdContext {
    /// # Errors
    /// Text of the first anomaly of the file.
    pub fn parse(json: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let devices = root["devices"]
            .as_object()
            .ok_or("devices absent")?
            .iter()
            .map(|(name, d)| {
                let command = d["command_topic"].as_str().unwrap_or_default();
                let base_topic = command
                    .strip_suffix(&format!("/{name}/set"))
                    .unwrap_or_default()
                    .to_string();
                Ok(ProdDevice {
                    name: name.clone(),
                    device_type: d["type"]
                        .as_str()
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(DeviceType::Generic),
                    base_topic,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let scenes = root["scenes"]
            .as_array()
            .ok_or("scenes absent")?
            .iter()
            .map(|sc| {
                let actions = sc["actions"]
                    .as_array()
                    .ok_or("actions absent")?
                    .iter()
                    .map(|a| Ok((s(a, "device")?, a["payload"].clone())))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(ProdScene {
                    pk: sc["pk"].as_i64().ok_or("pk absent")?,
                    name: s(sc, "name")?,
                    icon: sc["icon"].as_str().unwrap_or_default().to_string(),
                    actions,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let automations = root["automations"]
            .as_array()
            .ok_or("automations absent")?
            .iter()
            .map(|a| {
                Ok(ProdAutomation {
                    name: s(a, "name")?,
                    trigger_type: s(a, "trigger_type")?,
                    trigger_config: a["trigger_config"].clone(),
                    action_type: s(a, "action_type")?,
                    action_config: a["action_config"].clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let alert_rules = root["alert_rules"]
            .as_array()
            .ok_or("alert_rules absent")?
            .iter()
            .map(|r| {
                Ok(ProdAlertRule {
                    name: s(r, "name")?,
                    device: s(r, "device")?,
                    key: s(r, "key")?,
                    operator: Operator::parse(&s(r, "operator")?).ok_or("unknown operator")?,
                    threshold_num: r["threshold_num"].as_f64(),
                    threshold_str: r["threshold_str"].as_str().unwrap_or_default().to_string(),
                    severity: Severity::parse(&s(r, "severity")?).ok_or("unknown severity")?,
                    cooldown_minutes: i16::try_from(r["cooldown_minutes"].as_i64().unwrap_or(10))
                        .map_err(|e| e.to_string())?,
                    notify: (
                        r["notify"]["email"].as_bool().unwrap_or(false),
                        r["notify"]["sms"].as_bool().unwrap_or(false),
                        r["notify"]["android"].as_bool().unwrap_or(false),
                    ),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            devices,
            scenes,
            automations,
            alert_rules,
        })
    }
}

impl ProdAutomation {
    /// Automation draft. `scene_of` translates the production's scene number into the test database's.
    /// The `script` actions: the production's two receive their Rhai port.
    ///
    /// # Errors
    /// The domain's validation problems.
    pub fn draft(&self, scene_of: &BTreeMap<i64, SceneId>) -> Result<AutomationDraft, Problems> {
        let mut action_config = self.action_config.clone();
        if self.action_type == "scene"
            && let Some(pk) = action_config.get("scene_id").and_then(Value::as_i64)
            && let Some(local) = scene_of.get(&pk)
        {
            action_config["scene_id"] = Value::from(local.0);
        }
        AutomationDraft::from_parts(
            &self.name,
            true,
            "",
            &self.trigger_type,
            &self.trigger_config,
            &Value::Null,
            &self.action_type,
            &action_config,
            match (self.action_type.as_str(), self.name.contains("gauche")) {
                ("script", true) => PROD_SCRIPT_LEFT,
                ("script", false) => PROD_SCRIPT_RIGHT,
                _ => "",
            },
        )
    }
}

impl ProdAlertRule {
    #[must_use]
    pub fn draft(&self, device: DeviceId) -> AlertRuleDraft {
        let mut d = AlertRuleDraft::new(&self.name, device, &self.key);
        d.operator = self.operator;
        d.threshold_num = self.threshold_num;
        d.threshold_str.clone_from(&self.threshold_str);
        d.severity = self.severity;
        d.cooldown_minutes = self.cooldown_minutes;
        (d.notify_email, d.notify_sms, d.notify_android) = self.notify;
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_production_context_loads_and_every_automation_is_valid() {
        let c = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
        assert_eq!(
            (c.scenes.len(), c.automations.len(), c.alert_rules.len()),
            (3, 7, 1)
        );
        assert_eq!(
            c.scenes.iter().map(|s| s.actions.len()).collect::<Vec<_>>(),
            [6, 4, 4]
        );
        let by = |n: &str| c.devices.iter().find(|d| d.name == n).unwrap();
        assert_eq!(by("Volet-salon").device_type, DeviceType::Cover);
        assert_eq!(by("Volet-salon").base_topic, "zigbee2mqtt");
        assert!(
            c.devices
                .iter()
                .any(|d| d.base_topic.starts_with("solarmax/"))
        );
        let scenes: BTreeMap<i64, SceneId> = c
            .scenes
            .iter()
            .map(|s| (s.pk, SceneId(s.pk + 100)))
            .collect();
        let drafts: Vec<AutomationDraft> = c
            .automations
            .iter()
            .map(|a| a.draft(&scenes).unwrap())
            .collect();
        let kinds: Vec<(&str, &str)> = drafts
            .iter()
            .map(|d| (d.trigger.kind(), d.action.kind()))
            .collect();
        assert_eq!(kinds.iter().filter(|k| k.0 == "device_state").count(), 5);
        assert_eq!(
            kinds.iter().filter(|k| **k == ("solar", "scene")).count(),
            2
        );
        assert_eq!(
            kinds.iter().filter(|k| k.1 == "script").count(),
            2,
            "les deux plugins deviennent des scripts"
        );
        // The solar scenes target the scenes of the test database.
        let solar: Vec<i64> = drafts
            .iter()
            .filter_map(|d| match d.action {
                crate::automation::Action::Scene { scene } => Some(scene.0),
                _ => None,
            })
            .collect();
        assert_eq!(solar, [101, 102]);
        let rule = c.alert_rules[0].draft(DeviceId(1));
        assert_eq!(rule.problems(), []);
        assert!(rule.evaluate(&Value::from(36.0)) && !rule.evaluate(&Value::from(35.0)));
        assert_eq!(
            (
                rule.cooldown_minutes,
                rule.notify_email,
                rule.notify_sms,
                rule.notify_android
            ),
            (60, true, false, true)
        );
    }
}
