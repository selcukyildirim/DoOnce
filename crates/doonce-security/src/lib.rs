//! Security primitives shared by every DoOnce service.
//!
//! This crate owns the small, auditable pieces that must not be reimplemented
//! per component: envelope encryption, API key generation/verification,
//! authorization, secret redaction and the tamper-evident audit hash chain.

#![forbid(unsafe_code)]

pub mod apikey;
pub mod audit;
pub mod crypto;
pub mod error;
pub mod rbac;
pub mod redact;

pub use apikey::{GeneratedApiKey, generate_api_key, hash_api_key, verify_api_key};
pub use crypto::{Encryptor, MasterKey};
pub use error::{Result, SecurityError};
pub use rbac::{Permission, role_allows};
pub use redact::Redactor;
