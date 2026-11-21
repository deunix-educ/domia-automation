// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service des reglages applicatifs : lecture typee (avec valeur par defaut), ecriture validee et chiffrement des
//! secrets, cache memoire invalide a chaque ecriture, donnees initiales (`seed`).
//!
//! - Un reglage absent de la base vaut sa **valeur par defaut** du registre : l'application fonctionne avant `seed`.
//! - Une valeur vide n'est jamais chiffree (rien a proteger) ; une valeur secrete non vide l'est toujours, avec le
//!   nom du reglage comme donnee associee.
//! - Si la cle maitresse change, les secrets existants deviennent illisibles : seuls ces reglages le sont (erreur
//!   explicite), les autres continuent de fonctionner.

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

/// Ce que l'interface voit d'un reglage : jamais la valeur d'un secret.
// Indicateurs independants, repris tels quels dans le DTO.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingView {
    pub key: &'static str,
    pub value_type: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub is_secret: bool,
    /// Valeur affichable : masquee pour un secret renseigne.
    pub value: String,
    /// Vrai si une valeur non vide est definie.
    pub is_set: bool,
    /// Vrai si la base ne contient pas ce reglage (la valeur par defaut s'applique).
    pub from_default: bool,
    /// Faux si un secret ne peut pas etre dechiffre.
    pub readable: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Resultat de `seed`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedReport {
    pub created: Vec<&'static str>,
    pub existing: Vec<&'static str>,
}

#[derive(Debug, Clone)]
struct Cached {
    stored: AppSetting,
    /// Valeur en clair (dechiffree pour un secret) ou `None` si le secret est illisible.
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

    /// Recoit le nom de chaque reglage modifie (pour que les taches concernees se reconfigurent).
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

    /// Relit toute la table et remplace le cache.
    ///
    /// # Errors
    /// Erreur de depot si la base est injoignable.
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

    /// Vide le cache : la prochaine lecture recharge depuis la base.
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

    /// Valeur typee d'un reglage connu (valeur par defaut s'il n'est pas en base).
    ///
    /// # Errors
    /// `UnknownKey`, `Unreadable` (secret non dechiffrable) ou erreur de depot.
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
        // Une valeur vide (secret non renseigne) n'a pas de type numerique : on la rend telle quelle.
        if raw.is_empty() {
            return Ok(SettingValue::Str(raw));
        }
        Ok(SettingValue::parse(&raw, spec.value_type)?)
    }

    /// Texte d'un reglage lu **sans attendre** (cache deja charge, valeur par defaut sinon) : pour du code synchrone comme
    /// l'en-tete CSP, qui ne peut pas interroger la base. `None` si le reglage est inconnu, illisible ou si le cache est
    /// en cours d'ecriture.
    #[must_use]
    pub fn cached_string(&self, key: &str) -> Option<String> {
        let spec = setting_spec(key)?;
        let guard = self.inner.cache.try_read().ok()?;
        match guard.as_ref().and_then(|m| m.get(key)) {
            Some(c) => c.plain.clone(),
            None => Some(spec.default.to_string()),
        }
    }

    /// Texte brut d'un reglage (pratique pour les URL, adresses, secrets...).
    ///
    /// # Errors
    /// Voir [`Self::get`].
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

    /// Tous les reglages connus, tries par categorie puis cle, valeurs secretes masquees.
    ///
    /// # Errors
    /// Erreur de depot si la base est injoignable.
    pub async fn list(&self) -> Result<Vec<SettingView>, SettingsError> {
        let map = self.snapshot().await?;
        let mut views: Vec<_> = SETTINGS
            .iter()
            .map(|spec| Self::view(spec, map.get(spec.key)))
            .collect();
        views.sort_by(|a, b| (a.category, a.key).cmp(&(b.category, b.key)));
        Ok(views)
    }

    /// Valeur a stocker : normalisee selon le type, chiffree si secrete et non vide.
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

    /// Dit si `raw` serait accepte pour `key` (type, regles du reglage, cle maitresse pour un secret) sans rien ecrire.
    ///
    /// # Errors
    /// `UnknownKey`, `Invalid` ou `MasterKeyMissing`, comme [`Self::set`].
    pub fn check(&self, key: &str, raw: &str) -> Result<(), SettingsError> {
        let spec = setting_spec(key).ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        self.prepare(spec, raw).map(|_| ())
    }

    /// Modifie un reglage connu : valide, chiffre si secret, enregistre, met le cache a jour et previent les abonnes.
    ///
    /// # Errors
    /// `UnknownKey`, `Invalid` (valeur incompatible avec le type), `MasterKeyMissing` ou erreur de depot.
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
        let _ = self.inner.events.send(key.to_string()); // aucun abonne n'est pas une erreur
        Ok(view)
    }

    /// Cree les reglages absents (valeur de l'environnement `lookup`, sinon valeur par defaut) sans jamais ecraser
    /// un reglage existant. Avec `dry_run`, ne fait que rapporter.
    ///
    /// # Errors
    /// `MasterKeyMissing` si un secret non vide doit etre ecrit sans cle maitresse, `Invalid` si une valeur de
    /// l'environnement ne correspond pas au type, ou erreur de depot.
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
        // 1. Tout valider et chiffrer d'abord : une erreur ne doit rien avoir ecrit.
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
        // 2. Puis ecrire (sans ecraser ce qu'un autre processus aurait cree entre-temps).
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
