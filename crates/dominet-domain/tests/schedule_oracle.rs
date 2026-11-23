// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cron and solar events against the **reference implementation**: a cron in Europe/Paris time and
//! the reference's solar events (ephem). The expected values are frozen in the reference files.

use chrono::{DateTime, Utc};
use dominet_domain::automation::SolarEvent;
use dominet_domain::solar::next_event;
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../tests/fixtures/automation/schedule.json");

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn solar_events_stay_within_ninety_seconds_of_ephem() {
    let all: Value = serde_json::from_str(FIXTURE).unwrap();
    let cases = all["solar"].as_array().unwrap();
    assert!(cases.len() > 800, "{}", cases.len());
    let (mut worst, mut worst_case) = (0, String::new());
    let mut failures = vec![];
    for c in cases {
        let event = SolarEvent::parse(c["event"].as_str().unwrap()).unwrap();
        let (lat, lon) = (c["lat"].as_f64().unwrap(), c["lon"].as_f64().unwrap());
        let (start, expected) = (instant(&c["start"]), instant(&c["next"]));
        let Some(got) = next_event(event, lat, lon, start) else {
            failures.push(format!("{c}: aucun resultat"));
            continue;
        };
        let gap = (got - expected).num_seconds().abs();
        if gap > worst {
            worst = gap;
            worst_case = c.to_string();
        }
        if gap > 90 {
            failures.push(format!("{c}: obtenu {got}, ecart {gap} s"));
        }
    }
    println!("ecart maximal : {worst} s ({worst_case})");
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
