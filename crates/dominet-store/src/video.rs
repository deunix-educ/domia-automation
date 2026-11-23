// SPDX-License-Identifier: AGPL-3.0-or-later
//! Archive of the video segments in ReductStore (R6 lot 7, ADR 0049): bucket `surveillance`, one `cam_<uuid>` entry per camera,
//! one record per fMP4 segment timestamped in microseconds, with the labels of the reference implementation (`event_type`,
//! `camera_name` in ASCII, `camera_uuid`). FIFO quota: the oldest segments are evicted by ReductStore.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dominet_domain::camera::{CameraUuid, RecordingEvent};
use reduct_rs::{Bucket, QuotaType, ReductClient, ReductError};
use tokio::sync::Mutex;

/// Name of the segments bucket (the reference's).
pub const VIDEO_BUCKET: &str = "surveillance";

/// Name of a camera's entry.
#[must_use]
pub fn entry_name(uuid: CameraUuid) -> String {
    format!("cam_{uuid}")
}

/// ReductStore labels only accept ASCII: same method as the reference (NFKD decomposition, then we keep the ASCII).
#[must_use]
pub fn ascii_label(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.nfkd().filter(char::is_ascii).collect()
}

/// Timestamp of a segment in microseconds (key of the record in ReductStore).
#[must_use]
pub fn timestamp_us(at: DateTime<Utc>) -> u64 {
    u64::try_from(at.timestamp_micros()).unwrap_or(0)
}

/// Access to the segments bucket.
pub struct ReductVideo {
    client: ReductClient,
    bucket_name: String,
    quota_bytes: u64,
    bucket: Mutex<Option<Arc<Bucket>>>,
}

impl std::fmt::Debug for ReductVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReductVideo")
            .field("bucket", &self.bucket_name)
            .field("quota_bytes", &self.quota_bytes)
            .finish_non_exhaustive()
    }
}

impl ReductVideo {
    /// No network access here: the bucket is created at first use.
    ///
    /// # Errors
    /// Invalid URL, or HTTP client that cannot be built.
    pub fn new(
        url: &str,
        token: Option<&str>,
        bucket: &str,
        quota_bytes: u64,
    ) -> Result<Self, ReductError> {
        let mut builder = ReductClient::builder().url(url);
        if let Some(t) = token {
            builder = builder.api_token(t);
        }
        Ok(Self {
            client: builder.try_build()?,
            bucket_name: bucket.to_string(),
            quota_bytes,
            bucket: Mutex::new(None),
        })
    }

    async fn bucket(&self) -> Result<Arc<Bucket>, ReductError> {
        let mut slot = self.bucket.lock().await;
        if let Some(b) = slot.as_ref() {
            return Ok(Arc::clone(b));
        }
        let bucket = Arc::new(
            self.client
                .create_bucket(&self.bucket_name)
                .exist_ok(true)
                .quota_type(QuotaType::FIFO)
                .quota_size(self.quota_bytes)
                .send()
                .await?,
        );
        *slot = Some(Arc::clone(&bucket));
        Ok(bucket)
    }

    /// Writes a segment.
    ///
    /// # Errors
    /// Error if ReductStore is unreachable or refuses the write (the local segment is then not deleted).
    pub async fn push(
        &self,
        camera: CameraUuid,
        camera_name: &str,
        at: DateTime<Utc>,
        event: RecordingEvent,
        data: Vec<u8>,
    ) -> Result<(), ReductError> {
        let bucket = self.bucket().await?;
        let result = bucket
            .write_record(&entry_name(camera))
            .timestamp_us(timestamp_us(at))
            .content_type("video/mp4")
            .add_label("event_type", event.as_str())
            .add_label("camera_name", ascii_label(camera_name).as_str())
            .add_label("camera_uuid", camera.to_string().as_str())
            .data(data)
            .send()
            .await;
        if result.is_err() {
            *self.bucket.lock().await = None;
        }
        result
    }

    /// Reads a segment (`None` if it does not exist or no longer does: evicted by the quota).
    ///
    /// # Errors
    /// Error if ReductStore is unreachable.
    pub async fn read(
        &self,
        camera: CameraUuid,
        timestamp_us: u64,
    ) -> Result<Option<Vec<u8>>, ReductError> {
        let bucket = self.bucket().await?;
        match bucket
            .read_record(&entry_name(camera))
            .timestamp_us(timestamp_us)
            .send()
            .await
        {
            Ok(record) => Ok(Some(record.bytes().await?.to_vec())),
            Err(e) if e.status == reduct_rs::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Deletes a segment (no error if it no longer exists).
    ///
    /// # Errors
    /// Error if ReductStore is unreachable.
    pub async fn remove(&self, camera: CameraUuid, timestamp_us: u64) -> Result<(), ReductError> {
        let bucket = self.bucket().await?;
        match bucket
            .remove_record(&entry_name(camera))
            .timestamp_us(timestamp_us)
            .send()
            .await
        {
            Err(e) if e.status != reduct_rs::ErrorCode::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// The bucket (tests and diagnostics).
    ///
    /// # Errors
    /// Error if ReductStore is unreachable.
    pub async fn open(&self) -> Result<Arc<Bucket>, ReductError> {
        self.bucket().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_plain_ascii() {
        assert_eq!(ascii_label("Entr\u{e9}e gar\u{e7}on"), "Entree garcon");
        assert_eq!(ascii_label("Jardin \u{65e5}\u{672c}"), "Jardin ");
    }

    #[test]
    fn entry_and_timestamp_follow_the_reference_layout() {
        let uuid: CameraUuid = "5ca0e51964bd43c3bfebec36be21afc3".parse().unwrap();
        assert_eq!(entry_name(uuid), "cam_5ca0e51964bd43c3bfebec36be21afc3");
        let at = DateTime::from_timestamp(1_700_000_000, 500_000_000).unwrap();
        assert_eq!(timestamp_us(at), 1_700_000_000_500_000);
    }
}
