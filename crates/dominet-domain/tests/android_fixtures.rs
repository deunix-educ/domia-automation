// SPDX-License-Identifier: AGPL-3.0-or-later
//! Trames de l'application Android (`tests/fixtures/android/frames.json`, tirees du code Kotlin non modifie) : le serveur Rust doit
//! lire exactement ce que l'application envoie et produire exactement ce qu'elle comprend.

use chrono::DateTime;
use dominet_domain::android::{DeviceFrame, ServerFrame, parse_device_frame};
use serde_json::Value;

fn fixtures() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/android/frames.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_frame_of_the_app_is_read_as_expected() {
    let fx = fixtures();
    let cases = fx["from_app"].as_array().unwrap();
    assert!(cases.len() >= 7);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let e = &c["expect"];
        let got = parse_device_frame(c["json"].as_str().unwrap())
            .unwrap_or_else(|| panic!("{name}: illisible"));
        match (e["kind"].as_str().unwrap(), got) {
            ("sms_received", DeviceFrame::SmsReceived(s)) => {
                assert_eq!(s.from, e["from"].as_str().unwrap(), "{name}");
                assert_eq!(s.body, e["body"].as_str().unwrap(), "{name}");
                assert_eq!(
                    s.contact_name,
                    e["contact_name"].as_str().unwrap(),
                    "{name}"
                );
                let ts = DateTime::parse_from_rfc3339(e["timestamp"].as_str().unwrap()).unwrap();
                assert_eq!(s.timestamp, Some(ts.to_utc()), "{name}");
            }
            ("call_event", DeviceFrame::CallEvent(c)) => {
                assert_eq!(c.state.as_str(), e["state"].as_str().unwrap(), "{name}");
                assert_eq!(c.number, e["number"].as_str().unwrap(), "{name}");
                assert_eq!(
                    i64::from(c.duration),
                    e["duration"].as_i64().unwrap(),
                    "{name}"
                );
                assert!(c.timestamp.is_some(), "{name}");
            }
            ("sms_sent_ack", DeviceFrame::SmsSentAck(a)) => {
                assert_eq!(a.message_id, e["message_id"].as_i64(), "{name}");
                assert_eq!(a.success, e["success"].as_bool().unwrap(), "{name}");
                assert_eq!(a.error, e["error"].as_str().unwrap(), "{name}");
            }
            (kind, other) => panic!("{name}: attendu {kind}, obtenu {other:?}"),
        }
    }
}

#[test]
fn every_frame_for_the_app_has_the_expected_json() {
    let fx = fixtures();
    for c in fx["to_app"].as_array().unwrap() {
        let f = &c["frame"];
        let frame = match f["kind"].as_str().unwrap() {
            "auth_ok" => ServerFrame::AuthOk {
                device_name: f["device_name"].as_str().unwrap().into(),
            },
            "send_sms" => ServerFrame::SendSms {
                to: f["to"].as_str().unwrap().into(),
                body: f["body"].as_str().unwrap().into(),
                message_id: f["message_id"].as_i64().unwrap(),
            },
            "pong" => ServerFrame::Pong,
            k => panic!("{k}"),
        };
        let got: Value = serde_json::from_str(&frame.to_json()).unwrap();
        assert_eq!(got, c["json"], "{}", c["name"]);
    }
}
