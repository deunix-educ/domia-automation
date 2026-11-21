// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hachage des mots de passe (argon2id, ADR 0004).

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};

/// Cout memoire (Kio), iterations et parallelisme par defaut : 64 Mio, 3 passes, 1 voie, soit ~126 ms par
/// hachage sur l'EliteDesk i5-6500 (ADR 0004). Le minimum OWASP (19 Mio, 2 passes) ne prend que ~23 ms.
pub const DEFAULT_M_COST_KIB: u32 = 64 * 1024;
pub const DEFAULT_T_COST: u32 = 3;
pub const DEFAULT_P_COST: u32 = 1;

/// Resultat d'une verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    /// Le mot de passe correspond ; `needs_rehash` si le condensat utilise d'anciens parametres.
    Valid {
        needs_rehash: bool,
    },
    Invalid,
}

/// Hache et verifie. `verify_dummy` brule le meme temps de calcul pour un compte inconnu : la duree de
/// reponse ne revele pas si un nom d'utilisateur existe.
#[derive(Debug, Clone)]
pub struct Hasher {
    params: Params,
    dummy_hash: String,
}

impl Hasher {
    /// # Errors
    /// Retourne une erreur si les parametres argon2 sont hors bornes.
    pub fn with_params(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> anyhow::Result<Self> {
        let params = Params::new(m_cost_kib, t_cost, p_cost, None)
            .map_err(|e| anyhow::anyhow!("invalid argon2 parameters: {e}"))?;
        let mut hasher = Self {
            params,
            dummy_hash: String::new(),
        };
        hasher.dummy_hash = hasher.hash("dummy password to equalise response times")?;
        Ok(hasher)
    }

    /// Parametres par defaut.
    ///
    /// # Panics
    /// Jamais : les constantes par defaut sont valides.
    #[must_use]
    pub fn new() -> Self {
        Self::with_params(DEFAULT_M_COST_KIB, DEFAULT_T_COST, DEFAULT_P_COST)
            .expect("valid default parameters")
    }

    fn argon2(&self) -> Argon2<'static> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone())
    }

    /// Condensat au format PHC (`$argon2id$v=19$m=...`), sel aleatoire.
    ///
    /// # Errors
    /// Retourne une erreur si le hachage echoue (memoire, parametres).
    pub fn hash(&self, password: &str) -> anyhow::Result<String> {
        let hash = self
            .argon2()
            .hash_password(password.as_bytes())
            .map_err(|e| anyhow::anyhow!("hashing failed: {e}"))?;
        Ok(hash.to_string())
    }

    /// Verifie un mot de passe ; un condensat illisible compte comme invalide.
    #[must_use]
    pub fn verify(&self, password: &str, hash: &str) -> Verification {
        let Ok(parsed) = PasswordHash::new(hash) else {
            return Verification::Invalid;
        };
        if self
            .argon2()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            return Verification::Invalid;
        }
        Verification::Valid {
            needs_rehash: self.needs_rehash(&parsed),
        }
    }

    /// Calcul factice de meme cout qu'une verification reelle.
    pub fn verify_dummy(&self, password: &str) {
        let _ = self.verify(password, &self.dummy_hash);
    }

    fn needs_rehash(&self, parsed: &PasswordHash) -> bool {
        if parsed.algorithm.as_str() != "argon2id" {
            return true;
        }
        Params::try_from(parsed).map_or(true, |p| {
            p.m_cost() != self.params.m_cost()
                || p.t_cost() != self.params.t_cost()
                || p.p_cost() != self.params.p_cost()
        })
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    // Parametres legers pour que les tests restent rapides ; la calibration reelle est dans `calibrate`.
    fn light() -> Hasher {
        Hasher::with_params(1024, 1, 1).unwrap()
    }

    #[test]
    fn a_hash_verifies_only_with_the_right_password() {
        let h = light();
        let hash = h.hash("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$v=19$"), "{hash}");
        assert_eq!(
            h.verify("correct horse battery", &hash),
            Verification::Valid {
                needs_rehash: false
            }
        );
        assert_eq!(
            h.verify("Correct horse battery", &hash),
            Verification::Invalid
        );
        assert_eq!(h.verify("", &hash), Verification::Invalid);
    }

    #[test]
    fn two_hashes_of_the_same_password_differ() {
        let h = light();
        assert_ne!(
            h.hash("meme mot de passe").unwrap(),
            h.hash("meme mot de passe").unwrap(),
            "sel aleatoire"
        );
    }

    #[test]
    fn garbage_hashes_are_invalid_not_a_panic() {
        let h = light();
        for bad in [
            "",
            "pas un hash",
            "$argon2id$v=19$m=1$",
            "$2b$12$abcdefghijklmnopqrstuv",
        ] {
            assert_eq!(h.verify("x", bad), Verification::Invalid, "{bad}");
        }
    }

    #[test]
    fn old_parameters_trigger_a_rehash() {
        let weak = Hasher::with_params(1024, 1, 1).unwrap();
        let strong = Hasher::with_params(2048, 2, 1).unwrap();
        let old = weak.hash("un mot de passe valide").unwrap();
        assert_eq!(
            strong.verify("un mot de passe valide", &old),
            Verification::Valid { needs_rehash: true }
        );
        assert_eq!(
            weak.verify("un mot de passe valide", &old),
            Verification::Valid {
                needs_rehash: false
            }
        );
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        assert!(Hasher::with_params(1, 1, 1).is_err());
        assert!(Hasher::with_params(1024, 0, 1).is_err());
    }

    #[test]
    fn password_values_never_appear_in_debug() {
        let h = light();
        let dbg = format!("{h:?}");
        assert!(
            !dbg.contains("factice"),
            "le mot de passe factice ne doit pas fuiter"
        );
    }

    /// Calibration manuelle : `cargo test -p dominet-server calibrate -- --ignored --nocapture`.
    #[test]
    #[ignore = "mesure, pas un test"]
    fn calibrate() {
        for (m, t, p) in [
            (19 * 1024, 2, 1),
            (32 * 1024, 2, 1),
            (46 * 1024, 1, 1),
            (64 * 1024, 3, 1),
            (64 * 1024, 2, 2),
        ] {
            let h = Hasher::with_params(m, t, p).unwrap();
            let t0 = Instant::now();
            let n = 5;
            let mut hash = String::new();
            for _ in 0..n {
                hash = h.hash("mot de passe de calibration").unwrap();
            }
            let hash_ms = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
            let t1 = Instant::now();
            for _ in 0..n {
                let _ = h.verify("mot de passe de calibration", &hash);
            }
            println!(
                "m={:>6} Kio t={t} p={p} : hachage {hash_ms:6.1} ms, verification {:6.1} ms",
                m,
                t1.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
            );
        }
    }
}
