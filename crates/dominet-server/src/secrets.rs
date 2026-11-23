// SPDX-License-Identifier: AGPL-3.0-or-later
//! Encryption of sensitive values at rest (ADR 0011): AES-256-GCM.
//!
//! Stored format: `v1:` + base64( 96-bit nonce ‖ ciphertext ‖ authentication tag ).
//! The **associated data** (AAD) is the setting's name: a ciphertext copied into another setting's row does not
//! decrypt. The nonce is random at each encryption, so two ciphertexts of the same text differ.

use std::fmt;

use aes_gcm::aead::{Aead, Generate, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

const VERSION_PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error(
        "invalid master key: {KEY_LEN} bytes encoded in base64 expected (see `dominet gen-master-key`)"
    )]
    BadKey,
    #[error("valeur chiffree illisible (format inconnu ou tronque)")]
    BadFormat,
    #[error("dechiffrement impossible : mauvaise cle maitresse ou valeur alteree")]
    Decrypt,
}

/// AES-256 master key; `Debug` shows nothing.
#[derive(Clone)]
pub struct MasterKey([u8; KEY_LEN]);

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

impl MasterKey {
    /// # Errors
    /// `BadKey` if the text is not valid base64 of exactly 32 bytes.
    pub fn parse(b64: &str) -> Result<Self, SecretError> {
        let bytes = B64.decode(b64.trim()).map_err(|_| SecretError::BadKey)?;
        let key: [u8; KEY_LEN] = bytes.try_into().map_err(|_| SecretError::BadKey)?;
        Ok(Self(key))
    }

    /// New random key, already base64-encoded (to copy into `DOMINET_MASTER_KEY`).
    #[must_use]
    pub fn generate() -> String {
        let key = aes_gcm::Key::<Aes256Gcm>::generate();
        B64.encode(key.as_slice())
    }
}

/// Encrypts and decrypts with the master key.
#[derive(Clone)]
pub struct SecretBox {
    cipher: Aes256Gcm,
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBox(***)")
    }
}

impl SecretBox {
    #[must_use]
    pub fn new(key: &MasterKey) -> Self {
        // The length is guaranteed by `MasterKey`.
        Self {
            cipher: Aes256Gcm::new_from_slice(&key.0).expect("32-byte key"),
        }
    }

    /// Encrypts `plaintext`, bound to the name `aad` (the setting's name).
    ///
    /// # Panics
    /// If the system's random generator is unavailable (unrecoverable case).
    #[must_use]
    pub fn seal(&self, aad: &str, plaintext: &str) -> String {
        let nonce = Nonce::generate();
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .expect("AES-GCM encryption of a short text cannot fail");
        let mut blob = Vec::with_capacity(NONCE_LEN + sealed.len());
        blob.extend_from_slice(nonce.as_slice());
        blob.extend_from_slice(&sealed);
        format!("{VERSION_PREFIX}{}", B64.encode(blob))
    }

    /// # Errors
    /// `BadFormat` if the value is not in `v1:` format, `Decrypt` if the key, the associated data or the content
    /// do not match (tampered value included).
    pub fn open(&self, aad: &str, sealed: &str) -> Result<String, SecretError> {
        let encoded = sealed
            .strip_prefix(VERSION_PREFIX)
            .ok_or(SecretError::BadFormat)?;
        let blob = B64.decode(encoded).map_err(|_| SecretError::BadFormat)?;
        if blob.len() < NONCE_LEN + 16 {
            return Err(SecretError::BadFormat);
        }
        let (nonce, body) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(nonce).map_err(|_| SecretError::BadFormat)?;
        let plain = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: body,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Decrypt)?;
        String::from_utf8(plain).map_err(|_| SecretError::Decrypt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> SecretBox {
        SecretBox::new(&MasterKey::parse(&MasterKey::generate()).unwrap())
    }

    #[test]
    fn a_generated_key_parses_and_has_the_right_size() {
        let k = MasterKey::generate();
        assert_eq!(B64.decode(&k).unwrap().len(), 32);
        assert!(MasterKey::parse(&k).is_ok());
        assert!(
            MasterKey::parse(&format!("  {k}\n")).is_ok(),
            "espaces et saut de ligne tolerees"
        );
        assert_ne!(MasterKey::generate(), k, "chaque cle est differente");
    }

    #[test]
    fn bad_keys_are_rejected() {
        for bad in [
            "",
            "pas du base64 !!",
            &B64.encode([0u8; 16]),
            &B64.encode([0u8; 33]),
            "AAAA",
        ] {
            assert_eq!(
                MasterKey::parse(bad).unwrap_err(),
                SecretError::BadKey,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn round_trip_including_unicode_and_long_values() {
        let b = boxed();
        for plain in [
            "",
            "motdepasse",
            "clé Free Mobile — été 🔑",
            &"x".repeat(5000),
        ] {
            let sealed = b.seal("SMS_FREE_KEY", plain);
            assert!(sealed.starts_with("v1:"));
            assert_eq!(b.open("SMS_FREE_KEY", &sealed).unwrap(), plain);
        }
    }

    #[test]
    fn the_ciphertext_hides_the_plaintext_and_differs_each_time() {
        let b = boxed();
        let (a, c) = (
            b.seal("K", "secret-en-clair"),
            b.seal("K", "secret-en-clair"),
        );
        assert_ne!(a, c, "nonce aleatoire");
        assert!(!a.contains("secret-en-clair"));
        let raw = B64.decode(a.strip_prefix("v1:").unwrap()).unwrap();
        assert!(
            !raw.windows(6).any(|w| w == b"secret"),
            "pas de texte clair dans le blob"
        );
        assert_eq!(
            raw.len(),
            12 + "secret-en-clair".len() + 16,
            "nonce + texte + etiquette"
        );
    }

    #[test]
    fn a_ciphertext_cannot_be_moved_to_another_setting() {
        let b = boxed();
        let sealed = b.seal("SMS_FREE_KEY", "valeur");
        assert_eq!(
            b.open("AUTRE_CLE", &sealed).unwrap_err(),
            SecretError::Decrypt
        );
    }

    #[test]
    fn tampering_and_wrong_keys_are_detected() {
        let b = boxed();
        let sealed = b.seal("K", "valeur");
        let mut blob = B64.decode(sealed.strip_prefix("v1:").unwrap()).unwrap();
        for i in [0, NONCE_LEN, blob.len() - 1] {
            blob[i] ^= 0x01;
            assert_eq!(
                b.open("K", &format!("v1:{}", B64.encode(&blob)))
                    .unwrap_err(),
                SecretError::Decrypt,
                "octet {i}"
            );
            blob[i] ^= 0x01;
        }
        assert_eq!(
            boxed().open("K", &sealed).unwrap_err(),
            SecretError::Decrypt,
            "autre cle maitresse"
        );
    }

    #[test]
    fn malformed_values_are_format_errors() {
        let b = boxed();
        for bad in [
            "",
            "v1:",
            "v2:AAAA",
            "clair",
            "v1:%%%",
            &format!("v1:{}", B64.encode([0u8; 20])),
        ] {
            assert_eq!(
                b.open("K", bad).unwrap_err(),
                SecretError::BadFormat,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn keys_and_boxes_never_print_their_content() {
        let k = MasterKey::generate();
        let key = MasterKey::parse(&k).unwrap();
        assert!(!format!("{key:?} {:?}", SecretBox::new(&key)).contains(&k));
    }
}
