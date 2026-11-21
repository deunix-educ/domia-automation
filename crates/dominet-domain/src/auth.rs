// SPDX-License-Identifier: AGPL-3.0-or-later
//! Regles d'authentification : politique de mot de passe et blocage anti brute-force.

use chrono::{DateTime, Duration, Utc};

use crate::error::ValidationError;

/// Regles sur les mots de passe choisis par un utilisateur.
#[derive(Debug, Clone, Copy)]
pub struct PasswordPolicy;

/// Motif de refus d'un mot de passe (permet a l'interface de traduire le message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordViolation {
    TooShort,
    TooLong,
    AllDigits,
    SameAsUsername,
}

impl PasswordViolation {
    /// Message technique (francais) utilise par l'API et la CLI.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Self::TooShort => format!(
                "password too short ({} characters minimum)",
                PasswordPolicy::MIN_LEN
            ),
            Self::TooLong => format!(
                "password too long ({} characters maximum)",
                PasswordPolicy::MAX_LEN
            ),
            Self::AllDigits => "the password cannot be numeric only".into(),
            Self::SameAsUsername => "the password cannot be the same as the user name".into(),
        }
    }
}

impl PasswordPolicy {
    pub const MIN_LEN: usize = 10;
    /// Borne haute : evite qu'un mot de passe geant serve a saturer le CPU (hachage).
    pub const MAX_LEN: usize = 128;

    /// Premier motif de refus, ou `None` si le mot de passe est accepte.
    #[must_use]
    pub fn violation(password: &str, username: &str) -> Option<PasswordViolation> {
        let n = password.chars().count();
        if n < Self::MIN_LEN {
            Some(PasswordViolation::TooShort)
        } else if n > Self::MAX_LEN {
            Some(PasswordViolation::TooLong)
        } else if password.chars().all(|c| c.is_ascii_digit()) {
            Some(PasswordViolation::AllDigits)
        } else if password.eq_ignore_ascii_case(username) {
            Some(PasswordViolation::SameAsUsername)
        } else {
            None
        }
    }

    /// # Errors
    /// Refuse un mot de passe trop court ou trop long, uniquement numerique, ou identique au nom d'utilisateur.
    pub fn check(password: &str, username: &str) -> Result<(), ValidationError> {
        match Self::violation(password, username) {
            Some(v) => Err(ValidationError(v.message())),
            None => Ok(()),
        }
    }
}

/// Compteurs d'echecs recents utilises pour decider d'un blocage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptStats {
    /// Echecs de ce compte depuis cette adresse IP (depuis la derniere connexion reussie).
    pub failures_pair: u32,
    /// Echecs de ce compte depuis n'importe quelle adresse (depuis la derniere connexion reussie).
    pub failures_user: u32,
    /// Echecs depuis cette adresse IP, tous comptes confondus.
    pub failures_ip: u32,
    pub last_failure_at: Option<DateTime<Utc>>,
}

/// Blocage progressif : un seuil par couple (compte, IP), un par compte (attaque distribuee) et un par IP
/// (balayage de comptes). Le blocage dure `lock` apres le dernier echec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockoutPolicy {
    pub max_failures_pair: u32,
    pub max_failures_user: u32,
    pub max_failures_ip: u32,
    /// Fenetre dans laquelle les echecs sont comptes.
    pub window: Duration,
    pub lock: Duration,
}

impl Default for LockoutPolicy {
    fn default() -> Self {
        Self {
            max_failures_pair: 5,
            max_failures_user: 20,
            max_failures_ip: 30,
            window: Duration::minutes(15),
            lock: Duration::minutes(15),
        }
    }
}

impl LockoutPolicy {
    /// Duree restante de blocage, ou `None` si la connexion est autorisee.
    #[must_use]
    pub fn retry_after(&self, stats: &AttemptStats, now: DateTime<Utc>) -> Option<Duration> {
        let over = stats.failures_pair >= self.max_failures_pair
            || stats.failures_user >= self.max_failures_user
            || stats.failures_ip >= self.max_failures_ip;
        if !over {
            return None;
        }
        let until = stats.last_failure_at? + self.lock;
        let left = until - now;
        (left > Duration::zero()).then_some(left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_violations_are_classified() {
        use PasswordViolation::{AllDigits, SameAsUsername, TooLong, TooShort};
        let v = PasswordPolicy::violation;
        assert_eq!(v("court", "bob"), Some(TooShort));
        assert_eq!(v(&"x".repeat(129), "bob"), Some(TooLong));
        assert_eq!(v("1234567890123", "bob"), Some(AllDigits));
        assert_eq!(v("Bobbobbobbob", "bobbobbobbob"), Some(SameAsUsername));
        assert_eq!(v("correct horse battery", "bob"), None);
        // Le message de l'API reste celui de la violation.
        let err = PasswordPolicy::check("court", "bob").unwrap_err();
        assert_eq!(err.0, TooShort.message());
    }

    #[test]
    fn password_policy() {
        assert!(PasswordPolicy::check("correct horse battery", "bob").is_ok());
        let cases = [
            ("court", "bob"),
            ("1234567890123", "bob"),
            ("MonNomDutilisateur", "monnomdutilisateur"),
            (&"x".repeat(129), "bob"),
        ];
        for (pw, user) in cases {
            assert!(PasswordPolicy::check(pw, user).is_err(), "{pw}");
        }
        assert!(PasswordPolicy::check(&"x".repeat(128), "bob").is_ok());
        assert!(
            PasswordPolicy::check("éééééééééé", "bob").is_ok(),
            "10 caracteres, pas 10 octets"
        );
    }

    fn stats(pair: u32, user: u32, ip: u32, ago_s: i64, now: DateTime<Utc>) -> AttemptStats {
        AttemptStats {
            failures_pair: pair,
            failures_user: user,
            failures_ip: ip,
            last_failure_at: Some(now - Duration::seconds(ago_s)),
        }
    }

    #[test]
    fn below_every_threshold_nothing_is_locked() {
        let now = Utc::now();
        assert_eq!(
            LockoutPolicy::default().retry_after(&stats(4, 19, 29, 1, now), now),
            None
        );
        assert_eq!(
            LockoutPolicy::default().retry_after(&AttemptStats::default(), now),
            None
        );
    }

    #[test]
    fn each_threshold_triggers_a_lock() {
        let now = Utc::now();
        let p = LockoutPolicy::default();
        for s in [
            stats(5, 0, 0, 60, now),
            stats(0, 20, 0, 60, now),
            stats(0, 0, 30, 60, now),
        ] {
            let left = p.retry_after(&s, now).expect("bloque");
            assert_eq!(left, Duration::minutes(15) - Duration::seconds(60));
        }
    }

    #[test]
    fn the_lock_expires_after_its_duration() {
        let now = Utc::now();
        let p = LockoutPolicy::default();
        assert!(p.retry_after(&stats(5, 5, 5, 899, now), now).is_some());
        assert_eq!(p.retry_after(&stats(5, 5, 5, 900, now), now), None);
        assert_eq!(p.retry_after(&stats(50, 50, 50, 3600, now), now), None);
    }

    #[test]
    fn a_threshold_without_a_known_failure_time_does_not_lock() {
        let s = AttemptStats {
            failures_pair: 9,
            ..AttemptStats::default()
        };
        assert_eq!(LockoutPolicy::default().retry_after(&s, Utc::now()), None);
    }
}
