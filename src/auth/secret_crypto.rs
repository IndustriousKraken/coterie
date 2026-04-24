//! Symmetric encryption for secrets stored in the database (SMTP
//! password, and future per-tenant tokens if we ever need them).
//!
//! Uses ChaCha20-Poly1305 (AEAD) with a key derived from the
//! application's `session_secret` via SHA-256 — so there's only one
//! master secret to manage. Each encrypted value gets a fresh random
//! 96-bit nonce, packed into the output as `nonce || ciphertext`.
//!
//! **Operational caveat**: if `session_secret` changes, everything
//! encrypted under the old secret becomes unreadable. This is
//! intentional — it matches the behavior operators would expect
//! when rotating the master key.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

const NONCE_LEN: usize = 12; // 96 bits, ChaCha20-Poly1305 standard

pub struct SecretCrypto {
    cipher: ChaCha20Poly1305,
}

impl SecretCrypto {
    /// Derive an encryption key from the application secret.
    pub fn new(app_secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(app_secret.as_bytes());
        hasher.update(b"|coterie-secret-crypto-v1"); // domain separation
        let key_bytes = hasher.finalize();
        let key = Key::from_slice(&key_bytes);
        Self {
            cipher: ChaCha20Poly1305::new(key),
        }
    }

    /// Encrypt plaintext and return a base64-encoded `nonce||ciphertext`.
    /// Empty strings round-trip as empty strings (not encrypted) so
    /// the UI can treat "" as "no value set" without ceremony.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| AppError::Internal(format!("Encryption failed: {}", e)))?;
        let mut packed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        packed.extend_from_slice(&nonce_bytes);
        packed.extend_from_slice(&ciphertext);
        Ok(B64.encode(packed))
    }

    /// Decrypt a previously-encrypted string. Empty input returns an
    /// empty string (see `encrypt`).
    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        if encoded.is_empty() {
            return Ok(String::new());
        }
        let packed = B64
            .decode(encoded)
            .map_err(|e| AppError::Internal(format!("Base64 decode failed: {}", e)))?;
        if packed.len() <= NONCE_LEN {
            return Err(AppError::Internal(
                "Encrypted value too short".to_string(),
            ));
        }
        let (nonce_bytes, ciphertext) = packed.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self.cipher.decrypt(nonce, ciphertext).map_err(|_| {
            AppError::Internal(
                "Decryption failed — check that session_secret hasn't changed".to_string(),
            )
        })?;
        String::from_utf8(plaintext)
            .map_err(|e| AppError::Internal(format!("Decrypted value not UTF-8: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let crypto = SecretCrypto::new("test-secret-please-ignore");
        let plain = "hunter2-smtp-password";
        let ct = crypto.encrypt(plain).unwrap();
        assert_ne!(ct, plain);
        assert_eq!(crypto.decrypt(&ct).unwrap(), plain);
    }

    #[test]
    fn empty_is_passthrough() {
        let crypto = SecretCrypto::new("x");
        assert_eq!(crypto.encrypt("").unwrap(), "");
        assert_eq!(crypto.decrypt("").unwrap(), "");
    }

    #[test]
    fn different_nonces_produce_different_ciphertexts() {
        let crypto = SecretCrypto::new("x");
        let a = crypto.encrypt("same plaintext").unwrap();
        let b = crypto.encrypt("same plaintext").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_key_fails() {
        let a = SecretCrypto::new("secret-a");
        let b = SecretCrypto::new("secret-b");
        let ct = a.encrypt("plaintext").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }
}
