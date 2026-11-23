// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alert rules and alerts (R4, ADR 0031). The evaluation of a value is the reference implementation's,
//! quirks included; the trigger / cooldown / resolution cycle is that of the alert cycle (lot 6).

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::device::{DeviceId, is_state_key};
use crate::error::Problem;
use crate::user::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlertRuleId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlertId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
    Contains,
}

impl Operator {
    pub const ALL: [Self; 7] = [
        Self::Gt,
        Self::Gte,
        Self::Lt,
        Self::Lte,
        Self::Eq,
        Self::Neq,
        Self::Contains,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::Lt => "lt",
            Self::Lte => "lte",
            Self::Eq => "eq",
            Self::Neq => "neq",
            Self::Contains => "contains",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub const ALL: [Self; 3] = [Self::Info, Self::Warning, Self::Critical];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// Reference label (`get_severity_display`), reused in the messages sent.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertStatus {
    Firing,
    Resolved,
    Acknowledged,
}

impl AlertStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
            Self::Acknowledged => "acknowledged",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Firing, Self::Resolved, Self::Acknowledged]
            .into_iter()
            .find(|o| o.as_str() == s)
    }
}

// Mirror of a table row: the notification channels and the activation are independent.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct AlertRuleDraft {
    pub name: String,
    pub device: DeviceId,
    pub key: String,
    pub operator: Operator,
    pub threshold_num: Option<f64>,
    pub threshold_str: String,
    pub severity: Severity,
    pub cooldown_minutes: i16,
    pub notify_email: bool,
    pub notify_sms: bool,
    pub notify_android: bool,
    pub enabled: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertRule {
    pub id: AlertRuleId,
    pub rule: AlertRuleDraft,
}

impl AlertRuleDraft {
    #[must_use]
    pub fn new(name: &str, device: DeviceId, key: &str) -> Self {
        Self {
            name: name.to_string(),
            device,
            key: key.to_string(),
            operator: Operator::Gt,
            threshold_num: None,
            threshold_str: String::new(),
            severity: Severity::Warning,
            cooldown_minutes: 10,
            notify_email: true,
            notify_sms: false,
            notify_android: true,
            enabled: true,
            description: String::new(),
        }
    }

    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        if self.name.trim().is_empty() {
            out.push(("name", Problem::Required));
        } else if self.name.chars().count() > 128 {
            out.push(("name", Problem::TooLong { max: 128 }));
        }
        if self.key.is_empty() {
            out.push(("key", Problem::Required));
        } else if !is_state_key(&self.key) {
            out.push(("key", Problem::Invalid));
        }
        if self.threshold_str.chars().count() > 128 {
            out.push(("threshold_str", Problem::TooLong { max: 128 }));
        }
        if self.threshold_num.is_some_and(|t| !t.is_finite()) {
            out.push(("threshold_num", Problem::Invalid));
        } else if self.operator == Operator::Contains {
            if self.threshold_str.trim().is_empty() {
                out.push(("threshold_str", Problem::Required));
            }
        } else if self.threshold_num.is_none() && self.threshold_str.trim().is_empty() {
            out.push(("threshold_num", Problem::Required));
        }
        if !(0..=1440).contains(&self.cooldown_minutes) {
            out.push((
                "cooldown_minutes",
                Problem::OutOfRange { min: 0, max: 1440 },
            ));
        }
        if self.description.chars().count() > 256 {
            out.push(("description", Problem::TooLong { max: 256 }));
        }
        out
    }

    /// True if `value` satisfies the rule (the reference's):
    /// - `contains`: the text threshold (lower case) is contained in the value rendered as text (lower case);
    /// - otherwise the value and the threshold are read as numbers (text threshold if there is no numeric threshold); if one of them is not one,
    ///   fall back on text equality (`eq` / `neq` only, spaces and case ignored), any other operator is false.
    #[must_use]
    pub fn evaluate(&self, value: &Value) -> bool {
        let text = plain_str(value);
        if self.operator == Operator::Contains {
            return text
                .to_lowercase()
                .contains(&self.threshold_str.to_lowercase());
        }
        let number = float_value(value);
        let threshold = self
            .threshold_num
            .or_else(|| float_from_text(&self.threshold_str));
        let (Some(v), Some(t)) = (number, threshold) else {
            let (sv, st) = (
                text.trim().to_lowercase(),
                self.threshold_str.trim().to_lowercase(),
            );
            return match self.operator {
                Operator::Eq => sv == st,
                Operator::Neq => sv != st,
                _ => false,
            };
        };
        match self.operator {
            Operator::Gt => v > t,
            Operator::Gte => v >= t,
            Operator::Lt => v < t,
            Operator::Lte => v <= t,
            Operator::Eq => (v - t).abs() == 0.0,
            Operator::Neq => (v - t).abs() != 0.0,
            Operator::Contains => false,
        }
    }
}

