// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application settings service: typed reading (with default value), validated writing and encryption of
//! secrets, in-memory cache invalidated at each write, initial data (`seed`).
//!
//! - A setting absent from the database has the registry's **default value**: the application works before `seed`.
//! - An empty value is never encrypted (nothing to protect); a non-empty secret value always is, with the
//!   setting's name as associated data.
//! - If the master key changes, the existing secrets become unreadable: only those settings are (explicit
//!   error), the others keep working.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dominet_domain::registry::{SETTINGS, SettingSpec, setting_spec};
use dominet_domain::repo::SettingRepo;
use dominet_domain::setting::{AppSetting, MASK, SettingValue};
use dominet_domain::{RepoError, ValidationError};
use dominet_store::PgSettings;
use tokio::sync::{RwLock, broadcast};

use crate::secrets::{SecretBox, SecretError};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("reglage inconnu : {0}")]
    UnknownKey(String),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error("DOMINET_MASTER_KEY absente : impossible de lire ou d'enregistrer un secret")]
    MasterKeyMissing,
    #[error("le secret {0} est illisible (mauvaise DOMINET_MASTER_KEY ?)")]
    Unreadable(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// What the interface sees of a setting: never a secret's value.
// Independent flags, taken as is in the DTO.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingView {
    pub key: &'static str,
    pub value_type: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub is_secret: bool,
    /// Displayable value: masked for a set secret.
    pub value: String,
    /// True if a non-empty value is defined.
    pub is_set: bool,
    /// True if the database does not contain this setting (the default value applies).
    pub from_default: bool,
    /// False if a secret cannot be decrypted.
    pub readable: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Result of `seed`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedReport {
    pub created: Vec<&'static str>,
    pub existing: Vec<&'static str>,
}

#[derive(Debug, Clone)]
struct Cached {
    stored: AppSetting,
    /// Clear value (decrypted for a secret) or `None` if the secret is unreadable.
    plain: Option<String>,
}

#[derive(Debug)]
struct Inner {
    repo: PgSettings,
    sealer: Option<SecretBox>,
    cache: RwLock<Option<HashMap<String, Cached>>>,
    events: broadcast::Sender<String>,
}

#[derive(Debug, Clone)]
pub struct SettingsService {
    inner: Arc<Inner>,
}

impl SettingsService {
    #[must_use]
    pub fn new(repo: PgSettings, sealer: Option<SecretBox>) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Inner {
                repo,
                sealer,
                cache: RwLock::new(None),
                events,
            }),
        }
    }

    /// Receives the name of each modified setting (so that the tasks concerned reconfigure themselves).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.events.subscribe()
    }

    fn decrypt(&self, setting: &AppSetting) -> Option<String> {
        if !setting.is_secret || setting.value.is_empty() {
            return Some(setting.value.clone());
        }
        let opened = self
            .inner
            .sealer
            .as_ref()?
            .open(&setting.key, &setting.value);
        match opened {
            Ok(plain) => Some(plain),
            Err(e) => {
                tracing::error!(key = %setting.key, error = %e, "unreadable secret");
                None
            }
        }
    }

    /// Re-reads the whole table and replaces the cache.
    ///
    /// # Errors
    /// Repository error if the database is unreachable.
    pub async fn reload(&self) -> Result<(), SettingsError> {
        let rows = self.inner.repo.list().await?;
        let map = rows
            .into_iter()
            .map(|s| {
                let plain = self.decrypt(&s);
                (s.key.clone(), Cached { stored: s, plain })
            })
            .collect();
        *self.inner.cache.write().await = Some(map);
        Ok(())
    }

    /// Empties the cache: the next read reloads from the database.
    pub async fn invalidate(&self) {
        *self.inner.cache.write().await = None;
    }

    async fn snapshot(&self) -> Result<HashMap<String, Cached>, SettingsError> {
        if let Some(map) = self.inner.cache.read().await.as_ref() {
            return Ok(map.clone());
        }
        self.reload().await?;
        Ok(self.inner.cache.read().await.clone().unwrap_or_default())
    }

    /// Typed value of a known setting (default value if it is not in the database).
    ///
    /// # Errors
    /// `UnknownKey`, `Unreadable` (undecryptable secret) or repository error.
    pub async fn get(&self, key: &str) -> Result<SettingValue, SettingsError> {
        let spec = setting_spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        let map = self.snapshot().await?;
        let raw = match map.get(key) {
            Some(c) => c
                .plain
                .clone()
                .ok_or_else(|| SettingsError::Unreadable(key.to_string()))?,
            None => spec.default.to_string(),
        };
        // An empty value (secret not set) has no numeric type: we return it as is.
        if raw.is_empty() {
            return Ok(SettingValue::Str(raw));
        }
        Ok(SettingValue::parse(&raw, spec.value_type)?)
    }

    /// Text of a setting read **without waiting** (cache already loaded, default value otherwise): for synchronous code such as
    /// the CSP header, which cannot query the database. `None` if the setting is unknown, unreadable or if the cache is
    /// being written.
    #[must_use]
    pub fn cached_string(&self, key: &str) -> Option<String> {
        let spec = setting_spec(key)?;
        let guard = self.inner.cache.try_read().ok()?;
        match guard.as_ref().and_then(|m| m.get(key)) {
            Some(c) => c.plain.clone(),
            None => Some(spec.default.to_string()),
        }
    }

    /// Plain text of a setting (handy for URLs, addresses, secrets...).
    ///
    /// # Errors
    /// See [`Self::get`].
    pub async fn get_string(&self, key: &str) -> Result<String, SettingsError> {
        Ok(self.get(key).await?.to_raw())
    }

    fn view(spec: &'static SettingSpec, cached: Option<&Cached>) -> SettingView {
        let (is_set, readable) = match cached {
            Some(c) => (!c.stored.value.is_empty(), c.plain.is_some()),
            None => (!spec.default.is_empty(), true),
        };
        let value = match cached {
            Some(c) if spec.is_secret() => {
                if c.stored.value.is_empty() {
                    String::new()
                } else {
                    MASK.to_string()
                }
            }
            Some(c) => c.stored.value.clone(),
            None => spec.default.to_string(),
        };
        SettingView {
            key: spec.key,
            value_type: spec.value_type.as_str(),
            category: spec.category,
            description: spec.description,
            is_secret: spec.is_secret(),
            value,
            is_set,
            from_default: cached.is_none(),
            readable,
            updated_at: cached.map(|c| c.stored.updated_at),
        }
    }

    /// All the known settings, sorted by category then key, secret values masked.
    ///
    /// # Errors
    /// Repository error if the database is unreachable.
    pub async fn list(&self) -> Result<Vec<SettingView>, SettingsError> {
        let map = self.snapshot().await?;
        let mut views: Vec<_> = SETTINGS
            .iter()
            .map(|spec| Self::view(spec, map.get(spec.key)))
            .collect();
        views.sort_by(|a, b| (a.category, a.key).cmp(&(b.category, b.key)));
        Ok(views)
    }

    /// Value to store: normalised according to the type, encrypted if secret and non-empty.
    fn prepare(&self, spec: &SettingSpec, raw: &str) -> Result<String, SettingsError> {
        let draft = spec.draft(raw);
        draft.validate()?;
        let normalized = if raw.trim().is_empty()
            && spec.value_type == dominet_domain::setting::SettingType::Str
        {
            String::new()
        } else {
            SettingValue::parse(raw, spec.value_type)?.to_raw()
        };
        if spec.is_secret() && !normalized.is_empty() {
            let sealer = self
                .inner
                .sealer
                .as_ref()
                .ok_or(SettingsError::MasterKeyMissing)?;
            return Ok(sealer.seal(spec.key, &normalized));
        }
        Ok(normalized)
    }

    /// Says whether `raw` would be accepted for `key` (type, setting rules, master key for a secret) without writing anything.
    ///
    /// # Errors
    /// `UnknownKey`, `Invalid` or `MasterKeyMissing`, like [`Self::set`].
    pub fn check(&self, key: &str, raw: &str) -> Result<(), SettingsError> {
        let spec = setting_spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        self.prepare(spec, raw).map(|_| ())
    }

    /// Modifies a known setting: validates, encrypts if secret, saves, updates the cache and notifies the subscribers.
    ///
    /// # Errors
    /// `UnknownKey`, `Invalid` (value incompatible with the type), `MasterKeyMissing` or repository error.
    pub async fn set(&self, key: &str, raw: &str) -> Result<SettingView, SettingsError> {
        let spec = setting_spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        let stored_value = self.prepare(spec, raw)?;
        let stored = self
            .inner
            .repo
            .upsert(&spec.draft(raw), &stored_value)
            .await?;
        let plain = self.decrypt(&stored);
        let cached = Cached { stored, plain };
        let view = Self::view(spec, Some(&cached));
        if let Some(map) = self.inner.cache.write().await.as_mut() {
            map.insert(key.to_string(), cached);
        }
        let _ = self.inner.events.send(key.to_string()); // no subscriber is not an error
        Ok(view)
    }

    /// Creates the missing settings (value from the `lookup` environment, otherwise default value) without ever overwriting
    /// an existing setting. With `dry_run`, only reports.
    ///
    /// # Errors
    /// `MasterKeyMissing` if a non-empty secret must be written without a master key, `Invalid` if an
    /// environment value does not match the type, or repository error.
    pub async fn seed(
        &self,
        lookup: impl Fn(&str) -> Option<String>,
        dry_run: bool,
    ) -> Result<SeedReport, SettingsError> {
        let existing: HashMap<String, Cached> = {
            self.reload().await?;
            self.inner.cache.read().await.clone().unwrap_or_default()
        };
        let mut report = SeedReport::default();
        // 1. Validate and encrypt everything first: an error must have written nothing.
        let mut to_write = Vec::new();
        for spec in &SETTINGS {
            if existing.contains_key(spec.key) {
                report.existing.push(spec.key);
                continue;
            }
            let raw = lookup(spec.key)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| spec.default.to_string());
            let stored_value = self.prepare(spec, &raw)?;
            to_write.push((spec, raw, stored_value));
            report.created.push(spec.key);
        }
        // 2. Then write (without overwriting what another process might have created in the meantime).
        if !dry_run {
            for (spec, raw, stored_value) in &to_write {
                self.inner
                    .repo
                    .insert_if_absent(&spec.draft(raw), stored_value)
                    .await?;
            }
            self.reload().await?;
        }
        Ok(report)
    }
}

impl From<SecretError> for SettingsError {
    fn from(_: SecretError) -> Self {
        Self::MasterKeyMissing
    }
}
