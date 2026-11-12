use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use doonce_core::validation::validate_workflow as validate_definition;
use doonce_core::{
    Connector, Credential, CredentialKind, TaskStatus, WorkflowDefinition, now_rfc3339,
};
use doonce_security::apikey::{generate_api_key, prefix_from_token, verify_api_key};
use doonce_security::rbac::Permission;
use doonce_store::repo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::require;
use crate::error::ApiError;
use crate::generate;
use crate::llm;
use crate::openapi;
use crate::session;
use crate::state::AppState;
use crate::state::Principal;

pub fn router(state: AppState) -> Router {
    let ui_dir = state.config.ui_dir.clone();
    let index = ui_dir.join("index.html");
    let mut app = Router::new()
        .nest("/api/v1", api_router())
        .route("/healthz", get(health))
        .route("/readyz", get(ready));

    if index.exists() {
        app = app.fallback_service(ServeDir::new(ui_dir).fallback(ServeFile::new(index)));
    } else {
        app = app.fallback(ui_placeholder);
    }

    app.layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(cors_layer(&state.config.cors_origins))
        .with_state(state)
}

fn api_router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
        .route(
            "/llm-configs",
            get(llm::list_configs).post(llm::create_config),
        )
        .route(
            "/llm-configs/{id}",
            get(llm::get_config)
                .put(llm::update_config)
                .delete(llm::delete_config),
        )
        .route("/llm-configs/{id}/default", post(llm::set_default))
        .route("/llm-configs/test", post(llm::test_config))
        .route("/workflows", get(list_workflows).post(create_workflow))
        .route("/workflows/validate", post(validate_workflow))
        .route("/workflows/generate", post(generate::generate_workflow))
        .route(
            "/workflows/{reference}",
            get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
        .route(
            "/workflows/{reference}/versions",
            get(list_workflow_versions),
        )
        .route(
            "/workflows/{reference}/runs",
            get(list_workflow_runs).post(create_workflow_run),
        )
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/cancel", post(cancel_run))
        .route("/runs/{id}/artifacts", get(list_run_artifacts))
        .route("/runs/{id}/evidence", get(download_run_evidence))
        .route("/runs/{id}/diffs", get(list_run_diffs))
        .route("/runs/{id}/diff/{baseline}", post(create_run_diff))
        .route("/tasks/{id}/artifacts", get(list_task_artifacts))
        .route("/tasks/{id}/actions", get(list_task_actions))
        .route("/artifacts/{id}", get(get_artifact))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", get(get_task))
        .route("/tasks/{id}/cancel", post(cancel_task))
        .route(
            "/credentials",
            get(list_credentials).post(create_credential),
        )
        .route("/credentials/{id}", delete(delete_credential))
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route("/schedules/{id}", delete(delete_schedule))
        .route("/schedules/{id}/enabled", patch(set_schedule_enabled))
        .route("/settings", get(list_settings).put(set_setting))
        .route("/audit", get(list_audit))
        .route("/metrics", get(metrics))
        .route("/metrics/prometheus", get(metrics_prometheus))
        .route("/approvals", get(list_approvals))
        .route("/approvals/{id}", post(resolve_approval))
        .route("/webhooks", get(list_webhooks).post(create_webhook))
        .route("/webhooks/{id}", delete(delete_webhook))
        .route("/webhooks/{id}/deliveries", get(list_webhook_deliveries))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{id}", delete(revoke_api_key))
        .route("/connectors", get(list_connectors).post(create_connector))
        .route("/connectors/{id}", delete(delete_connector))
        .route("/connector/heartbeat", post(connector_heartbeat))
        .route("/connector/jobs/claim", post(connector_claim))
        .route(
            "/connector/jobs/{id}/heartbeat",
            post(connector_job_heartbeat),
        )
        .route("/connector/jobs/{id}/complete", post(connector_complete))
        .route("/openapi.json", get(openapi_spec))
}

