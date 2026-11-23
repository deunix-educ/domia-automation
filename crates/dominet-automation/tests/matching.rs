// SPDX-License-Identifier: AGPL-3.0-or-later
//! `mqtt_message` trigger against the **reference implementation**: topic matching, condition
//! and payload filter, on a grid of cases whose expected values are frozen in the reference files.

use dominet_automation::engine::topic_matches_py;
use dominet_domain::automation::{Condition, payload_filter_holds};
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../tests/fixtures/automation/matching.json");

fn cases(kind: &str) -> Vec<Value> {
    let all: Value = serde_json::from_str(FIXTURE).unwrap();
    all[kind].as_array().unwrap().clone()
}

#[test]
fn topic_matching_agrees_with_reference_on_every_case() {
    let cases = cases("topics");
    assert!(cases.len() > 100);
    for c in &cases {
        let (pattern, topic) = (c["pattern"].as_str().unwrap(), c["topic"].as_str().unwrap());
        assert_eq!(
            topic_matches_py(pattern, topic),
            c["matches"].as_bool().unwrap(),
            "{pattern:?} contre {topic:?}"
        );
    }
}

#[test]
fn conditions_agree_with_reference_on_every_case() {
    let cases = cases("conditions");
    assert!(cases.len() > 50);
    for c in &cases {
        let cond = Condition::from_json(&c["condition"]).unwrap().unwrap();
        assert_eq!(
            cond.holds(&c["payload"]),
            c["holds"].as_bool().unwrap(),
            "{} contre {}",
            c["condition"],
            c["payload"]
        );
    }
}

#[test]
fn payload_filters_agree_with_reference_on_every_case() {
    let cases = cases("filters");
    assert!(cases.len() > 50);
    for c in &cases {
        let filter = c["filter"].as_object().unwrap();
        assert_eq!(
            payload_filter_holds(filter, &c["payload"]),
            c["passes"].as_bool().unwrap(),
            "{} contre {}",
            c["filter"],
            c["payload"]
        );
    }
}
