// SPDX-License-Identifier: AGPL-3.0-or-later
//! Raw `iot` archive against the ReductStore of the dev stack (throw-away bucket, deleted at the end of the test).
//!
//! `REDUCTSTORE_URL` (default `http://127.0.0.1:18383`) and `REDUCTSTORE_TOKEN` (default `dominet-dev-token`).

use std::time::Duration;

use chrono::{TimeZone, Utc};
use dominet_domain::device::{DeviceId, LogEntry};
use dominet_store::reduct::{IotRecord, IotWriter, ReductIot, entry_name};
use serde_json::json;

fn url() -> String {
    std::env::var("REDUCTSTORE_URL").unwrap_or_else(|_| "http://127.0.0.1:18383".into())
}

fn token() -> String {
    std::env::var("REDUCTSTORE_TOKEN").unwrap_or_else(|_| "dominet-dev-token".into())
}

fn bucket_name(tag: &str) -> String {
    format!("test-iot-{tag}-{}", std::process::id())
}

fn record(device: i64, secs: i64, temperature: f64) -> IotRecord {
    IotRecord {
        device: DeviceId(device),
        at: Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap(),
        state: json!({"temperature": temperature, "state": "ON"}),
        entries: vec![
            LogEntry {
                key: "temperature".into(),
                value_num: Some(temperature),
                value_str: String::new(),
            },
            LogEntry {
                key: "state".into(),
                value_num: None,
                value_str: "ON".into(),
            },
        ],
    }
}

async fn wait_for_records(archive: &ReductIot, entry: &str, expected: u64) -> u64 {
    for _ in 0..100 {
        if let Ok(bucket) = archive.open().await {
            let count = bucket
                .entries()
                .await
                .unwrap()
                .iter()
                .find(|e| e.name == entry)
                .map_or(0, |e| e.record_count);
            if count >= expected {
                return count;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    0
}

#[tokio::test]
async fn records_are_archived_with_numeric_labels_and_unique_timestamps() {
    let name = bucket_name("write");
    let archive = ReductIot::with_bucket(&url(), Some(&token()), &name, 10_000_000).unwrap();
    let reader = ReductIot::with_bucket(&url(), Some(&token()), &name, 10_000_000).unwrap();
    let writer = IotWriter::spawn(archive);

    // Three messages from the same device in the same second: three distinct records (shifted timestamps).
    for _ in 0..3 {
        writer.push(record(7, 0, 21.5));
    }
    writer.push(record(7, 1, 22.0));
    let entry = entry_name(DeviceId(7));
    assert_eq!(wait_for_records(&reader, &entry, 4).await, 4);
    assert_eq!(writer.dropped(), 0);

    let bucket = reader.open().await.unwrap();
    let last = bucket.read_record(&entry).send().await.unwrap();
    assert_eq!(
        last.labels().get("temperature").map(String::as_str),
        Some("22")
    );
    assert!(
        !last.labels().contains_key("state"),
        "les cles textuelles ne sont pas des etiquettes"
    );
    let body: serde_json::Value = serde_json::from_slice(&last.bytes().await.unwrap()).unwrap();
    assert_eq!(body, json!({"temperature": 22.0, "state": "ON"}));

    bucket.remove().await.unwrap();
}

#[tokio::test]
async fn an_unreachable_server_never_blocks_and_records_are_counted_as_dropped() {
    let archive = ReductIot::new("http://127.0.0.1:1", Some("x"), 1_000_000).unwrap();
    let writer = IotWriter::spawn(archive);
    for i in 0..5 {
        writer.push(record(1, i, 20.0));
    }
    for _ in 0..100 {
        if writer.dropped() >= 5 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("enregistrements non comptes : {}", writer.dropped());
}
