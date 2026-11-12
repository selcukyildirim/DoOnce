//! API key generation and verification.
//!
//! Tokens have the form `dsk_<prefix>_<secret>`. Only an Argon2 hash of the
//! full token and the non-secret prefix are stored. Prefix lookup keeps
//! verification O(1) without weakening the hash.

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::distributions::Alphanumeric;
use rand::{Rng, RngCore};

use crate::error::{Result, SecurityError};

const TOKEN_PREFIX: &str = "dsk";
const PREFIX_LEN: usize = 8;
const SECRET_BYTES: usize = 32;

#[derive(Debug, Clone)]
pub struct GeneratedApiKey {
    /// Full token. Show this exactly once and never persist it in plaintext.
    pub token: String,
    pub prefix: String,
    pub hash: String,
}

pub fn generate_api_key() -> GeneratedApiKey {
    let prefix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(PREFIX_LEN)
        .map(char::from)
        .collect::<String>()
        .to_ascii_lowercase();
    let mut secret = [0u8; SECRET_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let secret = URL_SAFE_NO_PAD.encode(secret);
    let token = format!("{TOKEN_PREFIX}_{prefix}_{secret}");
    // Argon2 with the default parameters is infallible for our inputs, but the
    // API stays fallible so callers never unwrap in a request path.
    let hash = hash_api_key(&token).expect("argon2 hashing is available");
    GeneratedApiKey {
        token,
        prefix,
        hash,
    }
}

pub fn hash_api_key(token: &str) -> Result<String> {
    if token.len() > 512 {
        return Err(SecurityError::TooLarge(
            "API key is longer than 512 bytes".into(),
        ));
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| SecurityError::Hash(e.to_string()))
}

pub fn verify_api_key(token: &str, hash: &str) -> bool {
    if token.is_empty() || token.len() > 512 || hash.is_empty() {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(token.as_bytes(), &parsed)
        .is_ok()
}

/// Extracts the lookup prefix from a token without verifying it.
pub fn prefix_from_token(token: &str) -> Option<&str> {
    let mut parts = token.split('_');
    if parts.next()? != TOKEN_PREFIX {
        return None;
    }
    let prefix = parts.next()?;
    if prefix.len() != PREFIX_LEN || parts.next().is_none() {
        return None;
    }
    Some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_verifiable() {
        let a = generate_api_key();
        let b = generate_api_key();
        assert_ne!(a.token, b.token);
        assert_eq!(prefix_from_token(&a.token), Some(a.prefix.as_str()));
        assert!(verify_api_key(&a.token, &a.hash));
        assert!(!verify_api_key(&b.token, &a.hash));
        assert!(!verify_api_key("not-a-key", &a.hash));
    }

    #[test]
    fn malformed_prefixes_are_rejected() {
        assert_eq!(prefix_from_token("dsk_short_x"), None);
        assert_eq!(prefix_from_token("other_abcdefgh_x"), None);
        assert_eq!(prefix_from_token("dsk_abcdefgh"), None);
    }

    #[test]
    fn oversized_tokens_are_rejected() {
        let big = "x".repeat(513);
        assert!(hash_api_key(&big).is_err());
        assert!(!verify_api_key(
            &big,
            "$argon2id$v=19$m=1,t=1,p=1$c2FsdA$aGFzaA"
        ));
    }
}