fn cors_layer(origins: &[String]) -> CorsLayer {
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    let mut layer = CorsLayer::new().allow_methods(Any).allow_headers([
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        HeaderName::from_static("x-api-key"),
    ]);
    if !parsed.is_empty() {
        layer = layer.allow_origin(AllowOrigin::list(parsed));
    }
    layer
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorkflowRequest {
    definition: WorkflowDefinition,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWorkflowRequest {
    definition: WorkflowDefinition,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRequest {
    #[serde(default)]
    inputs: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskRequest {
    prompt: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    navigation_goal: Option<String>,
    #[serde(default)]
    data_schema: Option<Value>,
    #[serde(default)]
    error_codes: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    max_steps: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCredentialRequest {
    name: String,
    kind: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    external_ref: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    totp_secret: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateScheduleRequest {
    workflow_id: String,
    cron: String,
    #[serde(default)]
    timezone: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetScheduleEnabledRequest {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSettingRequest {
    key: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateApiKeyRequest {
    name: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    workflow_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct ApprovalDecisionRequest {
    decision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWebhookRequest {
    url: String,
    #[serde(default)]
    secret: Option<String>,
    events: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateConnectorRequest {
    name: String,
    #[serde(default)]
    region: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorHeartbeatRequest {
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorClaimRequest {
    #[serde(default)]
    lease_secs: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorJobHeartbeatRequest {
    #[serde(default)]
    lease_secs: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorCompleteRequest {
    status: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookView {
    id: String,
    url: String,
    events: Vec<String>,
    enabled: bool,
    created_at: String,
}

impl From<doonce_core::Webhook> for WebhookView {
    fn from(webhook: doonce_core::Webhook) -> Self {
        WebhookView {
            id: webhook.id,
            url: webhook.url,
            events: webhook.events,
            enabled: webhook.enabled,
            created_at: webhook.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialView {
    id: String,
    name: String,
    kind: String,
    provider: String,
    external_ref: Option<String>,
    username: Option<String>,
    metadata: Value,
    created_at: String,
    updated_at: String,
}

impl From<Credential> for CredentialView {
    fn from(credential: Credential) -> Self {
        CredentialView {
            id: credential.id,
            name: credential.name,
            kind: credential.kind,
            provider: credential.provider,
            external_ref: credential.external_ref,
            username: credential.username,
            metadata: credential.metadata,
            created_at: credential.created_at,
            updated_at: credential.updated_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyView {
    id: String,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    last_used_at: Option<String>,
    expires_at: Option<String>,
    revoked_at: Option<String>,
    created_at: String,
}

impl From<doonce_core::ApiKeyRecord> for ApiKeyView {
    fn from(record: doonce_core::ApiKeyRecord) -> Self {
        ApiKeyView {
            id: record.id,
            name: record.name,
            prefix: record.prefix,
            scopes: record.scopes,
            last_used_at: record.last_used_at,
            expires_at: record.expires_at,
            revoked_at: record.revoked_at,
            created_at: record.created_at,
        }
    }
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "mode": state.config.deployment_mode.as_str(),
        "region": state.config.region,
    }))
}

async fn ready(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state.db.ping().await?;
    Ok(Json(json!({ "status": "ready" })))
}

async fn version(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "name": "doonce",
        "version": env!("CARGO_PKG_VERSION"),
        "schema": "2.0",
        "mode": state.config.deployment_mode.as_str(),
        "region": state.config.region,
    }))
}

async fn logout(State(state): State<AppState>, principal: Principal) -> Result<Response, ApiError> {
    if let Some(session_id) = &principal.session_id {
        let _ = repo::revoke_session(&state.db, session_id).await;
    }
    let mut response = Json(json!({ "loggedOut": true })).into_response();
    for cookie in session::clear_cookies(&state.config) {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    Ok(response)
}

async fn me(State(state): State<AppState>, principal: Principal) -> Result<Json<Value>, ApiError> {
    let tenant = repo::get_tenant(&state.db, &principal.tenant_id).await?;
    Ok(Json(json!({
        "tenant": tenant,
        "role": principal.role.as_str(),
        "userId": principal.user_id,
        "scopes": principal.scopes,
        "apiKeyId": principal.api_key_id,
        "authMethod": principal.auth_method,
        "mfaRequired": principal.auth_method == "session" && !principal.mfa_verified,
    })))
}

async fn list_workflows(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowRead)?;
    let workflows = repo::list_workflows(&state.db, &principal.tenant_id).await?;
    Ok(Json(json!({ "workflows": workflows })))
}

async fn validate_workflow(
    principal: Principal,
    Json(definition): Json<WorkflowDefinition>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowWrite)?;
    let issues = validate_definition(&definition);
    Ok(Json(
        json!({ "valid": issues.is_empty(), "issues": issues }),
    ))
}

async fn create_workflow(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateWorkflowRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::WorkflowWrite)?;
    let reason = request.reason.unwrap_or_else(|| "Created via API".into());
    let record = repo::create_workflow(
        &state.db,
        &principal.tenant_id,
        &request.definition,
        principal.user_id.as_deref(),
        &reason,
    )
    .await?;
    audit(
        &state,
        &principal,
        "workflow.created",
        &format!("workflow:{}", record.slug),
        json!({ "name": record.name, "reason": reason }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({ "workflow": record }))))
}

async fn get_workflow(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowRead)?;
    let (workflow, definition) =
        repo::current_workflow_definition(&state.db, &principal.tenant_id, &reference).await?;
    Ok(Json(
        json!({ "workflow": workflow, "definition": definition }),
    ))
}

async fn update_workflow(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
    Json(request): Json<UpdateWorkflowRequest>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowWrite)?;
    let workflow = repo::get_workflow(&state.db, &principal.tenant_id, &reference).await?;
    let reason = request.reason.unwrap_or_else(|| "Updated via API".into());
    let version = repo::create_workflow_version(
        &state.db,
        &principal.tenant_id,
        &workflow.id,
        &request.definition,
        &reason,
        principal.user_id.as_deref(),
    )
    .await?;
    audit(
        &state,
        &principal,
        "workflow.version_created",
        &format!("workflow:{}", workflow.slug),
        json!({ "version": version.version, "reason": reason }),
    )
    .await;
    Ok(Json(json!({ "workflow": workflow, "version": version })))
}

async fn delete_workflow(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowWrite)?;
    repo::delete_workflow(&state.db, &principal.tenant_id, &reference).await?;
    audit(
        &state,
        &principal,
        "workflow.deleted",
        &format!("workflow:{reference}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

async fn list_workflow_versions(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::WorkflowRead)?;
    let workflow = repo::get_workflow(&state.db, &principal.tenant_id, &reference).await?;
    let versions =
        repo::list_workflow_versions(&state.db, &principal.tenant_id, &workflow.id).await?;
    Ok(Json(json!({ "versions": versions })))
}

async fn list_runs(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let runs = repo::list_workflow_runs(
        &state.db,
        &principal.tenant_id,
        query.workflow_id.as_deref(),
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(json!({ "runs": runs })))
}

async fn list_workflow_runs(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let workflow = repo::get_workflow(&state.db, &principal.tenant_id, &reference).await?;
    let runs = repo::list_workflow_runs(
        &state.db,
        &principal.tenant_id,
        Some(&workflow.id),
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(json!({ "runs": runs })))
}

async fn create_workflow_run(
    State(state): State<AppState>,
    principal: Principal,
    Path(reference): Path<String>,
    Json(request): Json<RunRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::WorkflowRun)?;
    let workflow = repo::get_workflow(&state.db, &principal.tenant_id, &reference).await?;
    let run = repo::create_workflow_run(
        &state.db,
        &principal.tenant_id,
        &workflow.id,
        workflow.current_version,
        &request.inputs.unwrap_or_else(|| json!({})),
        "api",
        None,
        principal.user_id.as_deref(),
    )
    .await?;
    audit(
        &state,
        &principal,
        "workflow.run_queued",
        &format!("workflow:{}", workflow.slug),
        json!({ "runId": run.id }),
    )
    .await;
    Ok((StatusCode::ACCEPTED, Json(json!({ "run": run }))))
}

async fn get_run(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let run = repo::get_workflow_run(&state.db, &principal.tenant_id, &id).await?;
    let blocks = repo::list_block_runs(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({ "run": run, "blocks": blocks })))
}

async fn list_run_artifacts(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ArtifactRead)?;
    let artifacts = repo::list_artifacts_for_run(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({ "artifacts": artifacts })))
}

async fn list_task_artifacts(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ArtifactRead)?;
    let artifacts = repo::list_artifacts_for_task(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({ "artifacts": artifacts })))
}

async fn download_run_evidence(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require(&principal, Permission::RunRead)?;
    let run = repo::get_workflow_run(&state.db, &principal.tenant_id, &id).await?;
    let blocks = repo::list_block_runs(&state.db, &principal.tenant_id, &id).await?;
    let artifacts = repo::list_artifacts_for_run(&state.db, &principal.tenant_id, &id).await?;

    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        // manifest
        let manifest = json!({
            "runId": run.id,
            "workflowId": run.workflow_id,
            "status": run.status,
            "trigger": run.trigger,
            "createdAt": run.created_at,
            "startedAt": run.started_at,
            "finishedAt": run.finished_at,
            "error": run.error,
            "outputs": run.outputs,
            "tenantId": principal.tenant_id,
            "blocks": blocks,
            "artifacts": artifacts,
            "exportedAt": now_rfc3339(),
        });
        zip.start_file("manifest.json", opts)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        use std::io::Write;
        zip.write_all(
            serde_json::to_string_pretty(&manifest)
                .unwrap_or_default()
                .as_bytes(),
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        // artifacts
        let root = std::fs::canonicalize(&state.config.artifact_dir).ok();
        for artifact in &artifacts {
            if let Some(root) = &root {
                let path = root.join(&artifact.storage_key);
                if let Ok(bytes) = tokio::fs::read(&path).await {
                    let name = format!("artifacts/{}", artifact.filename);
                    if zip.start_file(&name, opts).is_ok() {
                        let _ = zip.write_all(&bytes);
                    }
                }
            }
        }
        zip.finish()
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"evidence-{}.zip\"", id),
        )
        .body(axum::body::Body::from(buf))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(response)
}

async fn list_task_actions(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::TaskRead)?;
    repo::get_task(&state.db, &principal.tenant_id, &id).await?;
    let actions = repo::list_task_actions(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({ "actions": actions })))
}

async fn get_artifact(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require(&principal, Permission::ArtifactRead)?;
    let artifact = repo::get_artifact(&state.db, &principal.tenant_id, &id).await?;
    let root = std::fs::canonicalize(&state.config.artifact_dir)
        .map_err(|_| ApiError::not_found("artifact storage is not available"))?;
    let path = std::fs::canonicalize(root.join(&artifact.storage_key))
        .map_err(|_| ApiError::not_found(format!("artifact {} file is missing", artifact.id)))?;
    if !path.starts_with(&root) {
        return Err(ApiError::forbidden("artifact path escaped storage"));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| ApiError::internal(format!("could not read artifact: {error}")))?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err(ApiError::bad_request(
            "artifact exceeds the 50 MB download limit",
        ));
    }
    let response = Response::builder()
        .header(header::CONTENT_TYPE, artifact.content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "inline; filename=\"{}\"",
                artifact.filename.replace('"', "_")
            ),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(response)
}

async fn list_run_diffs(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let diffs = repo::list_run_diffs(&state.db, &principal.tenant_id, &id).await?;
    let views: Vec<Value> = diffs
        .iter()
        .map(|(did, baseline, pct, changed, created)| {
            json!({
                "id": did,
                "baselineRunId": baseline,
                "pixelDiffPct": pct,
                "changedBytes": changed,
                "createdAt": created,
            })
        })
        .collect();
    Ok(Json(json!({ "diffs": views })))
}

async fn create_run_diff(
    State(state): State<AppState>,
    principal: Principal,
    Path((id, baseline)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    // Compare screenshot bytes between the two runs.
    let a_artifacts = repo::list_artifacts_for_run(&state.db, &principal.tenant_id, &id).await?;
    let b_artifacts =
        repo::list_artifacts_for_run(&state.db, &principal.tenant_id, &baseline).await?;
    let root = std::fs::canonicalize(&state.config.artifact_dir).ok();
    let mut a_bytes: Option<Vec<u8>> = None;
    let mut b_bytes: Option<Vec<u8>> = None;
    if let Some(root) = &root {
        for art in a_artifacts.iter().filter(|a| a.kind == "screenshot") {
            if let Ok(bytes) = tokio::fs::read(root.join(&art.storage_key)).await {
                a_bytes = Some(bytes);
                break;
            }
        }
        for art in b_artifacts.iter().filter(|a| a.kind == "screenshot") {
            if let Ok(bytes) = tokio::fs::read(root.join(&art.storage_key)).await {
                b_bytes = Some(bytes);
                break;
            }
        }
    }
    let (pct, changed) = match (a_bytes, b_bytes) {
        (Some(a), Some(b)) => {
            if a == b {
                (0.0, 0i64)
            } else {
                // Simple byte-level difference estimate.
                let len = a.len().max(b.len()).max(1) as i64;
                let mut diff: i64 = 0;
                let n = a.len().min(b.len());
                for i in 0..n {
                    if a[i] != b[i] {
                        diff += 1;
                    }
                }
                diff += (a.len() as i64 - b.len() as i64).abs();
                let pct = (diff as f64 / len as f64) * 100.0;
                (pct, diff)
            }
        }
        _ => (0.0, 0i64),
    };
    let diff_id = repo::create_run_diff(
        &state.db,
        &principal.tenant_id,
        &id,
        &baseline,
        pct,
        changed,
    )
    .await?;
    Ok(Json(
        json!({ "diffId": diff_id, "pixelDiffPct": pct, "changedBytes": changed }),
    ))
}

async fn cancel_run(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunCancel)?;
    let run = repo::get_workflow_run(&state.db, &principal.tenant_id, &id).await?;
    if matches!(run.status.as_str(), "queued" | "running") {
        let updated = repo::set_workflow_run_status(
            &state.db,
            &principal.tenant_id,
            &id,
            "cancelled",
            None,
            Some("cancelled via API"),
            None,
            Some(&now_rfc3339()),
        )
        .await?;
        let _ = repo::cancel_connector_jobs_for_run(&state.db, &principal.tenant_id, &id).await;
        audit(
            &state,
            &principal,
            "run.cancelled",
            &format!("run:{id}"),
            json!({ "status": updated.status }),
        )
        .await;
        return Ok(Json(json!({ "run": updated })));
    }
    Err(ApiError::conflict(
        "only queued or running runs can be cancelled",
    ))
}

async fn list_tasks(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::TaskRead)?;
    let tasks =
        repo::list_tasks(&state.db, &principal.tenant_id, query.limit.unwrap_or(50)).await?;
    Ok(Json(json!({ "tasks": tasks })))
}

async fn create_task(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::TaskWrite)?;
    let task = repo::create_task(
        &state.db,
        &principal.tenant_id,
        principal.user_id.as_deref(),
        &request.prompt,
        request.url.as_deref(),
        request.navigation_goal.as_deref(),
        request.data_schema.as_ref(),
        request.error_codes.as_deref().unwrap_or(&[]),
        request.model.as_deref(),
        request.max_steps.unwrap_or(30),
    )
    .await?;
    audit(
        &state,
        &principal,
        "task.created",
        &format!("task:{}", task.id),
        json!({ "prompt": task.prompt, "url": task.url }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({ "task": task }))))
}

