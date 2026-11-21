// SPDX-License-Identifier: AGPL-3.0-or-later
//! Evaluation d'une regle d'alerte contre le **implementation de reference** : 2 030 combinaisons
//! d'operateur, de seuil et de valeur (nombres, textes numeriques, booleens, listes, `nan`, `inf`, `1_000`...).

use dominet_domain::alert::{AlertRuleDraft, Operator};
use dominet_domain::device::DeviceId;
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../tests/fixtures/automation/alerts.json");

#[test]
fn rule_evaluation_agrees_with_reference_on_every_case() {
    let all: Value = serde_json::from_str(FIXTURE).unwrap();
    let cases = all["evaluate"].as_array().unwrap();
    assert_eq!(cases.len(), 2030);
    let mut failures = vec![];
    for c in cases {
        let mut rule = AlertRuleDraft::new("R", DeviceId(1), "k");
        rule.operator = Operator::parse(c["operator"].as_str().unwrap()).unwrap();
        rule.threshold_num = c["threshold_num"].as_f64();
        rule.threshold_str = c["threshold_str"].as_str().unwrap().to_string();
        let got = rule.evaluate(&c["value"]);
        if got != c["holds"].as_bool().unwrap() {
            failures.push(format!("{c} -> obtenu {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} ecarts :\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
