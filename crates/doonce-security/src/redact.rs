//! Secret and PII redaction for logs, events, API responses and LLM prompts.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub const MASK: &str = "••••••";

const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "apikey",
    "api_key",
    "proxy-authorization",
    "x-auth-token",
    "password",
    "passwd",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "totp",
    "otp",
    "cvv",
    "cvc",
    "card_number",
    "pan",
    "ssn",
];

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEYS
        .iter()
        .any(|k| lower == *k || lower.ends_with(&format!("_{k}")))
}

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new() -> Self {
        Redactor::default()
    }

    /// Registers a literal secret value. Values shorter than three characters
    /// are ignored to avoid accidental over-redaction.
    pub fn add_secret(&mut self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.chars().count() >= 3 && !self.secrets.contains(&secret) {
            self.secrets.push(secret);
        }
    }

    pub fn redact(&self, input: &str) -> String {
        let mut out = input.to_string();
        for secret in &self.secrets {
            out = out.replace(secret.as_str(), MASK);
        }
        out
    }

    pub fn redact_json(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.redact(s)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.redact_json(v)).collect())
            }
            Value::Object(map) => {
                let mut out = Map::new();
                for (key, value) in map {
                    if is_sensitive_key(key) {
                        out.insert(key.clone(), Value::String(MASK.into()));
                    } else {
                        out.insert(key.clone(), self.redact_json(value));
                    }
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    pub fn redact_headers(&self, headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        headers
            .iter()
            .map(|(k, v)| {
                if is_sensitive_key(k) {
                    (k.clone(), MASK.to_string())
                } else {
                    (k.clone(), self.redact(v))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_registered_secrets() {
        let mut r = Redactor::new();
        r.add_secret("hunter2");
        assert_eq!(r.redact("password is hunter2"), "password is ••••••");
        r.add_secret("xy");
        assert_eq!(r.redact("xy"), "xy", "short values are not registered");
    }

    #[test]
    fn redacts_sensitive_json_keys_recursively() {
        let r = Redactor::new();
        let value = json!({
            "username": "ada",
            "password": "hunter2",
            "nested": { "access_token": "abc", "note": "ok" },
            "items": [ { "api_key": "k" } ]
        });
        let redacted = r.redact_json(&value);
        assert_eq!(redacted["username"], "ada");
        assert_eq!(redacted["password"], MASK);
        assert_eq!(redacted["nested"]["access_token"], MASK);
        assert_eq!(redacted["nested"]["note"], "ok");
        assert_eq!(redacted["items"][0]["api_key"], MASK);
    }

    #[test]
    fn redacts_headers() {
        let r = Redactor::new();
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "Bearer abc".into());
        headers.insert("Accept".into(), "application/json".into());
        let out = r.redact_headers(&headers);
        assert_eq!(out["Authorization"], MASK);
        assert_eq!(out["Accept"], "application/json");
    }
}
