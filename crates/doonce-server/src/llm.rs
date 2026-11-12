//! LLM provider configuration: CRUD, default selection and connection test.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use doonce_llm::{
    CompletionRequest, LlmProvider, Message, Provider, ProviderConfig, ProviderKind, Role,
};
use doonce_security::rbac::Permission;
use doonce_store::repo;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

use crate::auth::require;
use crate::state::{AppState, Principal};

#[derive(Debug, Deserialize)]
pub struct CreateLlmConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Value>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLlmConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Value>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TestLlmConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

pub async fn list_configs(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigRead)?;
    let configs = repo::list_llm_configs(&state.db, &principal.tenant_id).await?;
    let views: Vec<Value> = configs
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "provider": c.provider,
                "model": c.model,
                "baseUrl": c.base_url,
                "hasApiKey": c.api_key_encrypted.is_some(),
                "capabilities": c.capabilities,
                "isDefault": c.is_default,
                "createdAt": c.created_at,
                "updatedAt": c.updated_at,
            })
        })
        .collect();
    Ok(Json(json!({ "configs": views })))
}

pub async fn get_config(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigRead)?;
    let config = repo::get_llm_config(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({
        "config": {
            "id": config.id,
            "provider": config.provider,
            "model": config.model,
            "baseUrl": config.base_url,
            "hasApiKey": config.api_key_encrypted.is_some(),
            "capabilities": config.capabilities,
            "isDefault": config.is_default,
            "createdAt": config.created_at,
            "updatedAt": config.updated_at,
        }
    })))
}

pub async fn create_config(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateLlmConfigRequest>,
) -> Result<(StatusCode, Json<Value>), crate::error::ApiError> {
    require(&principal, Permission::LlmConfigWrite)?;
    ProviderKind::parse(&request.provider)
        .ok_or_else(|| crate::error::ApiError::bad_request("unknown provider"))?;
    let api_key_encrypted = match request.api_key.as_deref() {
        Some(secret) if !secret.trim().is_empty() => Some(state.encryptor.encrypt_str(secret)?),
        _ => None,
    };
    let capabilities = request.capabilities.unwrap_or_else(|| json!({}));
    let config = repo::create_llm_config(
        &state.db,
        &principal.tenant_id,
        &request.provider,
        &request.model,
        request.base_url.as_deref(),
        api_key_encrypted.as_deref(),
        &capabilities,
        request.is_default.unwrap_or(false),
    )
    .await?;
    audit_llm(&state, &principal, "llm_config.created", &config.id).await;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "config": {
                "id": config.id,
                "provider": config.provider,
                "model": config.model,
                "baseUrl": config.base_url,
                "hasApiKey": config.api_key_encrypted.is_some(),
                "capabilities": config.capabilities,
                "isDefault": config.is_default,
                "createdAt": config.created_at,
                "updatedAt": config.updated_at,
            }
        })),
    ))
}

pub async fn update_config(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
    Json(request): Json<UpdateLlmConfigRequest>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigWrite)?;
    ProviderKind::parse(&request.provider)
        .ok_or_else(|| crate::error::ApiError::bad_request("unknown provider"))?;
    let api_key_encrypted = match request.api_key.as_deref() {
        Some(secret) if !secret.trim().is_empty() => Some(state.encryptor.encrypt_str(secret)?),
        _ => None,
    };
    let capabilities = request.capabilities.unwrap_or_else(|| json!({}));
    let config = repo::update_llm_config(
        &state.db,
        &principal.tenant_id,
        &id,
        &request.provider,
        &request.model,
        request.base_url.as_deref(),
        api_key_encrypted.as_deref(),
        &capabilities,
        request.is_default.unwrap_or(false),
    )
    .await?;
    audit_llm(&state, &principal, "llm_config.updated", &config.id).await;
    Ok(Json(json!({
        "config": {
            "id": config.id,
            "provider": config.provider,
            "model": config.model,
            "baseUrl": config.base_url,
            "hasApiKey": config.api_key_encrypted.is_some(),
            "capabilities": config.capabilities,
            "isDefault": config.is_default,
            "createdAt": config.created_at,
            "updatedAt": config.updated_at,
        }
    })))
}

pub async fn delete_config(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigWrite)?;
    repo::delete_llm_config(&state.db, &principal.tenant_id, &id).await?;
    audit_llm(&state, &principal, "llm_config.deleted", &id).await;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn set_default(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigWrite)?;
    let config = repo::get_llm_config(&state.db, &principal.tenant_id, &id).await?;
    repo::update_llm_config(
        &state.db,
        &principal.tenant_id,
        &id,
        &config.provider,
        &config.model,
        config.base_url.as_deref(),
        None,
        &config.capabilities,
        true,
    )
    .await?;
    audit_llm(&state, &principal, "llm_config.default_set", &id).await;
    Ok(Json(json!({ "default": id })))
}

pub async fn test_config(
    State(_state): State<AppState>,
    principal: Principal,
    Json(request): Json<TestLlmConfigRequest>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::LlmConfigWrite)?;
    let kind = ProviderKind::parse(&request.provider)
        .ok_or_else(|| crate::error::ApiError::bad_request("unknown provider"))?;
    let config = ProviderConfig {
        kind,
        model: request.model.clone(),
        base_url: request.base_url.clone(),
        api_key: request.api_key.clone(),
        timeout: Duration::from_secs(30),
        max_output_tokens: 64,
        temperature: 0.0,
        vision: kind.supports_vision(),
        json_mode: false,
    };
    let provider = Provider::new(config)
        .map_err(|e| crate::error::ApiError::bad_request(format!("provider init failed: {e}")))?;
    let req = CompletionRequest {
        messages: vec![Message::text(Role::User, "Reply with the single word: ok")],
        response_schema: None,
    };
    match provider.complete(&req).await {
        Ok(completion) => Ok(Json(json!({
            "ok": true,
            "model": request.model,
            "reply": completion.text.trim(),
        }))),
        Err(error) => Ok(Json(json!({
            "ok": false,
            "model": request.model,
            "error": error.to_string(),
        }))),
    }
}

async fn audit_llm(state: &AppState, principal: &Principal, action: &str, target: &str) {
    let _ = repo::append_audit(
        &state.db,
        &principal.tenant_id,
        principal.user_id.as_deref(),
        action,
        &format!("llm_config:{target}"),
        &serde_json::json!({}),
    )
    .await;
}