async fn get_task(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::TaskRead)?;
    let task = repo::get_task(&state.db, &principal.tenant_id, &id).await?;
    Ok(Json(json!({ "task": task })))
}

async fn cancel_task(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::TaskWrite)?;
    let task = repo::get_task(&state.db, &principal.tenant_id, &id).await?;
    if matches!(
        task.status.as_str(),
        "queued" | "running" | "awaiting_approval"
    ) {
        let updated = repo::set_task_status(
            &state.db,
            &principal.tenant_id,
            &id,
            TaskStatus::Cancelled,
            None,
            Some("cancelled via API"),
            None,
            Some(&now_rfc3339()),
        )
        .await?;
        let _ = repo::cancel_connector_jobs_for_task(&state.db, &principal.tenant_id, &id).await;
        audit(
            &state,
            &principal,
            "task.cancelled",
            &format!("task:{id}"),
            json!({}),
        )
        .await;
        return Ok(Json(json!({ "task": updated })));
    }
    Err(ApiError::conflict(
        "only queued, running or awaiting-approval tasks can be cancelled",
    ))
}

async fn list_credentials(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::CredentialRead)?;
    let credentials: Vec<CredentialView> = repo::list_credentials(&state.db, &principal.tenant_id)
        .await?
        .into_iter()
        .map(CredentialView::from)
        .collect();
    Ok(Json(json!({ "credentials": credentials })))
}

