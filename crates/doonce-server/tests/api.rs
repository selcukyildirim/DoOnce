use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use doonce_core::Role;
use doonce_security::MasterKey;
use doonce_security::apikey::generate_api_key;
use doonce_server::config::ServerConfig;
use doonce_server::session;
use doonce_server::state::AppState;
use doonce_store::{Database, repo};
use serde_json::{Value, json};
use totp_rs::{Algorithm, Secret, TOTP};
use tower::ServiceExt;

struct TestApp {
    router: Router,
    token: String,
    tenant_id: String,
    user_id: String,
    db: Database,
    artifact_dir: std::path::PathBuf,
}

async fn setup() -> TestApp {
    let db = Database::open_in_memory().await.expect("database");
    let tenant = repo::create_tenant(&db, "Test", "test")
        .await
        .expect("tenant");
    let user = repo::create_user(&db, &tenant.id, "owner@test", "Owner", Role::Owner)
        .await
        .expect("user");
    let key = generate_api_key();
    repo::create_api_key(
        &db,
        &tenant.id,
        Some(&user.id),
        "test",
        &key.prefix,
        &key.hash,
        &[],
    )
    .await
    .expect("api key");

    let dir = tempfile::tempdir().expect("tempdir").keep();
    let artifact_dir = dir.join("artifacts");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    let config = ServerConfig {
        ui_dir: dir.join("missing-ui"),
        artifact_dir: artifact_dir.clone(),
        dev_no_auth: false,
        ..ServerConfig::default()
    };
    let state = AppState::new(db.clone(), MasterKey::generate().encryptor(), config);
    TestApp {
        router: doonce_server::build_router(state),
        token: key.token,
        tenant_id: tenant.id,
        user_id: user.id,
        db,
        artifact_dir,
    }
}

async fn request(
    app: &TestApp,
    method: &str,
    uri: &str,
    headers: Vec<(&str, String)>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let body = body
        .map(|value| Body::from(value.to_string()))
        .unwrap_or_else(Body::empty);
    let response = app
        .router
        .clone()
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn call(
    app: &TestApp,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let headers = token
        .map(|token| vec![("x-api-key", token.to_string())])
        .unwrap_or_default();
    request(app, method, &format!("/api/v1{path}"), headers, body).await
}

async fn call_cookie(
    app: &TestApp,
    method: &str,
    path: &str,
    token: &str,
    csrf: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut headers = vec![("cookie", format!("doonce_session={token}"))];
    if let Some(csrf) = csrf {
        headers.push(("x-csrf-token", csrf.to_string()));
    }
    request(app, method, &format!("/api/v1{path}"), headers, body).await
}

async fn call_scim(
    app: &TestApp,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    request(
        app,
        method,
        &format!("/scim/v2{path}"),
        vec![("authorization", format!("Bearer {token}"))],
        body,
    )
    .await
}

async fn create_browser_session(app: &TestApp, mfa_verified: bool) -> (String, String) {
    let token = session::random_token();
    let csrf = session::random_token();
    let expires = (Utc::now() + ChronoDuration::hours(1)).to_rfc3339();
    repo::create_session(
        &app.db,
        &app.tenant_id,
        &app.user_id,
        &session::hash_token(&token),
        &session::hash_token(&csrf),
        mfa_verified,
        None,
        None,
        &expires,
    )
    .await
    .expect("session");
    (token, csrf)
}

fn current_code(secret: &str) -> String {
    let secret = Secret::Encoded(secret.to_string());
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_bytes().expect("secret bytes"),
        Some("DoOnce".to_string()),
        "owner@test".to_string(),
    )
    .expect("totp");
    totp.generate_current().expect("code")
}

fn workflow() -> Value {
    json!({
        "schemaVersion": "2.0",
        "name": "Test workflow",
        "blocks": [
            {
                "id": "open",
                "type": "browserAction",
                "action": { "kind": "navigate", "url": "https://example.com/" },
                "next": "done"
            },
            { "id": "done", "type": "terminate", "status": "completed" }
        ]
    })
}

