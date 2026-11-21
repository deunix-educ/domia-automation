// SPDX-License-Identifier: AGPL-3.0-or-later
//! Archive des segments video dans ReductStore (R6 lot 7, ADR 0049) : bucket `surveillance`, une entree `cam_<uuid>` par camera,
//! un enregistrement par segment fMP4 horodate en microsecondes, avec les etiquettes de l'implementation de reference (`event_type`,
//! `camera_name` en ASCII, `camera_uuid`). Quota FIFO : les plus anciens segments sont evinces par ReductStore.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dominet_domain::camera::{CameraUuid, RecordingEvent};
use reduct_rs::{Bucket, QuotaType, ReductClient, ReductError};
use tokio::sync::Mutex;

/// Nom du bucket des segments (celui de la reference).
pub const VIDEO_BUCKET: &str = "surveillance";

/// Nom de l'entree d'une camera.
#[must_use]
pub fn entry_name(uuid: CameraUuid) -> String {
    format!("cam_{uuid}")
}

/// Les etiquettes ReductStore n'acceptent que de l'ASCII : meme methode que reference (decomposition NFKD, puis on garde l'ASCII).
#[must_use]
pub fn ascii_label(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.nfkd().filter(char::is_ascii).collect()
}

/// Horodatage d'un segment en microsecondes (cle de l'enregistrement dans ReductStore).
#[must_use]
pub fn timestamp_us(at: DateTime<Utc>) -> u64 {
    u64::try_from(at.timestamp_micros()).unwrap_or(0)
}

/// Acces au bucket des segments.
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
    /// Pas d'acces reseau ici : le bucket est cree au premier usage.
    ///
    /// # Errors
    /// URL invalide, ou client HTTP impossible a construire.
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

    /// Ecrit un segment.
    ///
    /// # Errors
    /// Erreur si ReductStore est injoignable ou refuse l'ecriture (le segment local n'est alors pas supprime).
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

    /// Lit un segment (`None` s'il n'existe pas ou plus : evince par le quota).
    ///
    /// # Errors
    /// Erreur si ReductStore est injoignable.
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

    /// Supprime un segment (sans erreur s'il n'existe plus).
    ///
    /// # Errors
    /// Erreur si ReductStore est injoignable.
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

    /// Le bucket (tests et diagnostic).
    ///
    /// # Errors
    /// Erreur si ReductStore est injoignable.
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