async fn create_credential(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateCredentialRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::CredentialWrite)?;
    let kind = credential_kind(&request.kind)?;
    let provider = request.provider.as_deref().unwrap_or("local");
    if !matches!(
        provider,
        "local" | "bitwarden" | "1password" | "lastpass" | "http"
    ) {
        return Err(ApiError::bad_request(
            "provider must be local, bitwarden, 1password, lastpass or http",
        ));
    }
    let external_ref = request
        .external_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if provider == "local" {
        if request.secret.as_deref().unwrap_or_default().is_empty() {
            return Err(ApiError::bad_request("local credentials require a secret"));
        }
    } else if external_ref.is_none() {
        return Err(ApiError::bad_request(
            "external credentials require externalRef",
        ));
    }
    let secret = match request.secret.as_deref() {
        Some(secret) => state.encryptor.encrypt_str(secret)?,
        None => String::new(),
    };
    let totp_secret = match request.totp_secret {
        Some(secret) => Some(state.encryptor.encrypt_str(&secret)?),
        None => None,
    };
    let metadata = request.metadata.unwrap_or_else(|| json!({}));
    let credential = repo::create_credential_with_provider(
        &state.db,
        &principal.tenant_id,
        &request.name,
        kind,
        provider,
        external_ref,
        request.username.as_deref(),
        &secret,
        totp_secret.as_deref(),
        &metadata,
    )
    .await?;
    audit(
        &state,
        &principal,
        "credential.created",
        &format!("credential:{}", credential.id),
        json!({ "name": credential.name, "kind": credential.kind, "provider": credential.provider }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "credential": CredentialView::from(credential) })),
    ))
}