/// The reference's `str(value)` for a JSON value (booleans `True` / `False`, `None`).
#[must_use]
pub fn plain_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// The reference's `float("text")`: spaces ignored, sign, `.5`, `5.`, `1e3`, `nan`, `inf`, and underscores between two digits
/// (`1_000`), which `f64::from_str` refuses.
fn float_from_text(text: &str) -> Option<f64> {
    let t = text.trim();
    if !t.contains('_') {
        return t.parse::<f64>().ok();
    }
    let bytes = t.as_bytes();
    let between_digits = bytes.iter().enumerate().all(|(i, b)| {
        *b != b'_'
            || (i > 0
                && bytes.get(i + 1).is_some()
                && bytes[i - 1].is_ascii_digit()
                && bytes[i + 1].is_ascii_digit())
    });
    if between_digits {
        t.replace('_', "").parse::<f64>().ok()
    } else {
        None
    }
}

/// The reference's `float(value)`: numbers, booleans (1.0 / 0.0) and numeric texts; the rest fails.
fn float_value(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) => float_from_text(s),
        _ => None,
    }
}

/// The two texts of an alert, like the reference: the long one (e-mail) and the short one (SMS, Android notification,
/// banner). The date is the alert's, in UTC (`fired_at.strftime` of a UTC instant in the reference).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertMessages {
    pub long: String,
    pub short: String,
}

#[must_use]
pub fn alert_messages(
    rule_name: &str,
    severity: Severity,
    device_name: &str,
    value: &str,
    fired_at: DateTime<Utc>,
) -> AlertMessages {
    AlertMessages {
        long: format!(
            "[DomiNet {}] {rule_name}\nDevice: {device_name}\nValue: {value}\nTriggered at: {}",
            severity.label(),
            fired_at.format("%d/%m/%Y %H:%M:%S")
        ),
        short: format!("[DomiNet] {rule_name} — {device_name} : {value}"),
    }
}

