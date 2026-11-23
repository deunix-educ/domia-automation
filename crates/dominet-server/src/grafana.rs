// SPDX-License-Identifier: AGPL-3.0-or-later
//! Writing of a device's Grafana dashboard into the folder watched by Grafana (R3 lot 5, ADR 0029).

use std::path::{Path, PathBuf};

use dominet_domain::device::Device;
use dominet_domain::grafana::{dashboard_file_name, dashboard_json};
use serde_json::Value;

/// Version to write: that of the existing file + 1 (Grafana ignores a version that does not change).
fn next_version(path: &Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("version").and_then(Value::as_u64))
        .map_or(1, |v| v + 1)
}

/// Deletes the old `device_*.json` files of the same dashboard (renamed device).
fn remove_stale(dir: &Path, current: &Path, uid: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_device_file = path.extension().is_some_and(|e| e == "json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("device_"));
        if !is_device_file || path == current {
            continue;
        }
        let same_uid = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| v.get("uid").and_then(Value::as_str) == Some(uid));
        if same_uid {
            let _ = std::fs::remove_file(&path);
            tracing::info!(file = %path.display(), "old Grafana dashboard removed (device renamed)");
        }
    }
}

/// Writes the device's dashboard into `dir` (created if needed): temporary file then rename, so that Grafana never reads
/// a half-written file. Returns `Ok(None)` if the device has no tracked key.
///
/// # Errors
/// Write error in the folder.
pub fn write_dashboard(dir: &Path, device: &Device) -> std::io::Result<Option<PathBuf>> {
    let path = dir.join(dashboard_file_name(device));
    let Some(document) = dashboard_json(device, next_version(&path)) else {
        return Ok(None);
    };
    std::fs::create_dir_all(dir)?;
    remove_stale(dir, &path, &device.grafana_uid());
    let tmp = dir.join(format!(".{}.tmp", dashboard_file_name(device)));
    let text = serde_json::to_string_pretty(&document).map_err(std::io::Error::other)? + "\n";
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(Some(path))
}

/// [`write_dashboard`] off the request thread.
///
/// # Errors
/// Write error in the folder.
pub async fn write_dashboard_async(
    dir: PathBuf,
    device: Device,
) -> std::io::Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || write_dashboard(&dir, &device))
        .await
        .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use dominet_domain::device::{DeviceId, DeviceType};
    use serde_json::json;

    use super::*;

    fn device(id: i64, name: &str, keys: &[&str]) -> Device {
        Device {
            id: DeviceId(id),
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

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dominet-grafana-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_file_is_written_versioned_and_replaced_atomically() {
        let dir = scratch("write");
        let d = device(7, "Sonde", &["temperature"]);
        let path = write_dashboard(&dir, &d).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap(), "device_Sonde.json");
        let v1: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            (v1["version"].clone(), v1["uid"].clone()),
            (json!(1), json!("dominet-device-7"))
        );
        write_dashboard(&dir, &device(7, "Sonde", &["temperature", "humidity"])).unwrap();
        let v2: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v2["version"], 2);
        assert_eq!(v2["panels"].as_array().unwrap().len(), 2);
        // No temporary file is left.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["device_Sonde.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_device_replaces_its_old_file_and_leaves_the_others() {
        let dir = scratch("rename");
        write_dashboard(&dir, &device(7, "Ancien nom", &["temperature"])).unwrap();
        write_dashboard(&dir, &device(8, "Autre", &["temperature"])).unwrap();
        // A dashboard of another origin (without `device_`) is never touched.
        std::fs::write(dir.join("weather.json"), "{\"uid\":\"dominet-device-7\"}").unwrap();
        write_dashboard(&dir, &device(7, "Nouveau nom", &["temperature"])).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "device_Autre.json",
                "device_Nouveau_nom.json",
                "weather.json"
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_is_written_without_tracked_keys() {
        let dir = scratch("empty");
        assert!(
            write_dashboard(&dir, &device(1, "x", &[]))
                .unwrap()
                .is_none()
        );
        assert!(!dir.exists());
    }
}