#[tokio::test]
async fn rejects_missing_or_invalid_api_keys() {
    let app = setup().await;
    let (status, problem) = call(&app, "GET", "/auth/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem["title"], "Unauthorized");

    let (status, _) = call(&app, "GET", "/auth/me", Some("dsk_bad_bad"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, me) = call(&app, "GET", "/auth/me", Some(&app.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["role"], "owner");
    assert_eq!(me["tenant"]["slug"], "test");
}

#[tokio::test]
async fn workflow_crud_and_validation() {
    let app = setup().await;
    let definition = workflow();

    let (status, created) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": definition, "reason": "test" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let slug = created["workflow"]["slug"].as_str().unwrap().to_string();
    assert_eq!(slug, "test-workflow");

    let (status, list) = call(&app, "GET", "/workflows", Some(&app.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["workflows"].as_array().unwrap().len(), 1);

    let (status, detail) = call(
        &app,
        "GET",
        &format!("/workflows/{slug}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["definition"]["blocks"].as_array().unwrap().len(), 2);

    let (status, version) = call(
        &app,
        "PUT",
        &format!("/workflows/{slug}"),
        Some(&app.token),
        Some(json!({ "definition": definition, "reason": "edit" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{version}");
    assert_eq!(version["version"]["version"], 2);

    let (status, validation) = call(
        &app,
        "POST",
        "/workflows/validate",
        Some(&app.token),
        Some(definition.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(validation["valid"], true);

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/workflows/{slug}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "GET",
        &format!("/workflows/{slug}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tenant_isolation_is_enforced() {
    let app = setup().await;
    let (status, created) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": workflow() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let slug = created["workflow"]["slug"].as_str().unwrap().to_string();

    let other = repo::create_tenant(&app.db, "Other", "other")
        .await
        .unwrap();
    let other_user = repo::create_user(&app.db, &other.id, "owner@other", "Owner", Role::Owner)
        .await
        .unwrap();
    let other_key = generate_api_key();
    repo::create_api_key(
        &app.db,
        &other.id,
        Some(&other_user.id),
        "other",
        &other_key.prefix,
        &other_key.hash,
        &[],
    )
    .await
    .unwrap();

    let (status, list) = call(&app, "GET", "/workflows", Some(&other_key.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["workflows"].as_array().unwrap().len(), 0);

    let (status, _) = call(
        &app,
        "GET",
        &format!("/workflows/{slug}"),
        Some(&other_key.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(&app, "GET", "/auth/me", Some(&other_key.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(app.tenant_id, other.id);
}

#[tokio::test]
async fn credentials_are_write_only() {
    let app = setup().await;
    let secret = "hunter2-super-secret";
    let (status, created) = call(
        &app,
        "POST",
        "/credentials",
        Some(&app.token),
        Some(json!({
            "name": "Portal",
            "kind": "username_password",
            "username": "ada",
            "secret": secret
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(!created.to_string().contains(secret));
    assert!(created["credential"].get("secretEncrypted").is_none());

    let (status, list) = call(&app, "GET", "/credentials", Some(&app.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!list.to_string().contains(secret));
    assert_eq!(list["credentials"][0]["username"], "ada");
}

#[tokio::test]
async fn tasks_schedules_and_api_keys() {
    let app = setup().await;

    let (status, task) = call(
        &app,
        "POST",
        "/tasks",
        Some(&app.token),
        Some(json!({
            "prompt": "Find the top title",
            "url": "https://example.com/",
            "dataSchema": { "type": "object", "properties": { "title": { "type": "string" } } }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["task"]["id"].as_str().unwrap().to_string();

    let (status, cancelled) = call(
        &app,
        "POST",
        &format!("/tasks/{task_id}/cancel"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["task"]["status"], "cancelled");

    let (status, created) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": workflow() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let workflow_id = created["workflow"]["id"].as_str().unwrap().to_string();
    let slug = created["workflow"]["slug"].as_str().unwrap().to_string();

    let (status, schedule) = call(
        &app,
        "POST",
        "/schedules",
        Some(&app.token),
        Some(json!({ "workflowId": workflow_id, "cron": "0 9 * * *", "timezone": "UTC" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["schedule"]["id"].as_str().unwrap().to_string();

    let (status, toggled) = call(
        &app,
        "PATCH",
        &format!("/schedules/{schedule_id}/enabled"),
        Some(&app.token),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(toggled["schedule"]["enabled"], false);

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/schedules/{schedule_id}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, queued) = call(
        &app,
        "POST",
        &format!("/workflows/{slug}/runs"),
        Some(&app.token),
        Some(json!({ "inputs": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    assert_eq!(queued["run"]["status"], "queued");

    let (status, key) = call(
        &app,
        "POST",
        "/api-keys",
        Some(&app.token),
        Some(json!({ "name": "ci" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{key}");
    let token = key["token"].as_str().unwrap().to_string();
    let key_id = key["apiKey"]["id"].as_str().unwrap().to_string();
    assert!(token.starts_with("dsk_"));

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api-keys/{key_id}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn settings_reject_secrets_and_audit_records_actions() {
    let app = setup().await;
    let (status, _) = call(
        &app,
        "PUT",
        "/settings",
        Some(&app.token),
        Some(json!({ "key": "ai.api_key", "value": "should-not-store" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &app,
        "PUT",
        "/settings",
        Some(&app.token),
        Some(json!({ "key": "server.bind", "value": "127.0.0.1:8080" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, audit) = call(&app, "GET", "/audit", Some(&app.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        audit["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["action"] == "setting.updated")
    );
}

#[tokio::test]
async fn session_auth_requires_csrf_for_mutations() {
    let app = setup().await;
    let (session_token, csrf_token) = create_browser_session(&app, true).await;

    let (status, me) = call_cookie(&app, "GET", "/auth/me", &session_token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["authMethod"], "session");
    assert_eq!(me["mfaRequired"], false);

    let (status, _) = call_cookie(
        &app,
        "PUT",
        "/settings",
        &session_token,
        None,
        Some(json!({ "key": "server.bind", "value": "127.0.0.1:9999" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call_cookie(
        &app,
        "PUT",
        "/settings",
        &session_token,
        Some(&csrf_token),
        Some(json!({ "key": "server.bind", "value": "127.0.0.1:9999" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn external_credentials_store_only_references() {
    let app = setup().await;
    let (status, created) = call(
        &app,
        "POST",
        "/credentials",
        Some(&app.token),
        Some(json!({
            "name": "HTTP vault",
            "kind": "username_password",
            "provider": "http",
            "externalRef": "item-1",
            "username": "ada"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["credential"]["provider"], "http");
    assert_eq!(created["credential"]["externalRef"], "item-1");
    assert!(created["credential"].get("secretEncrypted").is_none());

    let (status, list) = call(&app, "GET", "/credentials", Some(&app.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["credentials"][0]["provider"], "http");
    assert_eq!(list["credentials"][0]["externalRef"], "item-1");
}

#[tokio::test]
async fn approvals_and_webhooks_are_available_over_the_api() {
    let app = setup().await;
    let (status, created) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": workflow() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let slug = created["workflow"]["slug"].as_str().unwrap().to_string();
    let workflow_id = created["workflow"]["id"].as_str().unwrap().to_string();

    let (status, queued) = call(
        &app,
        "POST",
        &format!("/workflows/{slug}/runs"),
        Some(&app.token),
        Some(json!({ "inputs": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let run_id = queued["run"]["id"].as_str().unwrap().to_string();

    let approval = repo::create_approval(
        &app.db,
        &app.tenant_id,
        Some(&run_id),
        None,
        "step-1",
        "Delete account",
        "This looks destructive",
    )
    .await
    .unwrap();
    let (status, pending) = call(
        &app,
        "GET",
        "/approvals?status=pending",
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    assert_eq!(pending["approvals"].as_array().unwrap().len(), 1);

    let (status, resolved) = call(
        &app,
        "POST",
        &format!("/approvals/{}", approval.id),
        Some(&app.token),
        Some(json!({ "decision": "approved" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["approval"]["status"], "approved");

    let (status, webhook) = call(
        &app,
        "POST",
        "/webhooks",
        Some(&app.token),
        Some(json!({
            "url": "https://example.com/hook",
            "events": ["run.completed"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{webhook}");
    let webhook_id = webhook["webhook"]["id"].as_str().unwrap().to_string();
    assert!(webhook["secret"].as_str().is_some());
    assert!(webhook["webhook"].get("secretEncrypted").is_none());

    let (status, deliveries) = call(
        &app,
        "GET",
        &format!("/webhooks/{webhook_id}/deliveries"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deliveries}");
    assert_eq!(deliveries["deliveries"].as_array().unwrap().len(), 0);

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/webhooks/{webhook_id}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!workflow_id.is_empty());
}

#[tokio::test]
async fn artifacts_are_listed_and_downloaded_with_auth() {
    let app = setup().await;
    let (status, created) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": workflow() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let workflow_id = created["workflow"]["id"].as_str().unwrap().to_string();
    let run = repo::create_workflow_run(
        &app.db,
        &app.tenant_id,
        &workflow_id,
        1,
        &json!({}),
        "test",
        None,
        None,
    )
    .await
    .unwrap();
    let relative = format!("{}/artifact.png", run.id);
    let path = app.artifact_dir.join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
    let artifact = repo::create_artifact(
        &app.db,
        &app.tenant_id,
        Some(&run.id),
        None,
        None,
        doonce_core::ArtifactKind::Screenshot,
        "artifact.png",
        "image/png",
        8,
        &relative,
        "hash",
    )
    .await
    .unwrap();

    let (status, list) = call(
        &app,
        "GET",
        &format!("/runs/{}/artifacts", run.id),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["artifacts"][0]["id"], artifact.id);

    let (status, _) = call(
        &app,
        "GET",
        &format!("/artifacts/{}", artifact.id),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "GET",
        &format!("/artifacts/{}", artifact.id),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn outbound_connector_claims_and_completes_workflow_runs() {
    let app = setup().await;

    let (status, created) = call(
        &app,
        "POST",
        "/connectors",
        Some(&app.token),
        Some(json!({ "name": "vpc-connector", "region": "global" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["token"].as_str().unwrap().to_string();
    let connector_id = created["connector"]["id"].as_str().unwrap().to_string();

    let headers = vec![("x-connector-token", token.clone())];
    let (status, heartbeat) = request(
        &app,
        "POST",
        "/api/v1/connector/heartbeat",
        headers.clone(),
        Some(json!({ "version": "test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{heartbeat}");
    assert_eq!(heartbeat["connector"]["id"], connector_id);

    let (status, workflow) = call(
        &app,
        "POST",
        "/workflows",
        Some(&app.token),
        Some(json!({ "definition": workflow() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let slug = workflow["workflow"]["slug"].as_str().unwrap().to_string();

    let (status, _) = call(
        &app,
        "PUT",
        "/settings",
        Some(&app.token),
        Some(json!({ "key": "execution.connector_id", "value": connector_id })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, run) = call(
        &app,
        "POST",
        &format!("/workflows/{slug}/runs"),
        Some(&app.token),
        Some(json!({ "inputs": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{run}");
    let run_id = run["run"]["id"].as_str().unwrap().to_string();

    let (status, claimed) = request(
        &app,
        "POST",
        "/api/v1/connector/jobs/claim",
        headers.clone(),
        Some(json!({ "leaseSecs": 60 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let job_id = claimed["job"]["id"].as_str().unwrap().to_string();
    assert_eq!(claimed["job"]["runId"], run_id);
    assert_eq!(claimed["job"]["payload"]["kind"], "workflow");

    let (status, completed) = request(
        &app,
        "POST",
        &format!("/api/v1/connector/jobs/{job_id}/complete"),
        headers,
        Some(json!({ "status": "completed", "result": { "ok": true } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["job"]["status"], "completed");

    let (status, final_run) = call(
        &app,
        "GET",
        &format!("/runs/{run_id}"),
        Some(&app.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(final_run["run"]["status"], "completed");
    assert_eq!(final_run["run"]["outputs"]["ok"], true);
}
