// SPDX-License-Identifier: AGPL-3.0-or-later
//! Grafana dashboard of a device (R3 lot 5, ADR 0029): one panel per tracked key, same shape as that of the
//! reference version. The document is written **to a file** in the folder watched by Grafana, never through
//! the API (a dashboard loaded from a file becomes "provisioned" and the API then refuses to modify it).
//!
//! Only difference: the table read is called `device_state_log` (it used to be `automation_devicestatelog`).

use serde_json::{Value, json};

use crate::device::Device;

/// Grafana's PostgreSQL data source (uid provisioned by `monitoring/grafana/provisioning/datasources`).
fn datasource() -> Value {
    json!({"type": "grafana-postgresql-datasource", "uid": "postgresql"})
}

/// Safe file name: the device name may contain `/` and spaces.
#[must_use]
pub fn dashboard_file_name(device: &Device) -> String {
    let mut slug = String::new();
    let mut last_sep = true;
    for c in device.friendly_name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
            slug.push(c);
            last_sep = false;
        } else if !last_sep {
            slug.push('_');
            last_sep = true;
        }
    }
    let slug = slug.trim_matches(['_', '.']);
    let slug = if slug.is_empty() { "device" } else { slug };
    format!("device_{slug}.json")
}

fn panel(device: &Device, key: &str, index: usize) -> Value {
    let (x, y) = ((index % 2) * 12, (index / 2) * 8);
    json!({
        "id": index + 1,
        "title": key,
        "type": "timeseries",
        "datasource": datasource(),
        "gridPos": {"x": x, "y": y, "w": 12, "h": 8},
        "fieldConfig": {"defaults": {"color": {"mode": "palette-classic"}, "custom": {"lineWidth": 2, "fillOpacity": 10}}},
        "targets": [{
            "datasource": datasource(),
            // The tracked keys are validated (letters, digits, `_ . : -`): no apostrophe possible.
            "rawSql": format!(
                "SELECT ts AS \"time\", value_num AS \"{key}\"\nFROM device_state_log\nWHERE device_id = {id}\n  AND key = '{key}'\n  AND $__timeFilter(ts)\nORDER BY ts",
                id = device.id.0
            ),
            "format": "time_series",
            "refId": "A",
        }],
    })
}

/// The dashboard document; `None` if the device has no tracked key (nothing to show).
#[must_use]
pub fn dashboard_json(device: &Device, version: u64) -> Option<Value> {
    if device.tracked_keys.is_empty() {
        return None;
    }
    let panels: Vec<Value> = device
        .tracked_keys
        .iter()
        .enumerate()
        .map(|(i, k)| panel(device, k, i))
        .collect();
    Some(json!({
        "uid": device.grafana_uid(),
        "title": format!("DomiNet — {}", device.friendly_name),
        "tags": ["dominet", "device"],
        "schemaVersion": 38,
        "version": version,
        "time": {"from": "now-24h", "to": "now"},
        "refresh": "30s",
        "panels": panels,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::device::{DeviceId, DeviceType};

    fn device(name: &str, keys: &[&str]) -> Device {
        Device {
            id: DeviceId(42),
            friendly_name: name.to_string(),
            ieee: String::new(),
            vendor: String::new(),
            model_id: String::new(),
            description: String::new(),
            device_type: DeviceType::Sensor,
            group: None,
            location: None,
            base_topic: "zigbee2mqtt".into(),
            state: json!({}),
            last_seen: None,
            available: true,
            exposes: vec![],
            active: true,
            icon: String::new(),
            image_url: String::new(),
            tracked_keys: keys.iter().map(ToString::to_string).collect(),
            hidden_keys: vec![],
            grafana_dashboard: false,
        }
    }

    #[test]
    fn one_panel_per_tracked_key_in_two_columns() {
        let d = dashboard_json(
            &device("Sonde salon", &["temperature", "humidity", "battery"]),
            3,
        )
        .unwrap();
        assert_eq!(d["uid"], "dominet-device-42");
        assert_eq!(d["title"], "DomiNet — Sonde salon");
        assert_eq!(d["version"], 3);
        let panels = d["panels"].as_array().unwrap();
        assert_eq!(panels.len(), 3);
        assert_eq!(
            panels[0]["gridPos"],
            json!({"x": 0, "y": 0, "w": 12, "h": 8})
        );
        assert_eq!(panels[1]["gridPos"]["x"], 12);
        assert_eq!(
            panels[2]["gridPos"],
            json!({"x": 0, "y": 8, "w": 12, "h": 8})
        );
        let sql = panels[1]["targets"][0]["rawSql"].as_str().unwrap();
        assert!(
            sql.contains("FROM device_state_log")
                && sql.contains("device_id = 42")
                && sql.contains("key = 'humidity'")
        );
        assert!(sql.contains("$__timeFilter(ts)"));
    }

    #[test]
    fn a_device_without_tracked_keys_has_no_dashboard() {
        assert!(dashboard_json(&device("x", &[]), 1).is_none());
    }

    #[test]
    fn file_names_are_safe_whatever_the_device_name() {
        let n = |name: &str| dashboard_file_name(&device(name, &[]));
        assert_eq!(n("Sonde salon"), "device_Sonde_salon.json");
        assert_eq!(n("salon/lampe 1"), "device_salon_lampe_1.json");
        assert_eq!(n("../../etc/passwd"), "device_etc_passwd.json");
        assert!(!n("../../etc/passwd").contains(".."));
        assert!(!n("a/b").contains('/'));
        assert_eq!(n("///"), "device_device.json");
        assert_eq!(n("été"), "device_t.json");
    }
}
