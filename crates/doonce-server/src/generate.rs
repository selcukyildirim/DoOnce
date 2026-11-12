//! Natural-language to workflow generation.

use axum::Json;
use axum::extract::State;
use doonce_core::now_rfc3339;
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
pub struct GenerateWorkflowRequest {
    pub description: String,
}

const SYSTEM_PROMPT: &str = r#"You are a DoOnce workflow generator. Given a user's description, output a valid DoOnce workflow v2 JSON object.

Schema:
- schemaVersion: "2.0"
- name: short name
- description: one sentence
- prompt: optional natural-language prompt (becomes implicit browserTask)
- url: optional start URL
- maxSteps: optional (default 30)
- dataSchema: optional JSON Schema for structured output
- blocks: array of blocks, each with id, type, and type-specific fields, plus "next" (id of next block) or "branches".

Block types and key fields:
- browserTask: { prompt, url?, dataSchema?, maxSteps? }
- browserAction: { action: { kind: "navigate"|"click"|"fill"|"extract"|"press"|"hover"|"check", ... }, next }
- extract: { prompt, schema, list? }
- validate: { prompt, schema? }
- forLoop: { over, body, itemName?, maxIterations? }
- conditional: { condition (JSON Logic), then, else? }
- httpRequest: { method, url, headers?, body?, saveAs? }
- code: { language: "javascript", code, saveAs? }
- fileParse: { artifact, parser: "json"|"csv", saveAs? }
- textPrompt: { prompt }
- sendEmail: { to, subject, body }
- downloadFiles: { prompt }
- upload: { artifact, destination }
- login: { credentialId, totp? }
- humanApproval: { message }
- runWorkflow: { workflowId }
- wait: { condition: { urlContains?, textContains?, visible?, hidden? } }
- terminate: { status: "completed"|"failed" }
- goto: { target }

Rules:
- Every block needs a unique id.
- Last block should be terminate.
- Use browserTask for AI-driven steps, browserAction for deterministic ones.
- Output ONLY the JSON object, no markdown, no explanation.
"#;

pub async fn generate_workflow(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<GenerateWorkflowRequest>,
) -> Result<Json<Value>, crate::error::ApiError> {
    require(&principal, Permission::WorkflowWrite)?;
    // Resolve LLM config: DB default first, then env.
    let llm_config = repo::default_llm_config(&state.db, &principal.tenant_id)
        .await
        .ok()
        .flatten();
    let provider_config = if let Some(cfg) = llm_config {
        let kind = ProviderKind::parse(&cfg.provider)
            .ok_or_else(|| crate::error::ApiError::bad_request("unknown provider"))?;
        let api_key = cfg
            .api_key_encrypted
            .as_deref()
            .and_then(|enc| state.encryptor.decrypt_str(enc).ok());
        ProviderConfig {
            kind,
            model: cfg.model,
            base_url: cfg.base_url,
            api_key,
            timeout: Duration::from_secs(120),
            max_output_tokens: 4096,
            temperature: 0.2,
            vision: false,
            json_mode: true,
        }
    } else {
        ProviderConfig::from_env()
            .map_err(|e| crate::error::ApiError::bad_request(e.to_string()))?
    };
    let provider = Provider::new(provider_config)
        .map_err(|e| crate::error::ApiError::bad_request(e.to_string()))?;

    let req = CompletionRequest {
        messages: vec![
            Message::text(Role::System, SYSTEM_PROMPT),
            Message::text(Role::User, &request.description),
        ],
        response_schema: None,
    };
    let completion = provider
        .complete(&req)
        .await
        .map_err(|e| crate::error::ApiError::internal(e.to_string()))?;

    // Parse JSON from response (strip markdown fences if present).
    let text = completion.text.trim();
    let json_text = text
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let definition: Value = serde_json::from_str(json_text).map_err(|e| {
        crate::error::ApiError::bad_request(format!("LLM returned invalid JSON: {e}"))
    })?;

    // Validate using existing validator.
    // (Best-effort: just return the definition; console validates on save.)
    let now = now_rfc3339();
    let _ = now;

    Ok(Json(json!({ "definition": definition })))
}
