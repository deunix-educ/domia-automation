// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client of the MediaMTX REST API (`/v3/config/paths/...`, ADR 0044).
//!
//! One *path* per active camera, named by the camera's public identifier (32 hexadecimal digits). We **never** touch the other
//! paths (the `~.*` of the configuration, those of another use): only the 32-hexadecimal names are considered ours.
//!
//! The path's source contains the camera's password: it appears neither in `Debug`, nor in the errors, nor in the logs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use dominet_domain::camera::{CameraDraft, RecordingMode};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum MediaMtxError {
    #[error("MediaMTX injoignable : {0}")]
    Unreachable(String),
    #[error("MediaMTX a repondu {status} : {message}")]
    Status { status: u16, message: String },
    #[error("reponse de MediaMTX illisible")]
    BadResponse,
}

impl MediaMtxError {
    /// The path does not exist.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Status { status: 404, .. })
    }
}

/// Configuration of a path, in the names of the MediaMTX API. Absent options take the default value
/// (we use `replace`, so an option removed from a camera is indeed removed from the path).
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathConf {
    /// Camera stream URL, **password included**.
    pub source: String,
    pub source_on_demand: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub rtsp_any_port: bool,
    /// `tcp` to force the TCP transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_protocol: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub record: bool,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature imposee par serde"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl fmt::Debug for PathConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathConf")
            .field("source", &"***")
            .field("source_on_demand", &self.source_on_demand)
            .field("rtsp_any_port", &self.rtsp_any_port)
            .field("source_protocol", &self.source_protocol)
            .field("record", &self.record)
            .finish()
    }
}

impl PathConf {
    /// What the reference sends: always-open RTSP source, `rtspAnyPort` and TCP according to the options,
    /// recording for the continuous and motion modes. `password` is the **decrypted** RTSP password.
    #[must_use]
    pub fn for_camera(camera: &CameraDraft, password: &str) -> Self {
        Self {
            source: camera.source_url(password),
            source_on_demand: false,
            rtsp_any_port: camera.rtsp_any_port,
            source_protocol: camera.rtsp_transport_tcp.then(|| "tcp".to_string()),
            record: matches!(
                camera.recording_mode,
                RecordingMode::Continuous | RecordingMode::Motion
            ),
        }
    }
}

/// A path name that DomiNet manages: 32 lowercase hexadecimal digits.
#[must_use]
pub fn is_camera_path(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Outcome of a complete synchronisation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub replaced: Vec<String>,
    pub removed: Vec<String>,
    /// (path, message): a failure does not stop the others.
    pub failed: Vec<(String, String)>,
}

impl SyncReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct MediaMtx {
    http: reqwest::Client,
    base: String,
    page_size: u32,
}