async fn delete_credential(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::CredentialWrite)?;
    repo::delete_credential(&state.db, &principal.tenant_id, &id).await?;
    audit(
        &state,
        &principal,
        "credential.deleted",
        &format!("credential:{id}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

async fn list_schedules(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ScheduleRead)?;
    let schedules = repo::list_schedules(&state.db, &principal.tenant_id).await?;
    Ok(Json(json!({ "schedules": schedules })))
}

async fn create_schedule(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::ScheduleWrite)?;
    let workflow =
        repo::get_workflow(&state.db, &principal.tenant_id, &request.workflow_id).await?;
    let schedule = repo::create_schedule(
        &state.db,
        &principal.tenant_id,
        &workflow.id,
        &request.cron,
        request.timezone.as_deref().unwrap_or("UTC"),
    )
    .await?;
    audit(
        &state,
        &principal,
        "schedule.created",
        &format!("schedule:{}", schedule.id),
        json!({ "workflowId": workflow.id, "cron": schedule.cron }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({ "schedule": schedule }))))
}

async fn set_schedule_enabled(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
    Json(request): Json<SetScheduleEnabledRequest>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ScheduleWrite)?;
    let schedule =
        repo::set_schedule_enabled(&state.db, &principal.tenant_id, &id, request.enabled).await?;
    audit(
        &state,
        &principal,
        "schedule.toggled",
        &format!("schedule:{id}"),
        json!({ "enabled": request.enabled }),
    )
    .await;
    Ok(Json(json!({ "schedule": schedule })))
}

