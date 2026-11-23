// SPDX-License-Identifier: AGPL-3.0-or-later
//! Next runs of a cron in `Europe/Paris` time against frozen reference values: 119 series of six runs, including clock-change nights, the end of the year and 29 February.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use dominet_automation::next::{cron_next, next_run};
use dominet_domain::cron::CronSpec;
use dominet_domain::schedule::JobSchedule;
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../tests/fixtures/automation/schedule.json");

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn cron_occurrences_agree_with_reference_including_daylight_saving_changes() {
    let all: Value = serde_json::from_str(FIXTURE).unwrap();
    let cases = all["cron"].as_array().unwrap();
    assert_eq!(cases.len(), 119);
    let mut failures = vec![];
    for c in cases {
        let expr = c["expr"].as_str().unwrap();
        let spec = CronSpec::parse(expr).unwrap();
        let mut t = instant(&c["start"]);
        let mut got = vec![];
        for _ in 0..6 {
            match cron_next(&spec, Paris, t) {
                Some(n) => {
                    got.push(n);
                    t = n;
                }
                None => break,
            }
        }
        let expected: Vec<DateTime<Utc>> =
            c["next"].as_array().unwrap().iter().map(instant).collect();
        if got != expected {
            failures.push(format!(
                "{expr} depuis {} :\n  attendu {expected:?}\n  obtenu  {got:?}",
                c["start"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} ecarts :\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn an_invalid_or_impossible_schedule_has_no_next_run() {
    let now = Utc::now();
    assert_eq!(
        next_run(&JobSchedule::Cron("pas un cron".into()), Paris, now),
        None
    );
    assert_eq!(
        next_run(&JobSchedule::Cron("0 0 31 4 *".into()), Paris, now),
        None
    );
}
