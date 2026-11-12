//! Browser session cookies and CSRF helpers.

use axum::http::Method;
use axum::http::request::Parts;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use doonce_core::SessionRecord;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::config::ServerConfig;

pub const SESSION_COOKIE: &str = "doonce_session";
pub const CSRF_COOKIE: &str = "doonce_csrf";

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    let header = parts
        .headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?;
    for pair in header.split(';') {
        if let Some((key, value)) = pair.trim().split_once('=')
            && key == name
        {
            return Some(value.to_string());
        }
    }
    None
}

pub fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

pub fn csrf_valid(parts: &Parts, session: &SessionRecord) -> bool {
    if !is_mutating(&parts.method) {
        return true;
    }
    let token = parts
        .headers
        .get("x-csrf-token")
        .or_else(|| parts.headers.get("x-xsrf-token"))
        .and_then(|value| value.to_str().ok());
    token.is_some_and(|token| hash_token(token) == session.csrf_hash)
}

pub fn mfa_path_allowed(path: &str) -> bool {
    path == "/auth/me"
        || path == "/api/v1/auth/me"
        || path == "/auth/logout"
        || path == "/api/v1/auth/logout"
        || path.starts_with("/auth/mfa")
        || path.starts_with("/api/v1/auth/mfa")
}

pub fn session_cookie(token: &str, config: &ServerConfig) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        config.session_ttl_secs
    );
    if config.cookie_secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn csrf_cookie(token: &str, config: &ServerConfig) -> String {
    let mut cookie = format!(
        "{CSRF_COOKIE}={token}; Path=/; SameSite=Strict; Max-Age={}",
        config.session_ttl_secs
    );
    if config.cookie_secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_cookies(config: &ServerConfig) -> [String; 2] {
    let secure = if config.cookie_secure { "; Secure" } else { "" };
    [
        format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}"),
        format!("{CSRF_COOKIE}=; Path=/; SameSite=Strict; Max-Age=0{secure}"),
    ]
}