async fn delete_schedule(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ScheduleWrite)?;
    repo::delete_schedule(&state.db, &principal.tenant_id, &id).await?;
    audit(
        &state,
        &principal,
        "schedule.deleted",
        &format!("schedule:{id}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

async fn list_settings(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::SettingsRead)?;
    let settings = repo::list_settings(&state.db, &principal.tenant_id).await?;
    let map: BTreeMap<String, String> = settings
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect();
    Ok(Json(json!({ "settings": map })))
}

async fn set_setting(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<SetSettingRequest>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::SettingsWrite)?;
    if is_sensitive_key(&request.key) {
        return Err(ApiError::bad_request(
            "secrets must be stored through the credential API, not settings",
        ));
    }
    repo::set_setting(
        &state.db,
        &principal.tenant_id,
        &request.key,
        &request.value,
    )
    .await?;
    audit(
        &state,
        &principal,
        "setting.updated",
        &format!("setting:{}", request.key),
        json!({ "key": request.key }),
    )
    .await;
    Ok(Json(json!({ "key": request.key, "value": request.value })))
}

async fn list_audit(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::AuditRead)?;
    let audit =
        repo::list_audit(&state.db, &principal.tenant_id, query.limit.unwrap_or(100)).await?;
    Ok(Json(json!({ "audit": audit })))
}

async fn metrics(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let metrics = doonce_store::metrics::snapshot(&state.db, &principal.tenant_id).await?;
    Ok(Json(json!({ "metrics": metrics })))
}

async fn metrics_prometheus(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Response, ApiError> {
    require(&principal, Permission::RunRead)?;
    let m = doonce_store::metrics::snapshot(&state.db, &principal.tenant_id).await?;
    let body = format!(
        "# HELP doonce_workflows_total Number of workflows.\n# TYPE doonce_workflows_total gauge\ndoonce_workflows_total {}\n\
         # HELP doonce_tasks_total Number of tasks.\n# TYPE doonce_tasks_total gauge\ndoonce_tasks_total {}\n\
         # HELP doonce_runs_total Workflow runs by status.\n# TYPE doonce_runs_total gauge\n\
         doonce_runs_total{{status=\"completed\"}} {}\ndoonce_runs_total{{status=\"failed\"}} {}\n\
         doonce_runs_total{{status=\"cancelled\"}} {}\ndoonce_runs_total{{status=\"running\"}} {}\n\
         doonce_runs_total{{status=\"queued\"}} {}\n\
         # HELP doonce_tasks_total_by_status Tasks by status.\n# TYPE doonce_tasks_total_by_status gauge\n\
         doonce_tasks_total_by_status{{status=\"completed\"}} {}\ndoonce_tasks_total_by_status{{status=\"failed\"}} {}\n\
         doonce_tasks_total_by_status{{status=\"running\"}} {}\ndoonce_tasks_total_by_status{{status=\"queued\"}} {}\n\
         # HELP doonce_queue_depth Queued workflow runs plus queued tasks.\n# TYPE doonce_queue_depth gauge\ndoonce_queue_depth {}\n\
         # HELP doonce_pending_approvals Pending approvals.\n# TYPE doonce_pending_approvals gauge\ndoonce_pending_approvals {}\n\
         # HELP doonce_success_rate Workflow success rate.\n# TYPE doonce_success_rate gauge\ndoonce_success_rate {}\n\
         # HELP doonce_avg_run_duration_ms Average recent run duration.\n# TYPE doonce_avg_run_duration_ms gauge\ndoonce_avg_run_duration_ms {}\n\
         # HELP doonce_artifacts_total Stored artifacts.\n# TYPE doonce_artifacts_total gauge\ndoonce_artifacts_total {}\n",
        m.workflows,
        m.tasks,
        m.runs_completed,
        m.runs_failed,
        m.runs_cancelled,
        m.runs_running,
        m.runs_queued,
        m.tasks_completed,
        m.tasks_failed,
        m.tasks_running,
        m.tasks_queued,
        m.runs_queued + m.tasks_queued,
        m.pending_approvals,
        m.success_rate,
        m.avg_run_duration_ms.unwrap_or(0.0),
        m.artifacts
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
        .body(axum::body::Body::from(body))
        .map_err(|error| ApiError::internal(error.to_string()))
}

async fn list_approvals(
    State(state): State<AppState>,
    principal: Principal,
    Query(query): Query<ApprovalQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunRead)?;
    let approvals = repo::list_approvals(
        &state.db,
        &principal.tenant_id,
        query.status.as_deref(),
        query.run_id.as_deref(),
        query.task_id.as_deref(),
        query.limit.unwrap_or(100),
    )
    .await?;
    Ok(Json(json!({ "approvals": approvals })))
}

async fn resolve_approval(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
    Json(request): Json<ApprovalDecisionRequest>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::RunCancel)?;
    let decision = match request.decision.as_str() {
        "approved" | "approve" => "approved",
        "denied" | "deny" => "denied",
        _ => return Err(ApiError::bad_request("decision must be approved or denied")),
    };
    let approval = repo::resolve_approval(
        &state.db,
        &principal.tenant_id,
        &id,
        decision,
        principal.user_id.as_deref(),
    )
    .await?;
    audit(
        &state,
        &principal,
        "approval.resolved",
        &format!("approval:{id}"),
        json!({ "decision": decision, "stepId": approval.step_id }),
    )
    .await;
    Ok(Json(json!({ "approval": approval })))
}