impl MediaMtx {
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn new(base_url: &str) -> Result<Self, MediaMtxError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| MediaMtxError::Unreachable(e.to_string()))?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
            page_size: 500,
        })
    }

    /// Page size of the paths list (500 by default; reduced to test pagination).
    #[must_use]
    pub fn with_page_size(mut self, n: u32) -> Self {
        self.page_size = n.max(1);
        self
    }

    fn url(&self, action: &str, name: &str) -> String {
        format!("{}/v3/config/paths/{action}/{name}", self.base)
    }

    /// MediaMTX error response, without ever leaving the source in it (which carries the password).
    async fn error(resp: reqwest::Response, secret: Option<&str>) -> MediaMtxError {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let mut message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| body.chars().take(200).collect());
        if let Some(s) = secret.filter(|s| !s.is_empty()) {
            message = message.replace(s, "***");
        }
        MediaMtxError::Status { status, message }
    }

    fn unreachable(e: &reqwest::Error) -> MediaMtxError {
        // `without_url`: MediaMTX's address is not a secret, but the text stays short.
        MediaMtxError::Unreachable(e.to_string())
    }

    /// Names of all the configured paths.
    ///
    /// # Errors
    /// MediaMTX unreachable or unexpected response.
    pub async fn list_names(&self) -> Result<Vec<String>, MediaMtxError> {
        let mut names = vec![];
        let mut page = 0u32;
        loop {
            let resp = self
                .http
                .get(format!(
                    "{}/v3/config/paths/list?itemsPerPage={}&page={page}",
                    self.base, self.page_size
                ))
                .send()
                .await
                .map_err(|e| Self::unreachable(&e))?;
            if !resp.status().is_success() {
                return Err(Self::error(resp, None).await);
            }
            let v: Value = resp.json().await.map_err(|_| MediaMtxError::BadResponse)?;
            for item in v["items"].as_array().ok_or(MediaMtxError::BadResponse)? {
                if let Some(n) = item["name"].as_str() {
                    names.push(n.to_string());
                }
            }
            let pages = v["pageCount"].as_u64().ok_or(MediaMtxError::BadResponse)?;
            page += 1;
            if u64::from(page) >= pages {
                return Ok(names);
            }
        }
    }

    /// Configuration of a path (`None` if it does not exist).
    ///
    /// # Errors
    /// MediaMTX unreachable or unexpected response.
    pub async fn get(&self, name: &str) -> Result<Option<Value>, MediaMtxError> {
        let resp = self
            .http
            .get(self.url("get", name))
            .send()
            .await
            .map_err(|e| Self::unreachable(&e))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(Self::error(resp, None).await);
        }
        resp.json()
            .await
            .map(Some)
            .map_err(|_| MediaMtxError::BadResponse)
    }

    async fn send_conf(
        &self,
        action: &str,
        name: &str,
        conf: &PathConf,
    ) -> Result<(), MediaMtxError> {
        let resp = self
            .http
            .post(self.url(action, name))
            .json(conf)
            .send()
            .await
            .map_err(|e| Self::unreachable(&e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Self::error(resp, Some(&conf.source)).await)
        }
    }

    /// # Errors
    /// The path already exists, MediaMTX refuses the source or is unreachable.
    pub async fn add(&self, name: &str, conf: &PathConf) -> Result<(), MediaMtxError> {
        self.send_conf("add", name, conf).await
    }

    /// Entirely replaces the configuration of a path; MediaMTX **creates** it if it does not exist (checked on version 1.21.3).
    ///
    /// # Errors
    /// MediaMTX refuses the configuration or is unreachable.
    pub async fn replace(&self, name: &str, conf: &PathConf) -> Result<(), MediaMtxError> {
        self.send_conf("replace", name, conf).await
    }

    /// Turns the recording of a path on or off without touching the rest of its configuration (`PATCH`): manual recording.
    ///
    /// # Errors
    /// Unknown path (`is_not_found`), MediaMTX unreachable or refusal.
    pub async fn set_record(&self, name: &str, record: bool) -> Result<(), MediaMtxError> {
        let resp = self
            .http
            .patch(self.url("patch", name))
            .json(&serde_json::json!({ "record": record }))
            .send()
            .await
            .map_err(|e| Self::unreachable(&e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Self::error(resp, None).await)
        }
    }

    /// Deletes a path; an already absent path is not an error.
    ///
    /// # Errors
    /// MediaMTX unreachable or refusal.
    pub async fn remove(&self, name: &str) -> Result<(), MediaMtxError> {
        let resp = self
            .http
            .delete(self.url("delete", name))
            .send()
            .await
            .map_err(|e| Self::unreachable(&e))?;
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            Ok(())
        } else {
            Err(Self::error(resp, None).await)
        }
    }

    /// Brings MediaMTX in line with the wanted list: creates or replaces each wanted path, deletes the camera paths (32
    /// hexadecimal digits) that are no longer wanted, touches no other.
    ///
    /// # Errors
    /// Only if the paths list cannot be read; per-path failures are in the outcome.
    pub async fn sync(
        &self,
        wanted: &BTreeMap<String, PathConf>,
    ) -> Result<SyncReport, MediaMtxError> {
        self.sync_keeping(wanted, &[]).await
    }

    /// Like [`Self::sync`], without touching the paths of `keep` (cameras whose configuration could not be computed).
    ///
    /// # Errors
    /// Only if the paths list cannot be read.
    pub async fn sync_keeping(
        &self,
        wanted: &BTreeMap<String, PathConf>,
        keep: &[String],
    ) -> Result<SyncReport, MediaMtxError> {
        let existing: BTreeSet<String> = self.list_names().await?.into_iter().collect();
        let mut report = SyncReport::default();
        for (name, conf) in wanted {
            let result = if existing.contains(name) {
                self.replace(name, conf).await.map(|()| false)
            } else {
                self.add(name, conf).await.map(|()| true)
            };
            match result {
                Ok(true) => report.added.push(name.clone()),
                Ok(false) => report.replaced.push(name.clone()),
                Err(e) => report.failed.push((name.clone(), e.to_string())),
            }
        }
        for name in existing
            .iter()
            .filter(|n| is_camera_path(n) && !wanted.contains_key(*n) && !keep.contains(n))
        {
            match self.remove(name).await {
                Ok(()) => report.removed.push(name.clone()),
                Err(e) => report.failed.push((name.clone(), e.to_string())),
            }
        }
        tracing::info!(
            added = report.added.len(),
            replaced = report.replaced.len(),
            removed = report.removed.len(),
            failed = report.failed.len(),
            "MediaMTX synchronisation"
        );
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_paths_are_32_lowercase_hex() {
        assert!(is_camera_path("0123456789abcdef0123456789abcdef"));
        assert!(!is_camera_path("0123456789ABCDEF0123456789abcdef"));
        assert!(!is_camera_path("~.*"));
        assert!(!is_camera_path("all_others"));
        assert!(!is_camera_path("0123456789abcdef"));
    }

    #[test]
    fn the_conf_follows_reference_and_never_shows_its_source() {
        let mut d = CameraDraft::new("c");
        d.ip = "192.168.1.29".into();
        d.rtsp_user = "admin".into();
        d.path = "stream1".into();
        let plain = PathConf::for_camera(&d, "s3cret");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            serde_json::json!({"source": "rtsp://admin:s3cret@192.168.1.29:554/stream1", "sourceOnDemand": false})
        );
        d.rtsp_any_port = true;
        d.rtsp_transport_tcp = true;
        d.recording_mode = RecordingMode::Motion;
        let full = serde_json::to_value(PathConf::for_camera(&d, "s3cret")).unwrap();
        assert_eq!(full["rtspAnyPort"], true);
        assert_eq!(full["sourceProtocol"], "tcp");
        assert_eq!(full["record"], true);
        d.recording_mode = RecordingMode::Manual;
        assert!(!PathConf::for_camera(&d, "x").record);
        assert!(!format!("{plain:?}").contains("s3cret"));
    }
}
