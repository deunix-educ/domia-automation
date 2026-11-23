// SPDX-License-Identifier: AGPL-3.0-or-later
//! Raw archive of the history in ReductStore: bucket `iot`, one `dev_<id>` entry per device (ADR 0002 and 0028).
//!
//! One record per message that changes at least one tracked key: the body is the full JSON state, one label per
//! tracked numeric key allows threshold queries. PostgreSQL remains the source of the charts; ReductStore is optional and
//! **never blocks ingestion**: writes go through a bounded queue and a background writer, which retries the
//! bucket creation as long as the server is unreachable and drops (counting them) the records that do not go through.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dominet_domain::device::{DeviceId, LogEntry};
use reduct_rs::{Bucket, QuotaType, Record, ReductClient};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

/// Name of the raw-series bucket.
pub const IOT_BUCKET: &str = "iot";
/// Largest batch written at once (the batch metadata travels in the HTTP headers: ADR 0002).
const MAX_BATCH: usize = 50;
/// Size of the queue between ingestion and the writer.
const QUEUE: usize = 10_000;
/// Delay between two attempts after a write failure.
const RETRY_PAUSE: Duration = Duration::from_secs(2);

/// Name of a device's entry.
#[must_use]
pub fn entry_name(device: DeviceId) -> String {
    format!("dev_{}", device.0)
}

/// A record to archive.
#[derive(Debug, Clone)]
pub struct IotRecord {
    pub device: DeviceId,
    pub at: DateTime<Utc>,
    /// Full state of the device after the message.
    pub state: Value,
    /// History rows of the tracked keys (the numeric ones become labels).
    pub entries: Vec<LogEntry>,
}

impl IotRecord {
    fn labels(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter_map(|e| e.value_num.map(|n| (e.key.clone(), n.to_string())))
            .collect()
    }
}

/// Acces au bucket `iot`.
pub struct ReductIot {
    client: ReductClient,
    bucket_name: String,
    quota_bytes: u64,
    bucket: Mutex<Option<Arc<Bucket>>>,
}

impl std::fmt::Debug for ReductIot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReductIot")
            .field("quota_bytes", &self.quota_bytes)
            .finish_non_exhaustive()
    }
}

impl ReductIot {
    /// No network access here: the bucket is created at the first write.
    ///
    /// # Errors
    /// Invalid URL, or HTTP client that cannot be built (for example without root certificates in the image).
    pub fn new(
        url: &str,
        token: Option<&str>,
        quota_bytes: u64,
    ) -> Result<Self, reduct_rs::ReductError> {
        Self::with_bucket(url, token, IOT_BUCKET, quota_bytes)
    }

    /// With another bucket name (tests: do not touch the `iot` bucket of the dev stack).
    ///
    /// # Errors
    /// Voir [`ReductIot::new`].
    pub fn with_bucket(
        url: &str,
        token: Option<&str>,
        bucket: &str,
        quota_bytes: u64,
    ) -> Result<Self, reduct_rs::ReductError> {
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

    /// Creates the bucket if it does not exist (FIFO quota) and keeps it.
    async fn bucket(&self) -> Result<Arc<Bucket>, reduct_rs::ReductError> {
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

    async fn forget_bucket(&self) {
        *self.bucket.lock().await = None;
    }

    /// Writes a batch (at most [`MAX_BATCH`]). Returns the number of records refused by the server.
    ///
    /// # Errors
    /// Error if the server is unreachable or refuses the whole batch.
    pub async fn write(&self, batch: &[(IotRecord, u64)]) -> Result<usize, reduct_rs::ReductError> {
        let bucket = self.bucket().await?;
        let mut request = bucket.write_record_batch();
        for (rec, ts_us) in batch {
            let mut r = Record::builder()
                .entry(entry_name(rec.device))
                .timestamp_us(*ts_us)
                .content_type("application/json")
                .data(rec.state.to_string());
            for (k, v) in rec.labels() {
                r = r.add_label(k, v);
            }
            request = request.add_record(r.build());
        }
        match request.send().await {
            Ok(failed) => Ok(failed.len()),
            Err(e) => {
                self.forget_bucket().await;
                Err(e)
            }
        }
    }

    /// The bucket (read: tests and diagnostics).
    ///
    /// # Errors
    /// Error if the server is unreachable.
    pub async fn open(&self) -> Result<Arc<Bucket>, reduct_rs::ReductError> {
        self.bucket().await
    }
}

/// Background writer: ingestion drops the records there without waiting.
#[derive(Debug, Clone)]
pub struct IotWriter {
    tx: mpsc::Sender<IotRecord>,
    dropped: Arc<AtomicU64>,
}

impl IotWriter {
    /// Starts the writing task.
    #[must_use]
    pub fn spawn(archive: ReductIot) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        tokio::spawn(run(archive, rx, Arc::clone(&dropped)));
        Self { tx, dropped }
    }

    /// Enqueues a record; if the queue is full it is dropped (and counted).
    pub fn push(&self, record: IotRecord) {
        if self.tx.try_send(record).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n.is_multiple_of(1000) {
                tracing::warn!(abandonnes = n, "ReductStore archive saturated or stopped");
            }
        }
    }

    /// Records dropped since start-up.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

async fn run(archive: ReductIot, mut rx: mpsc::Receiver<IotRecord>, dropped: Arc<AtomicU64>) {
    // Last timestamp used per entry: ReductStore refuses two records at the same instant.
    let mut last: HashMap<i64, u64> = HashMap::new();
    let mut buf = Vec::with_capacity(MAX_BATCH);
    let mut unreachable = false;
    while rx.recv_many(&mut buf, MAX_BATCH).await > 0 {
        let batch: Vec<(IotRecord, u64)> = buf
            .drain(..)
            .map(|rec| {
                let wanted = u64::try_from(rec.at.timestamp_micros()).unwrap_or(0);
                let ts = last
                    .get(&rec.device.0)
                    .map_or(wanted, |prev| wanted.max(prev + 1));
                last.insert(rec.device.0, ts);
                (rec, ts)
            })
            .collect();
        match archive.write(&batch).await {
            Ok(0) => {
                if unreachable {
                    tracing::info!("ReductStore archive restored");
                    unreachable = false;
                }
            }
            Ok(refused) => {
                dropped.fetch_add(refused as u64, Ordering::Relaxed);
                tracing::warn!(refused, "records refused by ReductStore");
            }
            Err(e) => {
                dropped.fetch_add(batch.len() as u64, Ordering::Relaxed);
                if !unreachable {
                    tracing::warn!(error = %e, "ReductStore unreachable: the raw archive is suspended");
                    unreachable = true;
                }
                tokio::time::sleep(RETRY_PAUSE).await;
            }
        }
    }
}