async fn list_webhooks(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::SettingsRead)?;
    let webhooks: Vec<WebhookView> = repo::list_webhooks(&state.db, &principal.tenant_id)
        .await?
        .into_iter()
        .map(WebhookView::from)
        .collect();
    Ok(Json(json!({ "webhooks": webhooks })))
}

async fn create_webhook(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::SettingsWrite)?;
    if !request.url.starts_with("https://") && !request.url.starts_with("http://") {
        return Err(ApiError::bad_request("webhook URL must be http(s)"));
    }
    if request.events.is_empty() {
        return Err(ApiError::bad_request("at least one event is required"));
    }
    let generated = crate::session::random_token();
    let (secret, return_secret) = match request.secret {
        Some(secret) if !secret.trim().is_empty() => (secret, None),
        _ => (generated.clone(), Some(generated)),
    };
    let encrypted = state.encryptor.encrypt_str(&secret)?;
    let webhook = repo::create_webhook(
        &state.db,
        &principal.tenant_id,
        &request.url,
        &encrypted,
        &request.events,
    )
    .await?;
    audit(
        &state,
        &principal,
        "webhook.created",
        &format!("webhook:{}", webhook.id),
        json!({ "url": webhook.url, "events": webhook.events }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "webhook": WebhookView::from(webhook), "secret": return_secret })),
    ))
}

async fn delete_webhook(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::SettingsWrite)?;
    repo::delete_webhook(&state.db, &principal.tenant_id, &id).await?;
    audit(
        &state,
        &principal,
        "webhook.deleted",
        &format!("webhook:{id}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

async fn list_webhook_deliveries(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::SettingsRead)?;
    let deliveries = repo::list_webhook_deliveries(
        &state.db,
        &principal.tenant_id,
        &id,
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(json!({ "deliveries": deliveries })))
}

async fn list_api_keys(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ApiKeyRead)?;
    let keys: Vec<ApiKeyView> = repo::list_api_keys(&state.db, &principal.tenant_id)
        .await?
        .into_iter()
        .map(ApiKeyView::from)
        .collect();
    Ok(Json(json!({ "apiKeys": keys })))
}

async fn create_api_key(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::ApiKeyWrite)?;
    let generated = generate_api_key();
    let scopes = request.scopes.unwrap_or_default();
    let record = repo::create_api_key(
        &state.db,
        &principal.tenant_id,
        principal.user_id.as_deref(),
        &request.name,
        &generated.prefix,
        &generated.hash,
        &scopes,
    )
    .await?;
    audit(
        &state,
        &principal,
        "api_key.created",
        &format!("api_key:{}", record.id),
        json!({ "name": record.name, "prefix": record.prefix }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "token": generated.token, "apiKey": ApiKeyView::from(record) })),
    ))
}

async fn revoke_api_key(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ApiKeyWrite)?;
    repo::revoke_api_key(&state.db, &principal.tenant_id, &id).await?;
    audit(
        &state,
        &principal,
        "api_key.revoked",
        &format!("api_key:{id}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "revoked": true })))
}

async fn openapi_spec() -> Json<Value> {
    Json(openapi::spec())
}

async fn ui_placeholder() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>DoOnce</title>
<style>body{font-family:system-ui;margin:3rem;background:#0b0d10;color:#e6edf3}code{background:#161b22;padding:.2rem .4rem;border-radius:4px}a{color:#58a6ff}</style>
</head><body><h1>DoOnce API is running</h1>
<p>The web console has not been built. Run <code>npm install &amp;&amp; npm run build</code> in <code>web/</code>.</p>
<p>API health: <a href="/api/v1/health">/api/v1/health</a> · OpenAPI: <a href="/api/v1/openapi.json">/api/v1/openapi.json</a></p>
</body></html>"#,
    )
}

async fn connector_auth(state: &AppState, headers: &HeaderMap) -> Result<Connector, ApiError> {
    let token = headers
        .get("x-connector-token")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_string)
        })
        .ok_or_else(|| ApiError::unauthorized("send x-connector-token or a Bearer token"))?;
    let prefix = prefix_from_token(&token)
        .ok_or_else(|| ApiError::unauthorized("malformed connector token"))?;
    let (connector, hash) = repo::find_connector_by_prefix(&state.db, prefix)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid connector token"))?;
    if connector.status != "active" {
        return Err(ApiError::forbidden("connector is disabled"));
    }
    if !verify_api_key(&token, &hash) {
        return Err(ApiError::unauthorized("invalid connector token"));
    }
    Ok(connector)
}

