//! API key and browser-session authentication with CSRF and MFA enforcement.
//!
//! API keys are looked up by prefix and verified with Argon2. Browser sessions
//! use an opaque, HttpOnly cookie whose SHA-256 hash is stored in the database.
//! Session-authenticated mutations require a CSRF header; sessions for users
//! with a confirmed TOTP factor are limited to MFA endpoints until verified.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use doonce_core::{Role, now_rfc3339};
use doonce_security::apikey::{prefix_from_token, verify_api_key};
use doonce_security::rbac::{Permission, role_allows};
use doonce_store::repo;

use crate::error::ApiError;
use crate::session;
use crate::state::{AppState, Principal};

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.dev_no_auth {
            let tenant = repo::find_tenant_by_slug(&state.db, "default")
                .await?
                .ok_or_else(|| ApiError::service_unavailable("run `doonce quickstart` first"))?;
            return Ok(Principal {
                tenant_id: tenant.id,
                user_id: None,
                role: Role::Owner,
                scopes: Vec::new(),
                api_key_id: None,
                auth_method: "dev".into(),
                session_id: None,
                mfa_verified: true,
            });
        }

        if let Some(token) = api_key_from_headers(parts) {
            return principal_from_api_key(state, token).await;
        }

        if let Some(token) = session::cookie_value(parts, session::SESSION_COOKIE) {
            return principal_from_session(state, parts, &token).await;
        }

        Err(ApiError::unauthorized(
            "send x-api-key, Authorization: Bearer <key>, or a session cookie",
        ))
    }
}

async fn principal_from_api_key(state: &AppState, token: &str) -> Result<Principal, ApiError> {
    let prefix =
        prefix_from_token(token).ok_or_else(|| ApiError::unauthorized("malformed API key"))?;
    let record = repo::find_api_key_by_prefix(&state.db, prefix)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid API key"))?;
    if record.revoked_at.is_some() {
        return Err(ApiError::unauthorized("API key was revoked"));
    }
    if let Some(expires_at) = record.expires_at.as_deref()
        && expires_at < now_rfc3339().as_str()
    {
        return Err(ApiError::unauthorized("API key expired"));
    }
    if !verify_api_key(token, &record.key_hash) {
        return Err(ApiError::unauthorized("invalid API key"));
    }

    let role = match record.user_id.as_deref() {
        Some(user_id) => {
            let user = repo::get_user(&state.db, &record.tenant_id, user_id).await?;
            if user.status != "active" {
                return Err(ApiError::unauthorized("user is not active"));
            }
            Role::parse(&user.role)
                .ok_or_else(|| ApiError::unauthorized("user has an invalid role"))?
        }
        None => Role::Viewer,
    };
    let _ = repo::mark_api_key_used(&state.db, &record.id).await;

    Ok(Principal {
        tenant_id: record.tenant_id,
        user_id: record.user_id,
        role,
        scopes: record.scopes,
        api_key_id: Some(record.id),
        auth_method: "api_key".into(),
        session_id: None,
        mfa_verified: true,
    })
}

async fn principal_from_session(
    state: &AppState,
    parts: &Parts,
    token: &str,
) -> Result<Principal, ApiError> {
    let token_hash = session::hash_token(token);
    let record = repo::find_session_by_hash(&state.db, &token_hash)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid session"))?;
    let now = now_rfc3339();
    if record.revoked_at.is_some() || record.expires_at.as_str() < now.as_str() {
        return Err(ApiError::unauthorized("session expired"));
    }
    if session::is_mutating(&parts.method) && !session::csrf_valid(parts, &record) {
        return Err(ApiError::forbidden("CSRF token missing or invalid"));
    }

    let user = repo::get_user(&state.db, &record.tenant_id, &record.user_id).await?;
    if user.status != "active" {
        return Err(ApiError::unauthorized("user is not active"));
    }
    let role = Role::parse(&user.role)
        .ok_or_else(|| ApiError::unauthorized("user has an invalid role"))?;

    if !record.mfa_verified
        && let Some(totp) = repo::get_totp(&state.db, &record.tenant_id, &record.user_id).await?
        && totp.confirmed
        && !session::mfa_path_allowed(parts.uri.path())
    {
        return Err(ApiError::forbidden("MFA verification required"));
    }

    let _ = repo::touch_session(&state.db, &record.id).await;

    Ok(Principal {
        tenant_id: record.tenant_id,
        user_id: Some(record.user_id),
        role,
        scopes: Vec::new(),
        api_key_id: None,
        auth_method: "session".into(),
        session_id: Some(record.id),
        mfa_verified: record.mfa_verified,
    })
}

fn api_key_from_headers(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
        })
        .filter(|value| !value.trim().is_empty())
}

pub fn require(principal: &Principal, permission: Permission) -> Result<(), ApiError> {
    if role_allows(principal.role, permission) {
        Ok(())
    } else {
        Err(ApiError::forbidden(format!(
            "role \"{}\" cannot perform this action",
            principal.role.as_str()
        )))
    }
}

pub fn require_scim(principal: &Principal) -> Result<(), ApiError> {
    let scoped = principal
        .scopes
        .iter()
        .any(|scope| scope == "scim" || scope == "scim:users" || scope == "*");
    let session_admin =
        principal.auth_method == "session" && matches!(principal.role, Role::Owner | Role::Admin);
    let dev = principal.auth_method == "dev";
    if scoped || session_admin || dev {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "SCIM requires an API key with the `scim` scope or an admin session",
        ))
    }
}
