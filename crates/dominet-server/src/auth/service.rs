// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service d'authentification : connexion avec blocage, changement de mot de passe, creation de comptes.

use std::sync::Arc;

use chrono::Utc;
use dominet_domain::auth::{LockoutPolicy, PasswordPolicy};
use dominet_domain::repo::{LoginAttemptRepo, UserRepo};
use dominet_domain::user::{NewUser, Role, User, UserId, UserProfile, Username, normalize_mobile};
use dominet_domain::{RepoError, ValidationError};
use dominet_store::PgStore;
use tokio::sync::Semaphore;

use super::password::{Hasher, Verification};

/// Nombre maximal de hachages simultanes : chacun reserve 64 Mio de memoire.
const MAX_CONCURRENT_HASHES: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Nom inconnu, mot de passe faux ou compte desactive : meme reponse pour ne rien reveler.
    #[error("identifiants invalides")]
    InvalidCredentials,
    #[error("trop de tentatives")]
    Locked { retry_after_s: u64 },
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct AuthService {
    store: PgStore,
    hasher: Arc<Hasher>,
    policy: LockoutPolicy,
    permits: Arc<Semaphore>,
}

impl AuthService {
    #[must_use]
    pub fn new(store: PgStore, hasher: Hasher, policy: LockoutPolicy) -> Self {
        Self {
            store,
            hasher: Arc::new(hasher),
            policy,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_HASHES)),
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Hasher) -> T + Send + 'static,
    ) -> Result<T, AuthError> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
        let hasher = Arc::clone(&self.hasher);
        tokio::task::spawn_blocking(move || work(&hasher))
            .await
            .map_err(|e| AuthError::Internal(e.into()))
    }

    /// Refuse si le blocage anti force brute est actif pour ce compte et cette adresse.
    async fn ensure_not_locked(&self, username: &str, ip: &str) -> Result<(), AuthError> {
        let stats = self
            .store
            .attempts()
            .stats(username, ip, Utc::now() - self.policy.window)
            .await?;
        match self.policy.retry_after(&stats, Utc::now()) {
            Some(left) => Err(AuthError::Locked {
                retry_after_s: u64::try_from(left.num_seconds().max(1)).unwrap_or(1),
            }),
            None => Ok(()),
        }
    }

    /// Connexion. Ordre voulu : blocage d'abord (sans verifier le mot de passe, pour ne rien apprendre a un
    /// attaquant bloque), puis verification de duree constante, puis journal des tentatives.
    ///
    /// # Errors
    /// `Locked` si le blocage est actif, `InvalidCredentials` pour tout echec d'authentification.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        ip: &str,
    ) -> Result<User, AuthError> {
        let username = username.trim();
        let attempts = self.store.attempts();
        self.ensure_not_locked(username, ip).await?;

        let credentials =
            if username.is_empty() || password.chars().count() > PasswordPolicy::MAX_LEN {
                None
            } else {
                self.store.users().find_credentials(username).await?
            };

        let pw = password.to_string();
        let outcome = match &credentials {
            Some(c) if c.user.is_active => {
                let hash = c.password_hash.clone();
                self.blocking(move |h| h.verify(&pw, &hash)).await?
            }
            _ => {
                // Compte inconnu, desactive ou mot de passe demesure : meme travail qu'une vraie verification.
                self.blocking(move |h| h.verify_dummy(&pw)).await?;
                Verification::Invalid
            }
        };

        if let (Some(c), Verification::Valid { needs_rehash }) = (credentials, outcome) {
            attempts.record(username, ip, true).await?;
            self.store.users().touch_last_login(c.user.id).await?;
            if needs_rehash {
                self.rehash(c.user.id, password).await;
            }
            return Ok(c.user);
        }
        attempts.record(username, ip, false).await?;
        Err(AuthError::InvalidCredentials)
    }

    /// Remplace un condensat ancien par un neuf (echec journalise, sans consequence pour la connexion).
    async fn rehash(&self, id: UserId, password: &str) {
        let pw = password.to_string();
        let result = async {
            let hash = self
                .blocking(move |h| h.hash(&pw))
                .await?
                .map_err(AuthError::Internal)?;
            self.store.users().set_password_hash(id, &hash).await?;
            Ok::<(), AuthError>(())
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(error = ?e, user_id = id.0, "cannot rehash the password");
        }
    }

    /// Change le mot de passe d'un compte et **revoque toutes ses sessions**.
    ///
    /// # Errors
    /// `Invalid` si le nouveau mot de passe viole la politique.
    pub async fn set_password(
        &self,
        id: UserId,
        username: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        PasswordPolicy::check(new_password, username)?;
        let pw = new_password.to_string();
        let hash = self
            .blocking(move |h| h.hash(&pw))
            .await?
            .map_err(AuthError::Internal)?;
        self.store.users().set_password_hash(id, &hash).await?;
        self.store
            .sessions()
            .delete_for_user(id)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
        Ok(())
    }

    /// Change son propre mot de passe : exige l'ancien. Une session volee ne doit pas permettre de deviner l'ancien
    /// mot de passe sans limite : le meme blocage que la connexion s'applique (verifie avant tout calcul), et chaque
    /// verification est journalisee comme une tentative de connexion.
    ///
    /// # Errors
    /// `Locked` si le blocage est actif, `InvalidCredentials` si l'ancien mot de passe est faux, `Invalid` si le
    /// nouveau viole la politique.
    pub async fn change_own_password(
        &self,
        user: &User,
        current: &str,
        new: &str,
        ip: &str,
    ) -> Result<(), AuthError> {
        self.ensure_not_locked(&user.username, ip).await?;
        let attempts = self.store.attempts();
        let creds = self
            .store
            .users()
            .find_credentials(&user.username)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;
        let (cur, hash) = (current.to_string(), creds.password_hash);
        if !matches!(
            self.blocking(move |h| h.verify(&cur, &hash)).await?,
            Verification::Valid { .. }
        ) {
            attempts.record(&user.username, ip, false).await?;
            return Err(AuthError::InvalidCredentials);
        }
        attempts.record(&user.username, ip, true).await?;
        self.set_password(user.id, &user.username, new).await
    }

    /// Cree un compte (CLI `createsuperuser`).
    ///
    /// # Errors
    /// `Invalid` (nom, mobile, mot de passe) ou `Repo(Conflict)` si le nom existe deja.
    pub async fn create_user(
        &self,
        username: &str,
        email: &str,
        mobile: &str,
        role: Role,
        password: &str,
    ) -> Result<User, AuthError> {
        self.create_account(&NewAccount {
            username,
            email,
            first_name: "",
            last_name: "",
            mobile,
            role,
            is_active: true,
            password,
        })
        .await
    }

    /// Cree un compte complet (administration).
    ///
    /// # Errors
    /// `Invalid` (nom, mobile, mot de passe) ou `Repo(Conflict)` si le nom existe deja (casse ignoree).
    pub async fn create_account(&self, a: &NewAccount<'_>) -> Result<User, AuthError> {
        let username = Username::parse(a.username)?;
        PasswordPolicy::check(a.password, username.as_str())?;
        let mobile = normalize_mobile(a.mobile)?;
        let pw = a.password.to_string();
        let password_hash = self
            .blocking(move |h| h.hash(&pw))
            .await?
            .map_err(AuthError::Internal)?;
        let new = NewUser {
            username,
            email: a.email.trim().to_string(),
            first_name: a.first_name.trim().to_string(),
            last_name: a.last_name.trim().to_string(),
            mobile,
            role: a.role,
            is_active: a.is_active,
            password_hash,
        };
        Ok(self.store.users().create(&new).await?)
    }

    /// Modifie le profil d'un compte (administration) et, si `new_password` est fourni, le remplace en **fermant toutes
    /// les sessions du compte**. La politique du mot de passe est verifiee avant toute ecriture. Le depot refuse de
    /// retirer le dernier administrateur actif.
    ///
    /// # Errors
    /// `Invalid` (mobile, mot de passe), `Repo(NotFound)` si le compte n'existe pas, `Repo(LastAdmin)`.
    pub async fn update_account(
        &self,
        id: UserId,
        profile: &UserProfile,
        new_password: Option<&str>,
    ) -> Result<User, AuthError> {
        let users = self.store.users();
        let current = users.find_by_id(id).await?.ok_or(RepoError::NotFound)?;
        if let Some(pw) = new_password {
            PasswordPolicy::check(pw, &current.username)?;
        }
        let profile = UserProfile {
            mobile: normalize_mobile(&profile.mobile)?,
            email: profile.email.trim().to_string(),
            first_name: profile.first_name.trim().to_string(),
            last_name: profile.last_name.trim().to_string(),
            ..profile.clone()
        };
        let user = users.update_profile(id, &profile).await?;
        if let Some(pw) = new_password {
            self.set_password(id, &current.username, pw).await?;
        }
        Ok(user)
    }
}

/// Donnees d'un compte a creer depuis l'administration.
#[derive(Clone, Copy)]
pub struct NewAccount<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub first_name: &'a str,
    pub last_name: &'a str,
    pub mobile: &'a str,
    pub role: Role,
    pub is_active: bool,
    pub password: &'a str,
}

// `Debug` ne doit jamais montrer le mot de passe.
impl std::fmt::Debug for NewAccount<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewAccount")
            .field("username", &self.username)
            .field("role", &self.role)
            .field("is_active", &self.is_active)
            .finish_non_exhaustive()
    }
}