async fn list_connectors(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ConnectorRead)?;
    let connectors = repo::list_connectors(&state.db, &principal.tenant_id).await?;
    Ok(Json(json!({ "connectors": connectors })))
}

async fn create_connector(
    State(state): State<AppState>,
    principal: Principal,
    Json(request): Json<CreateConnectorRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require(&principal, Permission::ConnectorWrite)?;
    if request.name.trim().is_empty() {
        return Err(ApiError::bad_request("connector name must not be empty"));
    }
    let tenant = repo::get_tenant(&state.db, &principal.tenant_id).await?;
    let region = request
        .region
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| state.config.region.clone());
    if tenant.region != doonce_core::DEFAULT_REGION && tenant.region != region {
        return Err(ApiError::bad_request(format!(
            "connector region must match the tenant region {}",
            tenant.region
        )));
    }
    let generated = generate_api_key();
    let connector = repo::create_connector(
        &state.db,
        &principal.tenant_id,
        request.name.trim(),
        &generated.prefix,
        &generated.hash,
        &region,
    )
    .await?;
    audit(
        &state,
        &principal,
        "connector.created",
        &format!("connector:{}", connector.id),
        json!({ "name": connector.name, "region": connector.region }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "connector": connector, "token": generated.token })),
    ))
}

async fn delete_connector(
    State(state): State<AppState>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require(&principal, Permission::ConnectorWrite)?;
    repo::delete_connector(&state.db, &principal.tenant_id, &id).await?;
    audit(
        &state,
        &principal,
        "connector.deleted",
        &format!("connector:{id}"),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

async fn connector_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConnectorHeartbeatRequest>,
) -> Result<Json<Value>, ApiError> {
    let connector = connector_auth(&state, &headers).await?;
    let connector =
        repo::heartbeat_connector(&state.db, &connector.id, request.version.as_deref()).await?;
    let pending = repo::pending_connector_jobs(&state.db, &connector.id).await?;
    Ok(Json(
        json!({ "connector": connector, "pendingJobs": pending }),
    ))
}

async fn connector_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConnectorClaimRequest>,
) -> Result<Json<Value>, ApiError> {
    let connector = connector_auth(&state, &headers).await?;
    let job =
        repo::claim_connector_job(&state.db, &connector, request.lease_secs.unwrap_or(900)).await?;
    let pending = repo::pending_connector_jobs(&state.db, &connector.id).await?;
    Ok(Json(json!({ "job": job, "pendingJobs": pending })))
}

async fn connector_job_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ConnectorJobHeartbeatRequest>,
) -> Result<Json<Value>, ApiError> {
    let connector = connector_auth(&state, &headers).await?;
    let extended = repo::extend_connector_job(
        &state.db,
        &connector.id,
        &id,
        request.lease_secs.unwrap_or(900),
    )
    .await?;
    if !extended {
        return Err(ApiError::not_found(format!("connector job {id}")));
    }
    Ok(Json(json!({ "extended": true })))
}

async fn connector_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ConnectorCompleteRequest>,
) -> Result<Json<Value>, ApiError> {
    let connector = connector_auth(&state, &headers).await?;
    let status = request.status.trim().to_ascii_lowercase();
    if !matches!(status.as_str(), "completed" | "failed" | "cancelled") {
        return Err(ApiError::bad_request(
            "status must be completed, failed or cancelled",
        ));
    }
    let job = repo::complete_connector_job(
        &state.db,
        &connector.id,
        &id,
        &status,
        request.result.as_ref(),
        request.error.as_deref(),
    )
    .await?;
    if let Err(error) = repo::append_audit(
        &state.db,
        &job.tenant_id,
        None,
        "connector.job.completed",
        &format!("connector-job:{}", job.id),
        &json!({ "status": job.status, "connectorId": connector.id }),
    )
    .await
    {
        tracing::error!(%error, "failed to append connector job audit event");
    }
    Ok(Json(json!({ "job": job })))
}

async fn audit(
    state: &AppState,
    principal: &Principal,
    action: &str,
    target: &str,
    metadata: Value,
) {
    if let Err(error) = repo::append_audit(
        &state.db,
        &principal.tenant_id,
        principal.user_id.as_deref(),
        action,
        target,
        &metadata,
    )
    .await
    {
        tracing::error!(error = %error, action, target, "failed to append audit event");
    }
}

fn credential_kind(value: &str) -> Result<CredentialKind, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "username_password" | "username-password" | "password" => {
            Ok(CredentialKind::UsernamePassword)
        }
        "totp" => Ok(CredentialKind::Totp),
        "api_key" | "api-key" => Ok(CredentialKind::ApiKey),
        "credit_card" | "credit-card" | "card" => Ok(CredentialKind::CreditCard),
        "custom" => Ok(CredentialKind::Custom),
        _ => Err(ApiError::bad_request(
            "kind must be username_password, totp, api_key, credit_card or custom",
        )),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
