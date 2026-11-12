//! AES-256-GCM envelope encryption for credentials and other secrets.
//!
//! The master key is 32 bytes. In production it should be supplied by a
//! KMS/HSM-backed secret manager through `DOONCE_MASTER_KEY`; the local file
//! fallback exists for developer and single-node deployments and is created
//! with `0600` permissions on Unix.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rand::RngCore;
use zeroize::Zeroize;

use crate::error::{Result, SecurityError};

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const VERSION: &str = "v1";

#[derive(Zeroize)]
#[zeroize(drop)]
pub struct MasterKey {
    bytes: [u8; KEY_LEN],
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

impl MasterKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        MasterKey { bytes }
    }

    pub fn from_base64(value: &str) -> Result<Self> {
        let decoded = STANDARD
            .decode(value.trim())
            .or_else(|_| URL_SAFE_NO_PAD.decode(value.trim()))
            .map_err(|e| SecurityError::Encoding(e.to_string()))?;
        if decoded.len() != KEY_LEN {
            return Err(SecurityError::Key(format!(
                "master key must be {KEY_LEN} bytes"
            )));
        }
        let mut bytes = [0u8; KEY_LEN];
        bytes.copy_from_slice(&decoded);
        Ok(MasterKey { bytes })
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.bytes)
    }

    /// Loads `DOONCE_MASTER_KEY` when present, otherwise reads or creates a
    /// local key file with restrictive permissions.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if let Ok(value) = std::env::var("DOONCE_MASTER_KEY")
            && !value.trim().is_empty()
        {
            return Self::from_base64(&value);
        }
        if path.exists() {
            let value = std::fs::read_to_string(path)?;
            return Self::from_base64(value.trim());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let key = Self::generate();
        write_private(path, key.to_base64().as_bytes())?;
        Ok(key)
    }

    pub fn encryptor(&self) -> Encryptor {
        Encryptor { key: self.bytes }
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}

#[derive(Clone)]
pub struct Encryptor {
    key: [u8; KEY_LEN],
}

impl std::fmt::Debug for Encryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Encryptor(<redacted>)")
    }
}

impl Encryptor {
    /// Encrypts `plaintext` and returns `v1:<base64(nonce || ciphertext)>`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
            .map_err(|e| SecurityError::Crypto(e.to_string()))?;
        let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);
        Ok(format!("{VERSION}:{}", STANDARD.encode(combined)))
    }

    pub fn encrypt_str(&self, plaintext: &str) -> Result<String> {
        self.encrypt(plaintext.as_bytes())
    }

    pub fn decrypt(&self, token: &str) -> Result<Vec<u8>> {
        let Some((version, payload)) = token.split_once(':') else {
            return Err(SecurityError::Encoding("missing encryption version".into()));
        };
        if version != VERSION {
            return Err(SecurityError::Encoding(format!(
                "unsupported encryption version \"{version}\""
            )));
        }
        let combined = STANDARD
            .decode(payload)
            .map_err(|e| SecurityError::Encoding(e.to_string()))?;
        if combined.len() <= NONCE_LEN {
            return Err(SecurityError::Encoding("ciphertext is too short".into()));
        }
        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .map_err(|e| SecurityError::Crypto(e.to_string()))
    }

    pub fn decrypt_str(&self, token: &str) -> Result<String> {
        let bytes = self.decrypt(token)?;
        String::from_utf8(bytes).map_err(|e| SecurityError::Encoding(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_tampering() {
        let key = MasterKey::generate();
        let enc = key.encryptor();
        let token = enc.encrypt_str("hunter2").unwrap();
        assert_ne!(token, "hunter2");
        assert_eq!(enc.decrypt_str(&token).unwrap(), "hunter2");

        let mut tampered = token.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(
            enc.decrypt_str(&String::from_utf8(tampered).unwrap())
                .is_err()
        );
    }

    #[test]
    fn key_length_is_enforced() {
        assert!(MasterKey::from_base64("AAAA").is_err());
        let key = MasterKey::generate();
        assert!(MasterKey::from_base64(&key.to_base64()).is_ok());
    }

    #[test]
    fn debug_never_leaks_key_material() {
        let key = MasterKey::generate();
        let debug = format!("{key:?}");
        assert!(debug.contains("redacted"));
        assert!(!debug.contains(&key.to_base64()));
        assert!(format!("{:?}", key.encryptor()).contains("redacted"));
    }
}
