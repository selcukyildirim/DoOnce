//! Tamper-evident audit hash chain.
//!
//! Each entry commits to the previous entry's hash. Verification walks the
//! chain per tenant; any mutation, insertion or deletion breaks the chain.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Result, SecurityError};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditHashInput<'a> {
    pub tenant_id: &'a str,
    pub actor_id: Option<&'a str>,
    pub action: &'a str,
    pub target: &'a str,
    pub metadata: &'a Value,
    pub created_at: &'a str,
}

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub fn hash_entry(prev_hash: &str, input: &AuditHashInput<'_>) -> Result<String> {
    let encoded = serde_json::to_vec(input).map_err(|e| SecurityError::Hash(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(&encoded);
    Ok(hex::encode(hasher.finalize()))
}

/// Verifies a sequence of `(prev_hash, hash, input)` entries.
pub fn verify_chain(entries: &[(&str, &str, AuditHashInput<'_>)]) -> bool {
    let mut expected_prev = GENESIS_HASH;
    for (prev, hash, input) in entries {
        if *prev != expected_prev {
            return false;
        }
        let Ok(actual) = hash_entry(prev, input) else {
            return false;
        };
        if actual != *hash {
            return false;
        }
        expected_prev = hash;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input<'a>(action: &'a str, metadata: &'a Value, created_at: &'a str) -> AuditHashInput<'a> {
        AuditHashInput {
            tenant_id: "t1",
            actor_id: Some("u1"),
            action,
            target: "workflow:1",
            metadata,
            created_at,
        }
    }

    #[test]
    fn chain_detects_tampering() {
        let m1 = json!({"a": 1});
        let m2 = json!({"b": 2});
        let i1 = input("workflow.created", &m1, "2026-01-01T00:00:00Z");
        let h1 = hash_entry(GENESIS_HASH, &i1).unwrap();
        let i2 = input("workflow.updated", &m2, "2026-01-01T00:01:00Z");
        let h2 = hash_entry(&h1, &i2).unwrap();

        assert!(verify_chain(&[
            (GENESIS_HASH, h1.as_str(), i1),
            (h1.as_str(), h2.as_str(), i2)
        ]));

        let m3 = json!({"a": 999});
        let i1_bad = input("workflow.created", &m3, "2026-01-01T00:00:00Z");
        assert!(!verify_chain(&[(GENESIS_HASH, h1.as_str(), i1_bad)]));
    }

    #[test]
    fn hashes_are_stable() {
        let m = json!({"x": true});
        let i = input("task.completed", &m, "2026-01-01T00:00:00Z");
        assert_eq!(
            hash_entry(GENESIS_HASH, &i).unwrap(),
            hash_entry(GENESIS_HASH, &i).unwrap()
        );
    }
}