/// A raised alert.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub id: AlertId,
    pub rule: Option<AlertRuleId>,
    pub device: DeviceId,
    pub status: AlertStatus,
    pub severity: Severity,
    pub value_triggered: String,
    pub fired_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_by: Option<UserId>,
    pub notified: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule(op: Operator, num: Option<f64>, text: &str) -> AlertRuleDraft {
        let mut r = AlertRuleDraft::new("R", DeviceId(1), "temperature");
        r.operator = op;
        r.threshold_num = num;
        r.threshold_str = text.into();
        r
    }

    #[test]
    fn alert_texts_are_those_of_reference() {
        use chrono::TimeZone;
        let m = alert_messages(
            "Canicule",
            Severity::Critical,
            "meteo_locale",
            "36.2",
            Utc.with_ymd_and_hms(2026, 7, 14, 15, 4, 9).unwrap(),
        );
        assert_eq!(
            m.long,
            "[DomiNet Critical] Canicule\nDevice: meteo_locale\nValue: 36.2\nTriggered at: 14/07/2026 15:04:09"
        );
        assert_eq!(m.short, "[DomiNet] Canicule — meteo_locale : 36.2");
    }

    #[test]
    fn numeric_operators_compare_numbers_including_numeric_text() {
        let gt = rule(Operator::Gt, Some(35.0), "");
        assert!(
            gt.evaluate(&json!(36.2)) && gt.evaluate(&json!("40")) && !gt.evaluate(&json!(true))
        );
        assert!(!gt.evaluate(&json!(35)) && !gt.evaluate(&json!(10)));
        assert!(rule(Operator::Gte, Some(35.0), "").evaluate(&json!(35)));
        assert!(rule(Operator::Lt, Some(0.0), "").evaluate(&json!(-1)));
        assert!(rule(Operator::Lte, Some(0.0), "").evaluate(&json!(0)));
        assert!(
            rule(Operator::Eq, Some(1.0), "").evaluate(&json!(true)),
            "float(True) = 1.0"
        );
        assert!(rule(Operator::Neq, Some(1.0), "").evaluate(&json!(2)));
        // Numeric text threshold if there is no numeric threshold.
        assert!(rule(Operator::Gt, None, " 20 ").evaluate(&json!(21)));
    }

    #[test]
    fn non_numeric_values_fall_back_to_text_equality_only_for_eq_and_neq() {
        let eq = rule(Operator::Eq, None, "On");
        assert!(
            eq.evaluate(&json!(" on ")) && eq.evaluate(&json!("ON")) && !eq.evaluate(&json!("OFF"))
        );
        assert!(rule(Operator::Neq, None, "ON").evaluate(&json!("OFF")));
        for op in [Operator::Gt, Operator::Gte, Operator::Lt, Operator::Lte] {
            assert!(!rule(op, Some(1.0), "").evaluate(&json!("abc")), "{op:?}");
        }
        assert!(
            !rule(Operator::Gt, None, "abc").evaluate(&json!(5)),
            "seuil texte non numerique : faux"
        );
        // A list is not a number: text fallback, `"[1]" != ""` is true (a quirk of the reference: the text threshold is empty here).
        assert!(rule(Operator::Neq, Some(1.0), "").evaluate(&json!([1])));
    }

    #[test]
    fn contains_searches_the_reference_text_of_the_value_ignoring_case() {
        let c = rule(Operator::Contains, None, "ouv");
        assert!(c.evaluate(&json!("OUVERT")) && !c.evaluate(&json!("ferme")));
        assert!(
            rule(Operator::Contains, None, "true").evaluate(&json!(true)),
            "str(True) = True"
        );
        assert!(rule(Operator::Contains, None, "none").evaluate(&Value::Null));
        assert!(rule(Operator::Contains, None, "5").evaluate(&json!(25)));
    }

    #[test]
    fn rules_are_validated_field_by_field() {
        assert_eq!(rule(Operator::Gt, Some(35.0), "").problems(), []);
        assert_eq!(
            rule(Operator::Gt, None, "").problems(),
            [("threshold_num", Problem::Required)]
        );
        assert_eq!(
            rule(Operator::Contains, Some(1.0), "").problems(),
            [("threshold_str", Problem::Required)]
        );
        assert_eq!(rule(Operator::Eq, None, "ON").problems(), []);
        let mut r = rule(Operator::Gt, Some(f64::NAN), "");
        assert_eq!(r.problems(), [("threshold_num", Problem::Invalid)]);
        r = rule(Operator::Gt, Some(1.0), "");
        r.name = " ".into();
        r.key = "a b".into();
        r.cooldown_minutes = 2000;
        assert_eq!(
            r.problems(),
            [
                ("name", Problem::Required),
                ("key", Problem::Invalid),
                (
                    "cooldown_minutes",
                    Problem::OutOfRange { min: 0, max: 1440 }
                )
            ]
        );
    }
}
