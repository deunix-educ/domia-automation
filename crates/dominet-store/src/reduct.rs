// SPDX-License-Identifier: AGPL-3.0-or-later
//! Archive brute de l'historique dans ReductStore : bucket `iot`, une entree `dev_<id>` par appareil (ADR 0002 et 0028).
//!
//! Un enregistrement par message qui change au moins une cle suivie : le corps est l'etat JSON complet, une etiquette par
//! cle numerique suivie permet les requetes a seuil. PostgreSQL reste la source des graphiques ; ReductStore est facultatif et
//! **ne bloque jamais l'ingestion** : les ecritures passent par une file bornee et un ecrivain en tache de fond, qui reessaie la
//! creation du bucket tant que le serveur est injoignable et abandonne (en comptant) les enregistrements qui ne passent pas.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dominet_domain::device::{DeviceId, LogEntry};
use reduct_rs::{Bucket, QuotaType, Record, ReductClient};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

/// Nom du bucket des series brutes.
pub const IOT_BUCKET: &str = "iot";
/// Plus gros lot ecrit d'un coup (les metadonnees de lot voyagent dans les en-tetes HTTP : ADR 0002).
const MAX_BATCH: usize = 50;
/// Taille de la file entre l'ingestion et l'ecrivain.
const QUEUE: usize = 10_000;
/// Delai entre deux tentatives apres un echec d'ecriture.
const RETRY_PAUSE: Duration = Duration::from_secs(2);

/// Nom de l'entree d'un appareil.
#[must_use]
pub fn entry_name(device: DeviceId) -> String {
    format!("dev_{}", device.0)
}

/// Un enregistrement a archiver.
#[derive(Debug, Clone)]
pub struct IotRecord {
    pub device: DeviceId,
    pub at: DateTime<Utc>,
    /// Etat complet de l'appareil apres le message.
    pub state: Value,
    /// Lignes d'historique des cles suivies (les numeriques deviennent des etiquettes).
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
    /// Pas d'acces reseau ici : le bucket est cree a la premiere ecriture.
    ///
    /// # Errors
    /// URL invalide, ou client HTTP impossible a construire (par exemple sans certificats racine dans l'image).
    pub fn new(
        url: &str,
        token: Option<&str>,
        quota_bytes: u64,
    ) -> Result<Self, reduct_rs::ReductError> {
        Self::with_bucket(url, token, IOT_BUCKET, quota_bytes)
    }

    /// Avec un autre nom de bucket (tests : ne pas toucher au bucket `iot` de la stack de dev).
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

    /// Cree le bucket s'il n'existe pas (quota FIFO) et le garde.
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

    /// Ecrit un lot (au plus [`MAX_BATCH`]). Renvoie le nombre d'enregistrements refuses par le serveur.
    ///
    /// # Errors
    /// Erreur si le serveur est injoignable ou refuse le lot entier.
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

    /// Le bucket (lecture : tests et diagnostic).
    ///
    /// # Errors
    /// Erreur si le serveur est injoignable.
    pub async fn open(&self) -> Result<Arc<Bucket>, reduct_rs::ReductError> {
        self.bucket().await
    }
}

/// Ecrivain en tache de fond : l'ingestion y depose les enregistrements sans attendre.
#[derive(Debug, Clone)]
pub struct IotWriter {
    tx: mpsc::Sender<IotRecord>,
    dropped: Arc<AtomicU64>,
}

impl IotWriter {
    /// Demarre la tache d'ecriture.
    #[must_use]
    pub fn spawn(archive: ReductIot) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        tokio::spawn(run(archive, rx, Arc::clone(&dropped)));
        Self { tx, dropped }
    }

    /// Depose un enregistrement ; si la file est pleine il est abandonne (et compte).
    pub fn push(&self, record: IotRecord) {
        if self.tx.try_send(record).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n.is_multiple_of(1000) {
                tracing::warn!(abandonnes = n, "ReductStore archive saturated or stopped");
            }
        }
    }

    /// Enregistrements abandonnes depuis le demarrage.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

async fn run(archive: ReductIot, mut rx: mpsc::Receiver<IotRecord>, dropped: Arc<AtomicU64>) {
    // Dernier horodatage utilise par entree : ReductStore refuse deux enregistrements au meme instant.
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
