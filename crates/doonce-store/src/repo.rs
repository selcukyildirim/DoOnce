//! Repositories. Every function takes a [`Database`] and an explicit
//! `tenant_id`; there is no ambient tenant state, which makes cross-tenant
//! access bugs visible in code review and tests.

use doonce_core::validation::validate_workflow;
use doonce_core::{
    ApiKeyRecord, ApprovalRecord, Artifact, ArtifactKind, BrowserSession, Connector, ConnectorJob,
    Credential, CredentialKind, IdentityProvider, LlmConfig, MfaTotp, OidcStateRecord, Role,
    Schedule, SessionRecord, Setting, Task, TaskAction, TaskStatus, Tenant, User, UserIdentity,
    Webhook, WebhookDelivery, WorkflowDefinition, WorkflowRecord, WorkflowRun,
    WorkflowVersionRecord, new_id, now_rfc3339, slugify,
};
use doonce_security::audit::{AuditHashInput, GENESIS_HASH, hash_entry};
use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Database, Result, StoreError};

fn text(row: &AnyRow, col: &str) -> Result<String> {
    Ok(row.try_get(col)?)
}

fn opt_text(row: &AnyRow, col: &str) -> Result<Option<String>> {
    Ok(row.try_get(col)?)
}

fn integer(row: &AnyRow, col: &str) -> Result<i64> {
    Ok(row.try_get(col)?)
}

fn opt_i64(row: &AnyRow, col: &str) -> Result<Option<i64>> {
    Ok(row.try_get(col)?)
}

fn json_opt(row: &AnyRow, col: &str) -> Result<Option<Value>> {
    match opt_text(row, col)? {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

fn json_value(row: &AnyRow, col: &str) -> Result<Value> {
    Ok(json_opt(row, col)?.unwrap_or(Value::Null))
}

fn strings_opt(row: &AnyRow, col: &str) -> Result<Vec<String>> {
    Ok(json_opt(row, col)?
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

fn encode(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn require_tenant(tenant_id: &str) -> Result<()> {
    if tenant_id.trim().is_empty() {
        return Err(StoreError::Invalid("tenant_id must not be empty".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tenants
// ---------------------------------------------------------------------------

pub async fn create_tenant(db: &Database, name: &str, slug: &str) -> Result<Tenant> {
    create_tenant_with_region(db, name, slug, doonce_core::DEFAULT_REGION).await
}

pub async fn create_tenant_with_region(
    db: &Database,
    name: &str,
    slug: &str,
    region: &str,
) -> Result<Tenant> {
    if name.trim().is_empty() {
        return Err(StoreError::Invalid("tenant name must not be empty".into()));
    }
    if slug.trim().is_empty() {
        return Err(StoreError::Invalid("tenant slug must not be empty".into()));
    }
    let region = region.trim();
    if region.is_empty() {
        return Err(StoreError::Invalid(
            "tenant region must not be empty".into(),
        ));
    }
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO tenants (id, name, slug, status, region, created_at, updated_at) VALUES (?, ?, ?, 'active', ?, ?, ?)",
    ))
    .bind(&id)
    .bind(name.trim())
    .bind(slug.trim())
    .bind(region)
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_tenant(db, &id).await
}

pub async fn get_tenant(db: &Database, id: &str) -> Result<Tenant> {
    let row = sqlx::query(&db.sql(
        "SELECT id, name, slug, status, region, created_at, updated_at FROM tenants WHERE id = ?",
    ))
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("tenant {id}")))?;
    Ok(Tenant {
        id: text(&row, "id")?,
        name: text(&row, "name")?,
        slug: text(&row, "slug")?,
        status: text(&row, "status")?,
        region: text(&row, "region")?,
        created_at: text(&row, "created_at")?,
        updated_at: text(&row, "updated_at")?,
    })
}

pub async fn find_tenant_by_slug(db: &Database, slug: &str) -> Result<Option<Tenant>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, name, slug, status, region, created_at, updated_at FROM tenants WHERE slug = ?",
    ))
    .bind(slug)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(Tenant {
            id: text(&row, "id")?,
            name: text(&row, "name")?,
            slug: text(&row, "slug")?,
            status: text(&row, "status")?,
            region: text(&row, "region")?,
            created_at: text(&row, "created_at")?,
            updated_at: text(&row, "updated_at")?,
        })
    })
    .transpose()
}

pub async fn list_tenants(db: &Database) -> Result<Vec<Tenant>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, name, slug, status, region, created_at, updated_at FROM tenants ORDER BY created_at ASC",
    ))
    .fetch_all(db.pool())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(Tenant {
                id: text(&row, "id")?,
                name: text(&row, "name")?,
                slug: text(&row, "slug")?,
                status: text(&row, "status")?,
                region: text(&row, "region")?,
                created_at: text(&row, "created_at")?,
                updated_at: text(&row, "updated_at")?,
            })
        })
        .collect()
}

pub async fn count_tenants(db: &Database) -> Result<i64> {
    let row = sqlx::query(&db.sql("SELECT COUNT(*) AS n FROM tenants"))
        .fetch_one(db.pool())
        .await?;
    integer(&row, "n")
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

pub async fn create_user(
    db: &Database,
    tenant_id: &str,
    email: &str,
    display_name: &str,
    role: Role,
) -> Result<User> {
    require_tenant(tenant_id)?;
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(StoreError::Invalid(
            "a valid email address is required".into(),
        ));
    }
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO users (id, tenant_id, email, display_name, role, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(&email)
    .bind(display_name.trim())
    .bind(role.as_str())
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_user(db, tenant_id, &id).await
}

pub async fn get_user(db: &Database, tenant_id: &str, id: &str) -> Result<User> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, email, display_name, role, status, created_at, updated_at
         FROM users WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("user {id}")))?;
    user_from_row(&row)
}

pub async fn find_user_by_email(
    db: &Database,
    tenant_id: &str,
    email: &str,
) -> Result<Option<User>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, email, display_name, role, status, created_at, updated_at
         FROM users WHERE tenant_id = ? AND email = ?",
    ))
    .bind(tenant_id)
    .bind(email.trim().to_ascii_lowercase())
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| user_from_row(&row)).transpose()
}

pub async fn list_users(db: &Database, tenant_id: &str) -> Result<Vec<User>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, email, display_name, role, status, created_at, updated_at
         FROM users WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(user_from_row).collect()
}

pub async fn count_users(db: &Database, tenant_id: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql("SELECT COUNT(*) AS n FROM users WHERE tenant_id = ?"))
        .bind(tenant_id)
        .fetch_one(db.pool())
        .await?;
    integer(&row, "n")
}

fn user_from_row(row: &AnyRow) -> Result<User> {
    Ok(User {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        email: text(row, "email")?,
        display_name: text(row, "display_name")?,
        role: text(row, "role")?,
        status: text(row, "status")?,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_api_key(
    db: &Database,
    tenant_id: &str,
    user_id: Option<&str>,
    name: &str,
    prefix: &str,
    key_hash: &str,
    scopes: &[String],
) -> Result<ApiKeyRecord> {
    require_tenant(tenant_id)?;
    let id = new_id();
    let now = now_rfc3339();
    let scopes = encode(&serde_json::json!(scopes))?;
    sqlx::query(&db.sql(
        "INSERT INTO api_keys (id, tenant_id, user_id, name, prefix, key_hash, scopes, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(name.trim())
    .bind(prefix)
    .bind(key_hash)
    .bind(&scopes)
    .bind(&now)
    .execute(db.pool())
    .await?;
    find_api_key_by_prefix(db, prefix)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("api key {id}")))
}

pub async fn find_api_key_by_prefix(db: &Database, prefix: &str) -> Result<Option<ApiKeyRecord>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, user_id, name, prefix, key_hash, scopes, last_used_at, expires_at, revoked_at, created_at
         FROM api_keys WHERE prefix = ?",
    ))
    .bind(prefix)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(ApiKeyRecord {
            id: text(&row, "id")?,
            tenant_id: text(&row, "tenant_id")?,
            user_id: opt_text(&row, "user_id")?,
            name: text(&row, "name")?,
            prefix: text(&row, "prefix")?,
            key_hash: text(&row, "key_hash")?,
            scopes: strings_opt(&row, "scopes")?,
            last_used_at: opt_text(&row, "last_used_at")?,
            expires_at: opt_text(&row, "expires_at")?,
            revoked_at: opt_text(&row, "revoked_at")?,
            created_at: text(&row, "created_at")?,
        })
    })
    .transpose()
}

pub async fn count_api_keys(db: &Database, tenant_id: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql("SELECT COUNT(*) AS n FROM api_keys WHERE tenant_id = ?"))
        .bind(tenant_id)
        .fetch_one(db.pool())
        .await?;
    integer(&row, "n")
}

pub async fn mark_api_key_used(db: &Database, id: &str) -> Result<()> {
    sqlx::query(&db.sql("UPDATE api_keys SET last_used_at = ? WHERE id = ?"))
        .bind(now_rfc3339())
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

pub async fn list_api_keys(db: &Database, tenant_id: &str) -> Result<Vec<ApiKeyRecord>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, user_id, name, prefix, key_hash, scopes, last_used_at, expires_at, revoked_at, created_at
         FROM api_keys WHERE tenant_id = ? ORDER BY created_at DESC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ApiKeyRecord {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                user_id: opt_text(row, "user_id")?,
                name: text(row, "name")?,
                prefix: text(row, "prefix")?,
                key_hash: text(row, "key_hash")?,
                scopes: strings_opt(row, "scopes")?,
                last_used_at: opt_text(row, "last_used_at")?,
                expires_at: opt_text(row, "expires_at")?,
                revoked_at: opt_text(row, "revoked_at")?,
                created_at: text(row, "created_at")?,
            })
        })
        .collect()
}

pub async fn revoke_api_key(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let at = now_rfc3339();
    let result =
        sqlx::query(&db.sql("UPDATE api_keys SET revoked_at = ? WHERE tenant_id = ? AND id = ?"))
            .bind(&at)
            .bind(tenant_id)
            .bind(id)
            .execute(db.pool())
            .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("api key {id}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------------

pub async fn create_workflow(
    db: &Database,
    tenant_id: &str,
    definition: &WorkflowDefinition,
    created_by: Option<&str>,
    reason: &str,
) -> Result<WorkflowRecord> {
    require_tenant(tenant_id)?;
    let issues = validate_workflow(definition);
    if !issues.is_empty() {
        return Err(StoreError::Invalid(
            issues
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let base = slugify(&definition.name);
    let mut slug = base.clone();
    let mut n = 2u32;
    while find_workflow_by_slug(db, tenant_id, &slug).await?.is_some() {
        slug = format!("{base}-{n}");
        n += 1;
        if n > 10_000 {
            return Err(StoreError::Conflict(
                "could not allocate a unique workflow slug".into(),
            ));
        }
    }
    let id = new_id();
    let now = now_rfc3339();
    let json = encode(&serde_json::to_value(definition)?)?;
    let mut tx = db.pool().begin().await?;
    sqlx::query(&db.sql(
        "INSERT INTO workflows (id, tenant_id, slug, name, current_version, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(&slug)
    .bind(&definition.name)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(&db.sql(
        "INSERT INTO workflow_versions (id, tenant_id, workflow_id, version, definition, reason, created_by, created_at)
         VALUES (?, ?, ?, 1, ?, ?, ?, ?)",
    ))
    .bind(new_id())
    .bind(tenant_id)
    .bind(&id)
    .bind(&json)
    .bind(reason)
    .bind(created_by)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_workflow(db, tenant_id, &id).await
}

pub async fn get_workflow(
    db: &Database,
    tenant_id: &str,
    reference: &str,
) -> Result<WorkflowRecord> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, slug, name, current_version, created_at, updated_at
         FROM workflows
         WHERE tenant_id = ? AND (id = ? OR slug = ?)
         ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END
         LIMIT 1",
    ))
    .bind(tenant_id)
    .bind(reference)
    .bind(reference)
    .bind(reference)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("workflow {reference}")))?;
    workflow_from_row(&row)
}

pub async fn find_workflow_by_slug(
    db: &Database,
    tenant_id: &str,
    slug: &str,
) -> Result<Option<WorkflowRecord>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, slug, name, current_version, created_at, updated_at
         FROM workflows WHERE tenant_id = ? AND slug = ?",
    ))
    .bind(tenant_id)
    .bind(slug)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| workflow_from_row(&row)).transpose()
}

pub async fn list_workflows(db: &Database, tenant_id: &str) -> Result<Vec<WorkflowRecord>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, slug, name, current_version, created_at, updated_at
         FROM workflows WHERE tenant_id = ? ORDER BY updated_at DESC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(workflow_from_row).collect()
}

pub async fn delete_workflow(db: &Database, tenant_id: &str, reference: &str) -> Result<()> {
    let workflow = get_workflow(db, tenant_id, reference).await?;
    let result = sqlx::query(&db.sql("DELETE FROM workflows WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(&workflow.id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("workflow {reference}")));
    }
    Ok(())
}

pub async fn count_workflows(db: &Database, tenant_id: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql("SELECT COUNT(*) AS n FROM workflows WHERE tenant_id = ?"))
        .bind(tenant_id)
        .fetch_one(db.pool())
        .await?;
    integer(&row, "n")
}

fn workflow_from_row(row: &AnyRow) -> Result<WorkflowRecord> {
    Ok(WorkflowRecord {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        slug: text(row, "slug")?,
        name: text(row, "name")?,
        current_version: integer(row, "current_version")?,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

pub async fn create_workflow_version(
    db: &Database,
    tenant_id: &str,
    workflow_id: &str,
    definition: &WorkflowDefinition,
    reason: &str,
    created_by: Option<&str>,
) -> Result<WorkflowVersionRecord> {
    let issues = validate_workflow(definition);
    if !issues.is_empty() {
        return Err(StoreError::Invalid(
            issues
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let workflow = get_workflow(db, tenant_id, workflow_id).await?;
    let row = sqlx::query(&db.sql(
        "SELECT COALESCE(MAX(version), 0) + 1 AS next FROM workflow_versions WHERE workflow_id = ?",
    ))
    .bind(&workflow.id)
    .fetch_one(db.pool())
    .await?;
    let version = integer(&row, "next")?;
    let id = new_id();
    let now = now_rfc3339();
    let json = encode(&serde_json::to_value(definition)?)?;
    let mut tx = db.pool().begin().await?;
    sqlx::query(&db.sql(
        "INSERT INTO workflow_versions (id, tenant_id, workflow_id, version, definition, reason, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(&workflow.id)
    .bind(version)
    .bind(&json)
    .bind(reason)
    .bind(created_by)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(&db.sql(
        "UPDATE workflows SET name = ?, current_version = ?, updated_at = ? WHERE tenant_id = ? AND id = ?",
    ))
    .bind(&definition.name)
    .bind(version)
    .bind(&now)
    .bind(tenant_id)
    .bind(&workflow.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_workflow_version(db, tenant_id, &workflow.id, version).await
}

pub async fn get_workflow_version(
    db: &Database,
    tenant_id: &str,
    workflow_id: &str,
    version: i64,
) -> Result<WorkflowVersionRecord> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, version, definition, reason, created_by, created_at
         FROM workflow_versions WHERE tenant_id = ? AND workflow_id = ? AND version = ?",
    ))
    .bind(tenant_id)
    .bind(workflow_id)
    .bind(version)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("workflow version {workflow_id}@{version}")))?;
    workflow_version_from_row(&row)
}

pub async fn current_workflow_definition(
    db: &Database,
    tenant_id: &str,
    reference: &str,
) -> Result<(WorkflowRecord, WorkflowDefinition)> {
    let workflow = get_workflow(db, tenant_id, reference).await?;
    let version =
        get_workflow_version(db, tenant_id, &workflow.id, workflow.current_version).await?;
    Ok((workflow, version.definition))
}

pub async fn list_workflow_versions(
    db: &Database,
    tenant_id: &str,
    workflow_id: &str,
) -> Result<Vec<WorkflowVersionRecord>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, version, definition, reason, created_by, created_at
         FROM workflow_versions WHERE tenant_id = ? AND workflow_id = ? ORDER BY version DESC",
    ))
    .bind(tenant_id)
    .bind(workflow_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(workflow_version_from_row).collect()
}

fn workflow_version_from_row(row: &AnyRow) -> Result<WorkflowVersionRecord> {
    let raw = text(row, "definition")?;
    Ok(WorkflowVersionRecord {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        workflow_id: text(row, "workflow_id")?,
        version: integer(row, "version")?,
        definition: serde_json::from_str(&raw)?,
        reason: text(row, "reason")?,
        created_by: opt_text(row, "created_by")?,
        created_at: text(row, "created_at")?,
    })
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_task(
    db: &Database,
    tenant_id: &str,
    created_by: Option<&str>,
    prompt: &str,
    url: Option<&str>,
    navigation_goal: Option<&str>,
    data_schema: Option<&Value>,
    error_codes: &[String],
    model: Option<&str>,
    max_steps: u32,
) -> Result<Task> {
    require_tenant(tenant_id)?;
    if prompt.trim().is_empty() {
        return Err(StoreError::Invalid("prompt must not be empty".into()));
    }
    if max_steps == 0 || max_steps > 100 {
        return Err(StoreError::Invalid(
            "max_steps must be between 1 and 100".into(),
        ));
    }
    let id = new_id();
    let now = now_rfc3339();
    let schema = match data_schema {
        Some(v) => Some(encode(v)?),
        None => None,
    };
    let codes = encode(&serde_json::json!(error_codes))?;
    let mut tx = db.pool().begin().await?;
    let connector = connector_for_tenant_tx(db, &mut tx, tenant_id).await?;
    sqlx::query(&db.sql(
        "INSERT INTO tasks (id, tenant_id, created_by, prompt, url, navigation_goal, data_schema, error_codes,
                            status, model, max_steps, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(created_by)
    .bind(prompt)
    .bind(url)
    .bind(navigation_goal)
    .bind(schema)
    .bind(codes)
    .bind(model)
    .bind(i64::from(max_steps))
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    if let Some(connector) = connector {
        let payload = serde_json::json!({
            "kind": "task",
            "taskId": id,
            "prompt": prompt,
            "url": url,
            "navigationGoal": navigation_goal,
            "dataSchema": data_schema,
            "errorCodes": error_codes,
            "model": model,
            "maxSteps": max_steps,
        });
        insert_connector_job_tx(
            db,
            &mut tx,
            tenant_id,
            &connector.id,
            "task",
            None,
            Some(&id),
            &payload,
            &now,
        )
        .await?;
    }
    tx.commit().await?;
    get_task(db, tenant_id, &id).await
}

pub async fn get_task(db: &Database, tenant_id: &str, id: &str) -> Result<Task> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, created_by, prompt, url, navigation_goal, data_schema, error_codes, status,
                output, failure_reason, model, max_steps, browser_session_id, created_at, started_at, finished_at
         FROM tasks WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?;
    task_from_row(&row)
}

pub async fn list_tasks(db: &Database, tenant_id: &str, limit: i64) -> Result<Vec<Task>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, created_by, prompt, url, navigation_goal, data_schema, error_codes, status,
                output, failure_reason, model, max_steps, browser_session_id, created_at, started_at, finished_at
         FROM tasks WHERE tenant_id = ? ORDER BY created_at DESC LIMIT ?",
    ))
    .bind(tenant_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(task_from_row).collect()
}

pub async fn count_tasks(db: &Database, tenant_id: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql("SELECT COUNT(*) AS n FROM tasks WHERE tenant_id = ?"))
        .bind(tenant_id)
        .fetch_one(db.pool())
        .await?;
    integer(&row, "n")
}

#[allow(clippy::too_many_arguments)]
pub async fn set_task_status(
    db: &Database,
    tenant_id: &str,
    id: &str,
    status: TaskStatus,
    output: Option<&Value>,
    failure_reason: Option<&str>,
    started_at: Option<&str>,
    finished_at: Option<&str>,
) -> Result<Task> {
    let output = match output {
        Some(v) => Some(encode(v)?),
        None => None,
    };
    let result = sqlx::query(&db.sql(
        "UPDATE tasks SET status = ?, output = COALESCE(?, output), failure_reason = COALESCE(?, failure_reason),
                          started_at = COALESCE(?, started_at), finished_at = COALESCE(?, finished_at)
         WHERE tenant_id = ? AND id = ?",
    ))
    .bind(status.as_str())
    .bind(output)
    .bind(failure_reason)
    .bind(started_at)
    .bind(finished_at)
    .bind(tenant_id)
    .bind(id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("task {id}")));
    }
    get_task(db, tenant_id, id).await
}

pub async fn record_task_action(
    db: &Database,
    tenant_id: &str,
    task_id: &str,
    action: &Value,
    reasoning: Option<&str>,
    screenshot_artifact_id: Option<&str>,
    status: &str,
) -> Result<TaskAction> {
    let row = sqlx::query(
        &db.sql("SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM task_actions WHERE task_id = ?"),
    )
    .bind(task_id)
    .fetch_one(db.pool())
    .await?;
    let seq = integer(&row, "next")?;
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO task_actions (id, tenant_id, task_id, seq, action, reasoning, screenshot_artifact_id, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(task_id)
    .bind(seq)
    .bind(encode(action)?)
    .bind(reasoning)
    .bind(screenshot_artifact_id)
    .bind(status)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(TaskAction {
        id,
        tenant_id: tenant_id.to_string(),
        task_id: task_id.to_string(),
        seq,
        action: action.clone(),
        reasoning: reasoning.map(str::to_string),
        screenshot_artifact_id: screenshot_artifact_id.map(str::to_string),
        status: status.to_string(),
        created_at: now,
    })
}

pub async fn create_run_diff(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
    baseline_run_id: &str,
    pixel_diff_pct: f64,
    changed_bytes: i64,
) -> Result<String> {
    require_tenant(tenant_id)?;
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO run_diffs (id, tenant_id, run_id, baseline_run_id, pixel_diff_pct, changed_bytes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(run_id)
    .bind(baseline_run_id)
    .bind(pixel_diff_pct)
    .bind(changed_bytes)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(id)
}

pub async fn list_run_diffs(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<(String, String, f64, i64, String)>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, baseline_run_id, pixel_diff_pct, changed_bytes, created_at FROM run_diffs WHERE tenant_id = ? AND run_id = ? ORDER BY created_at DESC",
    ))
    .bind(tenant_id)
    .bind(run_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                text(row, "id")?,
                text(row, "baseline_run_id")?,
                row.try_get::<f64, _>("pixel_diff_pct").unwrap_or(0.0),
                row.try_get::<i64, _>("changed_bytes").unwrap_or(0),
                text(row, "created_at")?,
            ))
        })
        .collect()
}

pub async fn list_task_actions(
    db: &Database,
    tenant_id: &str,
    task_id: &str,
) -> Result<Vec<TaskAction>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, task_id, seq, action, reasoning, screenshot_artifact_id, status, created_at
         FROM task_actions WHERE tenant_id = ? AND task_id = ? ORDER BY seq ASC",
    ))
    .bind(tenant_id)
    .bind(task_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(TaskAction {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                task_id: text(row, "task_id")?,
                seq: integer(row, "seq")?,
                action: json_value(row, "action")?,
                reasoning: opt_text(row, "reasoning")?,
                screenshot_artifact_id: opt_text(row, "screenshot_artifact_id")?,
                status: text(row, "status")?,
                created_at: text(row, "created_at")?,
            })
        })
        .collect()
}

fn task_from_row(row: &AnyRow) -> Result<Task> {
    Ok(Task {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        created_by: opt_text(row, "created_by")?,
        prompt: text(row, "prompt")?,
        url: opt_text(row, "url")?,
        navigation_goal: opt_text(row, "navigation_goal")?,
        data_schema: json_opt(row, "data_schema")?,
        error_codes: strings_opt(row, "error_codes")?,
        status: text(row, "status")?,
        output: json_opt(row, "output")?,
        failure_reason: opt_text(row, "failure_reason")?,
        model: opt_text(row, "model")?,
        max_steps: u32::try_from(integer(row, "max_steps")?).unwrap_or(30),
        browser_session_id: opt_text(row, "browser_session_id")?,
        created_at: text(row, "created_at")?,
        started_at: opt_text(row, "started_at")?,
        finished_at: opt_text(row, "finished_at")?,
    })
}

// ---------------------------------------------------------------------------
// Workflow runs and blocks
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_workflow_run(
    db: &Database,
    tenant_id: &str,
    workflow_id: &str,
    workflow_version: i64,
    inputs: &Value,
    trigger: &str,
    parent_run_id: Option<&str>,
    created_by: Option<&str>,
) -> Result<WorkflowRun> {
    let id = new_id();
    let now = now_rfc3339();
    let mut tx = db.pool().begin().await?;
    let connector = connector_for_tenant_tx(db, &mut tx, tenant_id).await?;
    sqlx::query(&db.sql(
        "INSERT INTO workflow_runs (id, tenant_id, workflow_id, workflow_version, status, inputs, trigger, parent_run_id, created_by, created_at)
         VALUES (?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(workflow_id)
    .bind(workflow_version)
    .bind(encode(inputs)?)
    .bind(trigger)
    .bind(parent_run_id)
    .bind(created_by)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    if let Some(connector) = connector {
        let row = sqlx::query(&db.sql(
            "SELECT w.slug AS slug, v.definition AS definition
             FROM workflows w JOIN workflow_versions v ON v.workflow_id = w.id AND v.version = ?
             WHERE w.tenant_id = ? AND w.id = ?",
        ))
        .bind(workflow_version)
        .bind(tenant_id)
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("workflow version {workflow_version}")))?;
        let slug = text(&row, "slug")?;
        let definition: Value = serde_json::from_str(&text(&row, "definition")?)?;
        let payload = serde_json::json!({
            "kind": "workflow",
            "runId": id,
            "workflowId": workflow_id,
            "workflowVersion": workflow_version,
            "workflowSlug": slug,
            "definition": definition,
            "inputs": inputs,
        });
        insert_connector_job_tx(
            db,
            &mut tx,
            tenant_id,
            &connector.id,
            "workflow",
            Some(&id),
            None,
            &payload,
            &now,
        )
        .await?;
    }
    tx.commit().await?;
    get_workflow_run(db, tenant_id, &id).await
}

pub async fn get_workflow_run(db: &Database, tenant_id: &str, id: &str) -> Result<WorkflowRun> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, workflow_version, status, inputs, outputs, error, trigger,
                parent_run_id, created_by, created_at, started_at, finished_at
         FROM workflow_runs WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("workflow run {id}")))?;
    Ok(WorkflowRun {
        id: text(&row, "id")?,
        tenant_id: text(&row, "tenant_id")?,
        workflow_id: text(&row, "workflow_id")?,
        workflow_version: integer(&row, "workflow_version")?,
        status: text(&row, "status")?,
        inputs: json_value(&row, "inputs")?,
        outputs: json_opt(&row, "outputs")?,
        error: opt_text(&row, "error")?,
        trigger: text(&row, "trigger")?,
        parent_run_id: opt_text(&row, "parent_run_id")?,
        created_by: opt_text(&row, "created_by")?,
        created_at: text(&row, "created_at")?,
        started_at: opt_text(&row, "started_at")?,
        finished_at: opt_text(&row, "finished_at")?,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn list_workflow_runs(
    db: &Database,
    tenant_id: &str,
    workflow_id: Option<&str>,
    limit: i64,
) -> Result<Vec<WorkflowRun>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, workflow_version, status, inputs, outputs, error, trigger,
                parent_run_id, created_by, created_at, started_at, finished_at
         FROM workflow_runs
         WHERE tenant_id = ? AND (? IS NULL OR workflow_id = ?)
         ORDER BY created_at DESC LIMIT ?",
    ))
    .bind(tenant_id)
    .bind(workflow_id)
    .bind(workflow_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(WorkflowRun {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                workflow_id: text(row, "workflow_id")?,
                workflow_version: integer(row, "workflow_version")?,
                status: text(row, "status")?,
                inputs: json_value(row, "inputs")?,
                outputs: json_opt(row, "outputs")?,
                error: opt_text(row, "error")?,
                trigger: text(row, "trigger")?,
                parent_run_id: opt_text(row, "parent_run_id")?,
                created_by: opt_text(row, "created_by")?,
                created_at: text(row, "created_at")?,
                started_at: opt_text(row, "started_at")?,
                finished_at: opt_text(row, "finished_at")?,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub async fn set_workflow_run_status(
    db: &Database,
    tenant_id: &str,
    id: &str,
    status: &str,
    outputs: Option<&Value>,
    error: Option<&str>,
    started_at: Option<&str>,
    finished_at: Option<&str>,
) -> Result<WorkflowRun> {
    let outputs = match outputs {
        Some(v) => Some(encode(v)?),
        None => None,
    };
    let result = sqlx::query(&db.sql(
        "UPDATE workflow_runs SET status = ?, outputs = COALESCE(?, outputs), error = COALESCE(?, error),
                                  started_at = COALESCE(?, started_at), finished_at = COALESCE(?, finished_at)
         WHERE tenant_id = ? AND id = ?",
    ))
    .bind(status)
    .bind(outputs)
    .bind(error)
    .bind(started_at)
    .bind(finished_at)
    .bind(tenant_id)
    .bind(id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("workflow run {id}")));
    }
    get_workflow_run(db, tenant_id, id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn record_block_run(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
    block_id: &str,
    seq: i64,
    status: &str,
    input: Option<&Value>,
    output: Option<&Value>,
    error: Option<&str>,
    attempt: i64,
) -> Result<String> {
    let id = new_id();
    let now = now_rfc3339();
    let finished = if matches!(status, "completed" | "failed" | "cancelled" | "skipped") {
        Some(now.as_str())
    } else {
        None
    };
    let input = match input {
        Some(v) => Some(encode(v)?),
        None => None,
    };
    let output = match output {
        Some(v) => Some(encode(v)?),
        None => None,
    };
    sqlx::query(&db.sql(
        "INSERT INTO workflow_block_runs (id, tenant_id, run_id, block_id, seq, status, input, output, error, attempt, created_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(run_id)
    .bind(block_id)
    .bind(seq)
    .bind(status)
    .bind(input)
    .bind(output)
    .bind(error)
    .bind(attempt)
    .bind(&now)
    .bind(finished)
    .execute(db.pool())
    .await?;
    Ok(id)
}

pub async fn list_block_runs(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<doonce_core::WorkflowBlockRun>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, block_id, seq, status, input, output, error, attempt, created_at, finished_at
         FROM workflow_block_runs WHERE tenant_id = ? AND run_id = ? ORDER BY seq ASC",
    ))
    .bind(tenant_id)
    .bind(run_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(doonce_core::WorkflowBlockRun {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                run_id: text(row, "run_id")?,
                block_id: text(row, "block_id")?,
                seq: integer(row, "seq")?,
                status: text(row, "status")?,
                input: json_opt(row, "input")?,
                output: json_opt(row, "output")?,
                error: opt_text(row, "error")?,
                attempt: integer(row, "attempt")?,
                created_at: text(row, "created_at")?,
                finished_at: opt_text(row, "finished_at")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_artifact(
    db: &Database,
    tenant_id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    block_run_id: Option<&str>,
    kind: ArtifactKind,
    filename: &str,
    content_type: &str,
    size: i64,
    storage_key: &str,
    sha256: &str,
) -> Result<Artifact> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO artifacts (id, tenant_id, run_id, task_id, block_run_id, kind, filename, content_type, size, storage_key, sha256, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(run_id)
    .bind(task_id)
    .bind(block_run_id)
    .bind(kind.as_str())
    .bind(filename)
    .bind(content_type)
    .bind(size)
    .bind(storage_key)
    .bind(sha256)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(Artifact {
        id,
        tenant_id: tenant_id.to_string(),
        run_id: run_id.map(str::to_string),
        task_id: task_id.map(str::to_string),
        block_run_id: block_run_id.map(str::to_string),
        kind: kind.as_str().to_string(),
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        size,
        storage_key: storage_key.to_string(),
        sha256: sha256.to_string(),
        created_at: now,
    })
}

pub async fn list_artifacts_for_run(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<Artifact>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, task_id, block_run_id, kind, filename, content_type, size, storage_key, sha256, created_at
         FROM artifacts WHERE tenant_id = ? AND run_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .bind(run_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(artifact_from_row).collect()
}

pub async fn list_artifacts_for_task(
    db: &Database,
    tenant_id: &str,
    task_id: &str,
) -> Result<Vec<Artifact>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, task_id, block_run_id, kind, filename, content_type, size, storage_key, sha256, created_at
         FROM artifacts WHERE tenant_id = ? AND task_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .bind(task_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(artifact_from_row).collect()
}

pub async fn get_artifact(db: &Database, tenant_id: &str, id: &str) -> Result<Artifact> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, task_id, block_run_id, kind, filename, content_type, size, storage_key, sha256, created_at
         FROM artifacts WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("artifact {id}")))?;
    artifact_from_row(&row)
}

fn artifact_from_row(row: &AnyRow) -> Result<Artifact> {
    Ok(Artifact {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        run_id: opt_text(row, "run_id")?,
        task_id: opt_text(row, "task_id")?,
        block_run_id: opt_text(row, "block_run_id")?,
        kind: text(row, "kind")?,
        filename: text(row, "filename")?,
        content_type: text(row, "content_type")?,
        size: integer(row, "size")?,
        storage_key: text(row, "storage_key")?,
        sha256: text(row, "sha256")?,
        created_at: text(row, "created_at")?,
    })
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_credential(
    db: &Database,
    tenant_id: &str,
    name: &str,
    kind: CredentialKind,
    username: Option<&str>,
    secret_encrypted: &str,
    totp_secret_encrypted: Option<&str>,
    metadata: &Value,
) -> Result<Credential> {
    create_credential_with_provider(
        db,
        tenant_id,
        name,
        kind,
        "local",
        None,
        username,
        secret_encrypted,
        totp_secret_encrypted,
        metadata,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_credential_with_provider(
    db: &Database,
    tenant_id: &str,
    name: &str,
    kind: CredentialKind,
    provider: &str,
    external_ref: Option<&str>,
    username: Option<&str>,
    secret_encrypted: &str,
    totp_secret_encrypted: Option<&str>,
    metadata: &Value,
) -> Result<Credential> {
    require_tenant(tenant_id)?;
    if name.trim().is_empty() {
        return Err(StoreError::Invalid(
            "credential name must not be empty".into(),
        ));
    }
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO credentials (id, tenant_id, name, kind, provider, external_ref, username, secret_encrypted, totp_secret_encrypted, metadata, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(name.trim())
    .bind(kind.as_str())
    .bind(provider)
    .bind(external_ref)
    .bind(username)
    .bind(secret_encrypted)
    .bind(totp_secret_encrypted)
    .bind(encode(metadata)?)
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_credential(db, tenant_id, &id).await
}

pub async fn get_credential(db: &Database, tenant_id: &str, id: &str) -> Result<Credential> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, provider, external_ref, username, secret_encrypted, totp_secret_encrypted, metadata, created_at, updated_at
         FROM credentials WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("credential {id}")))?;
    credential_from_row(&row)
}

pub async fn list_credentials(db: &Database, tenant_id: &str) -> Result<Vec<Credential>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, provider, external_ref, username, secret_encrypted, totp_secret_encrypted, metadata, created_at, updated_at
         FROM credentials WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(credential_from_row).collect()
}

pub async fn delete_credential(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let result = sqlx::query(&db.sql("DELETE FROM credentials WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("credential {id}")));
    }
    Ok(())
}

fn credential_from_row(row: &AnyRow) -> Result<Credential> {
    Ok(Credential {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        name: text(row, "name")?,
        kind: text(row, "kind")?,
        provider: text(row, "provider")?,
        external_ref: opt_text(row, "external_ref")?,
        username: opt_text(row, "username")?,
        secret_encrypted: text(row, "secret_encrypted")?,
        totp_secret_encrypted: opt_text(row, "totp_secret_encrypted")?,
        metadata: json_value(row, "metadata")?,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

// ---------------------------------------------------------------------------
// Browser sessions
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_browser_session(
    db: &Database,
    tenant_id: &str,
    name: &str,
    kind: &str,
    user_data_dir: Option<&str>,
    cdp_url: Option<&str>,
    proxy_url: Option<&str>,
) -> Result<BrowserSession> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO browser_sessions (id, tenant_id, name, kind, status, user_data_dir, cdp_url, proxy_url, created_at)
         VALUES (?, ?, ?, ?, 'opening', ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(name)
    .bind(kind)
    .bind(user_data_dir)
    .bind(cdp_url)
    .bind(proxy_url)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_browser_session(db, tenant_id, &id).await
}

pub async fn get_browser_session(
    db: &Database,
    tenant_id: &str,
    id: &str,
) -> Result<BrowserSession> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, status, user_data_dir, cdp_url, proxy_url, created_at, closed_at
         FROM browser_sessions WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("browser session {id}")))?;
    Ok(BrowserSession {
        id: text(&row, "id")?,
        tenant_id: text(&row, "tenant_id")?,
        name: text(&row, "name")?,
        kind: text(&row, "kind")?,
        status: text(&row, "status")?,
        user_data_dir: opt_text(&row, "user_data_dir")?,
        cdp_url: opt_text(&row, "cdp_url")?,
        proxy_url: opt_text(&row, "proxy_url")?,
        created_at: text(&row, "created_at")?,
        closed_at: opt_text(&row, "closed_at")?,
    })
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

pub async fn create_schedule(
    db: &Database,
    tenant_id: &str,
    workflow_id: &str,
    cron: &str,
    timezone: &str,
) -> Result<Schedule> {
    if cron.split_whitespace().count() != 5 {
        return Err(StoreError::Invalid(
            "cron must have exactly 5 fields".into(),
        ));
    }
    if timezone.trim().is_empty() {
        return Err(StoreError::Invalid("timezone must not be empty".into()));
    }
    let next_run = crate::cron::next_run(cron.trim(), timezone.trim())?;
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO schedules (id, tenant_id, workflow_id, cron, timezone, enabled, next_run_at, created_at)
         VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(workflow_id)
    .bind(cron.trim())
    .bind(timezone.trim())
    .bind(&next_run)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_schedule(db, tenant_id, &id).await
}

pub async fn get_schedule(db: &Database, tenant_id: &str, id: &str) -> Result<Schedule> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, cron, timezone, enabled, next_run_at, last_run_at, last_status, created_at
         FROM schedules WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("schedule {id}")))?;
    schedule_from_row(&row)
}

pub async fn list_schedules(db: &Database, tenant_id: &str) -> Result<Vec<Schedule>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, cron, timezone, enabled, next_run_at, last_run_at, last_status, created_at
         FROM schedules WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(schedule_from_row).collect()
}

pub async fn set_schedule_enabled(
    db: &Database,
    tenant_id: &str,
    id: &str,
    enabled: bool,
) -> Result<Schedule> {
    let result =
        sqlx::query(&db.sql("UPDATE schedules SET enabled = ? WHERE tenant_id = ? AND id = ?"))
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(tenant_id)
            .bind(id)
            .execute(db.pool())
            .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("schedule {id}")));
    }
    get_schedule(db, tenant_id, id).await
}

pub async fn delete_schedule(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let result = sqlx::query(&db.sql("DELETE FROM schedules WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("schedule {id}")));
    }
    Ok(())
}

fn schedule_from_row(row: &AnyRow) -> Result<Schedule> {
    Ok(Schedule {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        workflow_id: text(row, "workflow_id")?,
        cron: text(row, "cron")?,
        timezone: text(row, "timezone")?,
        enabled: integer(row, "enabled")? != 0,
        next_run_at: opt_text(row, "next_run_at")?,
        last_run_at: opt_text(row, "last_run_at")?,
        last_status: opt_text(row, "last_status")?,
        created_at: text(row, "created_at")?,
    })
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub async fn set_setting(db: &Database, tenant_id: &str, key: &str, value: &str) -> Result<()> {
    require_tenant(tenant_id)?;
    if key.trim().is_empty() {
        return Err(StoreError::Invalid("setting key must not be empty".into()));
    }
    sqlx::query(&db.sql(
        "INSERT INTO settings (tenant_id, key, value, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (tenant_id, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    ))
    .bind(tenant_id)
    .bind(key)
    .bind(value)
    .bind(now_rfc3339())
    .execute(db.pool())
    .await?;
    Ok(())
}

pub async fn get_setting(db: &Database, tenant_id: &str, key: &str) -> Result<Option<String>> {
    let row = sqlx::query(&db.sql("SELECT value FROM settings WHERE tenant_id = ? AND key = ?"))
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(db.pool())
        .await?;
    match row {
        Some(row) => Ok(Some(text(&row, "value")?)),
        None => Ok(None),
    }
}

pub async fn list_settings(db: &Database, tenant_id: &str) -> Result<Vec<Setting>> {
    let rows = sqlx::query(&db.sql(
        "SELECT tenant_id, key, value, updated_at FROM settings WHERE tenant_id = ? ORDER BY key ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Setting {
                tenant_id: text(row, "tenant_id")?,
                key: text(row, "key")?,
                value: text(row, "value")?,
                updated_at: text(row, "updated_at")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

pub async fn append_audit(
    db: &Database,
    tenant_id: &str,
    actor_id: Option<&str>,
    action: &str,
    target: &str,
    metadata: &Value,
) -> Result<doonce_core::AuditLog> {
    let row = sqlx::query(
        &db.sql("SELECT hash FROM audit_logs WHERE tenant_id = ? ORDER BY id DESC LIMIT 1"),
    )
    .bind(tenant_id)
    .fetch_optional(db.pool())
    .await?;
    let prev_hash = match row {
        Some(row) => text(&row, "hash")?,
        None => GENESIS_HASH.to_string(),
    };
    let created_at = now_rfc3339();
    let hash = hash_entry(
        &prev_hash,
        &AuditHashInput {
            tenant_id,
            actor_id,
            action,
            target,
            metadata,
            created_at: &created_at,
        },
    )?;
    let id = new_id();
    sqlx::query(&db.sql(
        "INSERT INTO audit_logs (id, tenant_id, actor_id, action, target, metadata, prev_hash, hash, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(actor_id)
    .bind(action)
    .bind(target)
    .bind(encode(metadata)?)
    .bind(&prev_hash)
    .bind(&hash)
    .bind(&created_at)
    .execute(db.pool())
    .await?;
    Ok(doonce_core::AuditLog {
        id,
        tenant_id: tenant_id.to_string(),
        actor_id: actor_id.map(str::to_string),
        action: action.to_string(),
        target: target.to_string(),
        metadata: metadata.clone(),
        prev_hash,
        hash,
        created_at,
    })
}

pub async fn list_audit(
    db: &Database,
    tenant_id: &str,
    limit: i64,
) -> Result<Vec<doonce_core::AuditLog>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, actor_id, action, target, metadata, prev_hash, hash, created_at
         FROM audit_logs WHERE tenant_id = ? ORDER BY id ASC LIMIT ?",
    ))
    .bind(tenant_id)
    .bind(limit.clamp(1, 10_000))
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(doonce_core::AuditLog {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                actor_id: opt_text(row, "actor_id")?,
                action: text(row, "action")?,
                target: text(row, "target")?,
                metadata: json_value(row, "metadata")?,
                prev_hash: text(row, "prev_hash")?,
                hash: text(row, "hash")?,
                created_at: text(row, "created_at")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Webhooks / LLM configs are intentionally minimal until the worker lands.
// ---------------------------------------------------------------------------

pub async fn list_webhooks(db: &Database, tenant_id: &str) -> Result<Vec<Webhook>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, url, secret_encrypted, events, enabled, created_at
         FROM webhooks WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Webhook {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                url: text(row, "url")?,
                secret_encrypted: text(row, "secret_encrypted")?,
                events: strings_opt(row, "events")?,
                enabled: integer(row, "enabled")? != 0,
                created_at: text(row, "created_at")?,
            })
        })
        .collect()
}

pub async fn list_llm_configs(db: &Database, tenant_id: &str) -> Result<Vec<LlmConfig>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, provider, model, base_url, api_key_encrypted, capabilities, is_default, created_at, updated_at\n         FROM llm_configs WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(LlmConfig {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                provider: text(row, "provider")?,
                model: text(row, "model")?,
                base_url: opt_text(row, "base_url")?,
                api_key_encrypted: opt_text(row, "api_key_encrypted")?,
                capabilities: json_value(row, "capabilities")?,
                is_default: integer(row, "is_default")? != 0,
                created_at: text(row, "created_at")?,
                updated_at: text(row, "updated_at")?,
            })
        })
        .collect()
}

pub async fn get_llm_config(db: &Database, tenant_id: &str, id: &str) -> Result<LlmConfig> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, provider, model, base_url, api_key_encrypted, capabilities, is_default, created_at, updated_at\n         FROM llm_configs WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("llm config {id}")))?;
    Ok(LlmConfig {
        id: text(&row, "id")?,
        tenant_id: text(&row, "tenant_id")?,
        provider: text(&row, "provider")?,
        model: text(&row, "model")?,
        base_url: opt_text(&row, "base_url")?,
        api_key_encrypted: opt_text(&row, "api_key_encrypted")?,
        capabilities: json_value(&row, "capabilities")?,
        is_default: integer(&row, "is_default")? != 0,
        created_at: text(&row, "created_at")?,
        updated_at: text(&row, "updated_at")?,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_llm_config(
    db: &Database,
    tenant_id: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    api_key_encrypted: Option<&str>,
    capabilities: &Value,
    is_default: bool,
) -> Result<LlmConfig> {
    require_tenant(tenant_id)?;
    let id = new_id();
    let now = now_rfc3339();
    let caps = encode(capabilities)?;
    if is_default {
        sqlx::query(&db.sql("UPDATE llm_configs SET is_default = 0 WHERE tenant_id = ?"))
            .bind(tenant_id)
            .execute(db.pool())
            .await?;
    }
    sqlx::query(&db.sql(
        "INSERT INTO llm_configs (id, tenant_id, provider, model, base_url, api_key_encrypted, capabilities, is_default, created_at, updated_at)\n         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(provider.trim())
    .bind(model.trim())
    .bind(base_url.map(str::trim).filter(|v| !v.is_empty()))
    .bind(api_key_encrypted)
    .bind(&caps)
    .bind(if is_default { 1 } else { 0 })
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_llm_config(db, tenant_id, &id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_llm_config(
    db: &Database,
    tenant_id: &str,
    id: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    api_key_encrypted: Option<&str>,
    capabilities: &Value,
    is_default: bool,
) -> Result<LlmConfig> {
    require_tenant(tenant_id)?;
    let now = now_rfc3339();
    let caps = encode(capabilities)?;
    if is_default {
        sqlx::query(&db.sql("UPDATE llm_configs SET is_default = 0 WHERE tenant_id = ?"))
            .bind(tenant_id)
            .execute(db.pool())
            .await?;
    }
    let result = sqlx::query(&db.sql(
        "UPDATE llm_configs SET provider = ?, model = ?, base_url = ?, api_key_encrypted = COALESCE(?, api_key_encrypted), capabilities = ?, is_default = ?, updated_at = ?\n         WHERE tenant_id = ? AND id = ?",
    ))
    .bind(provider.trim())
    .bind(model.trim())
    .bind(base_url.map(str::trim).filter(|v| !v.is_empty()))
    .bind(api_key_encrypted)
    .bind(&caps)
    .bind(if is_default { 1 } else { 0 })
    .bind(&now)
    .bind(tenant_id)
    .bind(id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("llm config {id}")));
    }
    get_llm_config(db, tenant_id, id).await
}

pub async fn delete_llm_config(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let result = sqlx::query(&db.sql("DELETE FROM llm_configs WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("llm config {id}")));
    }
    Ok(())
}

pub async fn default_llm_config(db: &Database, tenant_id: &str) -> Result<Option<LlmConfig>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, provider, model, base_url, api_key_encrypted, capabilities, is_default, created_at, updated_at\n         FROM llm_configs WHERE tenant_id = ? AND is_default = 1 LIMIT 1",
    ))
    .bind(tenant_id)
    .fetch_optional(db.pool())
    .await?;
    match row {
        Some(row) => Ok(Some(LlmConfig {
            id: text(&row, "id")?,
            tenant_id: text(&row, "tenant_id")?,
            provider: text(&row, "provider")?,
            model: text(&row, "model")?,
            base_url: opt_text(&row, "base_url")?,
            api_key_encrypted: opt_text(&row, "api_key_encrypted")?,
            capabilities: json_value(&row, "capabilities")?,
            is_default: true,
            created_at: text(&row, "created_at")?,
            updated_at: text(&row, "updated_at")?,
        })),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Identity providers, OIDC state, sessions and MFA
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn create_identity_provider(
    db: &Database,
    tenant_id: &str,
    name: &str,
    kind: &str,
    issuer: &str,
    client_id: &str,
    client_secret_encrypted: Option<&str>,
    scopes: &[String],
) -> Result<IdentityProvider> {
    require_tenant(tenant_id)?;
    let id = new_id();
    let now = now_rfc3339();
    let scopes = encode(&serde_json::json!(scopes))?;
    sqlx::query(&db.sql(
        "INSERT INTO identity_providers (id, tenant_id, name, kind, issuer, client_id, client_secret_encrypted, scopes, enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(name.trim())
    .bind(kind)
    .bind(issuer.trim())
    .bind(client_id.trim())
    .bind(client_secret_encrypted)
    .bind(&scopes)
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_identity_provider(db, tenant_id, &id).await
}

pub async fn get_identity_provider(
    db: &Database,
    tenant_id: &str,
    id: &str,
) -> Result<IdentityProvider> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, issuer, client_id, client_secret_encrypted, scopes, enabled, created_at, updated_at
         FROM identity_providers WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("identity provider {id}")))?;
    identity_provider_from_row(&row)
}

pub async fn list_identity_providers(
    db: &Database,
    tenant_id: &str,
) -> Result<Vec<IdentityProvider>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, issuer, client_id, client_secret_encrypted, scopes, enabled, created_at, updated_at
         FROM identity_providers WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(identity_provider_from_row).collect()
}

pub async fn list_enabled_identity_providers(
    db: &Database,
    tenant_id: &str,
) -> Result<Vec<IdentityProvider>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, issuer, client_id, client_secret_encrypted, scopes, enabled, created_at, updated_at
         FROM identity_providers WHERE tenant_id = ? AND enabled = 1 ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(identity_provider_from_row).collect()
}

pub async fn find_enabled_identity_provider(
    db: &Database,
    tenant_id: &str,
    kind: &str,
) -> Result<Option<IdentityProvider>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, kind, issuer, client_id, client_secret_encrypted, scopes, enabled, created_at, updated_at
         FROM identity_providers WHERE tenant_id = ? AND kind = ? AND enabled = 1 ORDER BY created_at ASC LIMIT 1",
    ))
    .bind(tenant_id)
    .bind(kind)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| identity_provider_from_row(&row)).transpose()
}

fn identity_provider_from_row(row: &AnyRow) -> Result<IdentityProvider> {
    Ok(IdentityProvider {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        name: text(row, "name")?,
        kind: text(row, "kind")?,
        issuer: text(row, "issuer")?,
        client_id: text(row, "client_id")?,
        client_secret_encrypted: opt_text(row, "client_secret_encrypted")?,
        scopes: strings_opt(row, "scopes")?,
        enabled: integer(row, "enabled")? != 0,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

pub async fn delete_identity_provider(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let result =
        sqlx::query(&db.sql("DELETE FROM identity_providers WHERE tenant_id = ? AND id = ?"))
            .bind(tenant_id)
            .bind(id)
            .execute(db.pool())
            .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("identity provider {id}")));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_oidc_state(
    db: &Database,
    tenant_id: &str,
    provider_id: &str,
    state: &str,
    nonce: &str,
    pkce_verifier_encrypted: &str,
    redirect_uri: &str,
    expires_at: &str,
) -> Result<OidcStateRecord> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO oidc_states (id, tenant_id, provider_id, state, nonce, pkce_verifier_encrypted, redirect_uri, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(provider_id)
    .bind(state)
    .bind(nonce)
    .bind(pkce_verifier_encrypted)
    .bind(redirect_uri)
    .bind(expires_at)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(OidcStateRecord {
        id,
        tenant_id: tenant_id.to_string(),
        provider_id: provider_id.to_string(),
        state: state.to_string(),
        nonce: nonce.to_string(),
        pkce_verifier_encrypted: pkce_verifier_encrypted.to_string(),
        redirect_uri: redirect_uri.to_string(),
        expires_at: expires_at.to_string(),
        consumed_at: None,
        created_at: now,
    })
}

pub async fn find_oidc_state_by_state(
    db: &Database,
    state: &str,
) -> Result<Option<OidcStateRecord>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, provider_id, state, nonce, pkce_verifier_encrypted, redirect_uri, expires_at, consumed_at, created_at
         FROM oidc_states WHERE state = ?",
    ))
    .bind(state)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(OidcStateRecord {
            id: text(&row, "id")?,
            tenant_id: text(&row, "tenant_id")?,
            provider_id: text(&row, "provider_id")?,
            state: text(&row, "state")?,
            nonce: text(&row, "nonce")?,
            pkce_verifier_encrypted: text(&row, "pkce_verifier_encrypted")?,
            redirect_uri: text(&row, "redirect_uri")?,
            expires_at: text(&row, "expires_at")?,
            consumed_at: opt_text(&row, "consumed_at")?,
            created_at: text(&row, "created_at")?,
        })
    })
    .transpose()
}

pub async fn consume_oidc_state(
    db: &Database,
    tenant_id: &str,
    state: &str,
) -> Result<Option<OidcStateRecord>> {
    let now = now_rfc3339();
    let result = sqlx::query(&db.sql(
        "UPDATE oidc_states SET consumed_at = ? WHERE tenant_id = ? AND state = ? AND consumed_at IS NULL",
    ))
    .bind(&now)
    .bind(tenant_id)
    .bind(state)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, provider_id, state, nonce, pkce_verifier_encrypted, redirect_uri, expires_at, consumed_at, created_at
         FROM oidc_states WHERE tenant_id = ? AND state = ?",
    ))
    .bind(tenant_id)
    .bind(state)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(OidcStateRecord {
            id: text(&row, "id")?,
            tenant_id: text(&row, "tenant_id")?,
            provider_id: text(&row, "provider_id")?,
            state: text(&row, "state")?,
            nonce: text(&row, "nonce")?,
            pkce_verifier_encrypted: text(&row, "pkce_verifier_encrypted")?,
            redirect_uri: text(&row, "redirect_uri")?,
            expires_at: text(&row, "expires_at")?,
            consumed_at: opt_text(&row, "consumed_at")?,
            created_at: text(&row, "created_at")?,
        })
    })
    .transpose()
}

pub async fn find_user_identity(
    db: &Database,
    tenant_id: &str,
    provider: &str,
    subject: &str,
) -> Result<Option<UserIdentity>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, user_id, provider, subject, email, created_at
         FROM user_identities WHERE tenant_id = ? AND provider = ? AND subject = ?",
    ))
    .bind(tenant_id)
    .bind(provider)
    .bind(subject)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(UserIdentity {
            id: text(&row, "id")?,
            tenant_id: text(&row, "tenant_id")?,
            user_id: text(&row, "user_id")?,
            provider: text(&row, "provider")?,
            subject: text(&row, "subject")?,
            email: opt_text(&row, "email")?,
            created_at: text(&row, "created_at")?,
        })
    })
    .transpose()
}

pub async fn link_user_identity(
    db: &Database,
    tenant_id: &str,
    user_id: &str,
    provider: &str,
    subject: &str,
    email: Option<&str>,
) -> Result<UserIdentity> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO user_identities (id, tenant_id, user_id, provider, subject, email, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(provider)
    .bind(subject)
    .bind(email)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(UserIdentity {
        id,
        tenant_id: tenant_id.to_string(),
        user_id: user_id.to_string(),
        provider: provider.to_string(),
        subject: subject.to_string(),
        email: email.map(str::to_string),
        created_at: now,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    db: &Database,
    tenant_id: &str,
    user_id: &str,
    token_hash: &str,
    csrf_hash: &str,
    mfa_verified: bool,
    user_agent: Option<&str>,
    ip: Option<&str>,
    expires_at: &str,
) -> Result<SessionRecord> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO sessions (id, tenant_id, user_id, token_hash, csrf_hash, mfa_verified, user_agent, ip, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(token_hash)
    .bind(csrf_hash)
    .bind(if mfa_verified { 1i64 } else { 0i64 })
    .bind(user_agent)
    .bind(ip)
    .bind(expires_at)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_session(db, &id).await
}

pub async fn get_session(db: &Database, id: &str) -> Result<SessionRecord> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, user_id, token_hash, csrf_hash, mfa_verified, user_agent, ip, expires_at, created_at, last_used_at, revoked_at
         FROM sessions WHERE id = ?",
    ))
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("session {id}")))?;
    session_from_row(&row)
}

pub async fn find_session_by_hash(
    db: &Database,
    token_hash: &str,
) -> Result<Option<SessionRecord>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, user_id, token_hash, csrf_hash, mfa_verified, user_agent, ip, expires_at, created_at, last_used_at, revoked_at
         FROM sessions WHERE token_hash = ?",
    ))
    .bind(token_hash)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| session_from_row(&row)).transpose()
}

fn session_from_row(row: &AnyRow) -> Result<SessionRecord> {
    Ok(SessionRecord {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        user_id: text(row, "user_id")?,
        token_hash: text(row, "token_hash")?,
        csrf_hash: text(row, "csrf_hash")?,
        mfa_verified: integer(row, "mfa_verified")? != 0,
        user_agent: opt_text(row, "user_agent")?,
        ip: opt_text(row, "ip")?,
        expires_at: text(row, "expires_at")?,
        created_at: text(row, "created_at")?,
        last_used_at: opt_text(row, "last_used_at")?,
        revoked_at: opt_text(row, "revoked_at")?,
    })
}

pub async fn touch_session(db: &Database, id: &str) -> Result<()> {
    sqlx::query(&db.sql("UPDATE sessions SET last_used_at = ? WHERE id = ?"))
        .bind(now_rfc3339())
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

pub async fn set_session_mfa_verified(db: &Database, id: &str, verified: bool) -> Result<()> {
    let result = sqlx::query(&db.sql("UPDATE sessions SET mfa_verified = ? WHERE id = ?"))
        .bind(if verified { 1i64 } else { 0i64 })
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("session {id}")));
    }
    Ok(())
}

pub async fn revoke_session(db: &Database, id: &str) -> Result<()> {
    let at = now_rfc3339();
    let result = sqlx::query(&db.sql("UPDATE sessions SET revoked_at = ? WHERE id = ?"))
        .bind(&at)
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("session {id}")));
    }
    Ok(())
}

pub async fn delete_expired_sessions(db: &Database, before: &str) -> Result<usize> {
    Ok(
        sqlx::query(&db.sql("DELETE FROM sessions WHERE expires_at < ?"))
            .bind(before)
            .execute(db.pool())
            .await?
            .rows_affected() as usize,
    )
}

pub async fn upsert_totp(
    db: &Database,
    tenant_id: &str,
    user_id: &str,
    secret_encrypted: &str,
    confirmed: bool,
) -> Result<MfaTotp> {
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO mfa_totp (tenant_id, user_id, secret_encrypted, confirmed, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (tenant_id, user_id) DO UPDATE SET secret_encrypted = excluded.secret_encrypted, confirmed = excluded.confirmed",
    ))
    .bind(tenant_id)
    .bind(user_id)
    .bind(secret_encrypted)
    .bind(if confirmed { 1i64 } else { 0i64 })
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_totp(db, tenant_id, user_id)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("totp for {user_id}")))
}

pub async fn get_totp(db: &Database, tenant_id: &str, user_id: &str) -> Result<Option<MfaTotp>> {
    let row = sqlx::query(&db.sql(
        "SELECT tenant_id, user_id, secret_encrypted, confirmed, created_at FROM mfa_totp WHERE tenant_id = ? AND user_id = ?",
    ))
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        Ok(MfaTotp {
            tenant_id: text(&row, "tenant_id")?,
            user_id: text(&row, "user_id")?,
            secret_encrypted: text(&row, "secret_encrypted")?,
            confirmed: integer(&row, "confirmed")? != 0,
            created_at: text(&row, "created_at")?,
        })
    })
    .transpose()
}

pub async fn set_totp_confirmed(
    db: &Database,
    tenant_id: &str,
    user_id: &str,
    confirmed: bool,
) -> Result<()> {
    let result = sqlx::query(
        &db.sql("UPDATE mfa_totp SET confirmed = ? WHERE tenant_id = ? AND user_id = ?"),
    )
    .bind(if confirmed { 1i64 } else { 0i64 })
    .bind(tenant_id)
    .bind(user_id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("totp for {user_id}")));
    }
    Ok(())
}

pub async fn delete_totp(db: &Database, tenant_id: &str, user_id: &str) -> Result<()> {
    let result = sqlx::query(&db.sql("DELETE FROM mfa_totp WHERE tenant_id = ? AND user_id = ?"))
        .bind(tenant_id)
        .bind(user_id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("totp for {user_id}")));
    }
    Ok(())
}

pub async fn update_user_profile(
    db: &Database,
    tenant_id: &str,
    id: &str,
    display_name: Option<&str>,
    role: Option<&str>,
    status: Option<&str>,
) -> Result<User> {
    let now = now_rfc3339();
    let result = sqlx::query(&db.sql(
        "UPDATE users SET display_name = COALESCE(?, display_name), role = COALESCE(?, role),
                          status = COALESCE(?, status), updated_at = ? WHERE tenant_id = ? AND id = ?",
    ))
    .bind(display_name)
    .bind(role)
    .bind(status)
    .bind(&now)
    .bind(tenant_id)
    .bind(id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("user {id}")));
    }
    get_user(db, tenant_id, id).await
}

pub async fn set_user_status(
    db: &Database,
    tenant_id: &str,
    id: &str,
    status: &str,
) -> Result<User> {
    update_user_profile(db, tenant_id, id, None, None, Some(status)).await
}

// ---------------------------------------------------------------------------
// Worker queue
// ---------------------------------------------------------------------------

pub async fn claim_next_workflow_run(
    db: &Database,
    worker_id: &str,
    stale_before: &str,
) -> Result<Option<WorkflowRun>> {
    for _ in 0..5 {
        let row = sqlx::query(&db.sql(
            "SELECT id, tenant_id FROM workflow_runs
             WHERE status = 'queued' AND (locked_at IS NULL OR locked_at < ?)
               AND NOT EXISTS (
                   SELECT 1 FROM connector_jobs cj
                   WHERE cj.run_id = workflow_runs.id AND cj.status IN ('queued', 'running')
               )
             ORDER BY created_at ASC LIMIT 1",
        ))
        .bind(stale_before)
        .fetch_optional(db.pool())
        .await?;
        let Some(row) = row else { return Ok(None) };
        let id = text(&row, "id")?;
        let tenant_id = text(&row, "tenant_id")?;
        let now = now_rfc3339();
        let result = sqlx::query(&db.sql(
            "UPDATE workflow_runs SET status = 'running', locked_at = ?, locked_by = ?,
                    attempts = attempts + 1, started_at = COALESCE(started_at, ?)
             WHERE id = ? AND status = 'queued'",
        ))
        .bind(&now)
        .bind(worker_id)
        .bind(&now)
        .bind(&id)
        .execute(db.pool())
        .await?;
        if result.rows_affected() == 1 {
            return Ok(Some(get_workflow_run(db, &tenant_id, &id).await?));
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(None)
}

pub async fn claim_next_task(
    db: &Database,
    worker_id: &str,
    stale_before: &str,
) -> Result<Option<Task>> {
    for _ in 0..5 {
        let row = sqlx::query(&db.sql(
            "SELECT id, tenant_id FROM tasks
             WHERE status = 'queued' AND (locked_at IS NULL OR locked_at < ?)
               AND NOT EXISTS (
                   SELECT 1 FROM connector_jobs cj
                   WHERE cj.task_id = tasks.id AND cj.status IN ('queued', 'running')
               )
             ORDER BY created_at ASC LIMIT 1",
        ))
        .bind(stale_before)
        .fetch_optional(db.pool())
        .await?;
        let Some(row) = row else { return Ok(None) };
        let id = text(&row, "id")?;
        let tenant_id = text(&row, "tenant_id")?;
        let now = now_rfc3339();
        let result = sqlx::query(&db.sql(
            "UPDATE tasks SET status = 'running', locked_at = ?, locked_by = ?,
                    attempts = attempts + 1, started_at = COALESCE(started_at, ?)
             WHERE id = ? AND status = 'queued'",
        ))
        .bind(&now)
        .bind(worker_id)
        .bind(&now)
        .bind(&id)
        .execute(db.pool())
        .await?;
        if result.rows_affected() == 1 {
            return Ok(Some(get_task(db, &tenant_id, &id).await?));
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(None)
}

pub async fn release_stale_workflow_runs(db: &Database, stale_before: &str) -> Result<usize> {
    Ok(sqlx::query(&db.sql(
        "UPDATE workflow_runs SET status = 'queued', locked_at = NULL, locked_by = NULL
         WHERE status = 'running' AND locked_at < ?",
    ))
    .bind(stale_before)
    .execute(db.pool())
    .await?
    .rows_affected() as usize)
}

pub async fn release_stale_tasks(db: &Database, stale_before: &str) -> Result<usize> {
    Ok(sqlx::query(&db.sql(
        "UPDATE tasks SET status = 'queued', locked_at = NULL, locked_by = NULL
         WHERE status = 'running' AND locked_at < ?",
    ))
    .bind(stale_before)
    .execute(db.pool())
    .await?
    .rows_affected() as usize)
}

pub async fn heartbeat_workflow_run(db: &Database, id: &str) -> Result<()> {
    sqlx::query(&db.sql("UPDATE workflow_runs SET locked_at = ? WHERE id = ?"))
        .bind(now_rfc3339())
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

pub async fn heartbeat_task(db: &Database, id: &str) -> Result<()> {
    sqlx::query(&db.sql("UPDATE tasks SET locked_at = ? WHERE id = ?"))
        .bind(now_rfc3339())
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

pub async fn clear_workflow_run_lease(db: &Database, id: &str) -> Result<()> {
    sqlx::query(
        &db.sql("UPDATE workflow_runs SET locked_at = NULL, locked_by = NULL WHERE id = ?"),
    )
    .bind(id)
    .execute(db.pool())
    .await?;
    Ok(())
}

pub async fn clear_task_lease(db: &Database, id: &str) -> Result<()> {
    sqlx::query(&db.sql("UPDATE tasks SET locked_at = NULL, locked_by = NULL WHERE id = ?"))
        .bind(id)
        .execute(db.pool())
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

pub async fn create_approval(
    db: &Database,
    tenant_id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    step_id: &str,
    summary: &str,
    reason: &str,
) -> Result<ApprovalRecord> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO approvals (id, tenant_id, run_id, task_id, step_id, summary, reason, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(run_id)
    .bind(task_id)
    .bind(step_id)
    .bind(summary)
    .bind(reason)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_approval(db, tenant_id, &id).await
}

pub async fn get_approval(db: &Database, tenant_id: &str, id: &str) -> Result<ApprovalRecord> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, task_id, step_id, summary, reason, status, created_at, resolved_at, resolved_by
         FROM approvals WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("approval {id}")))?;
    approval_from_row(&row)
}

pub async fn list_approvals(
    db: &Database,
    tenant_id: &str,
    status: Option<&str>,
    run_id: Option<&str>,
    task_id: Option<&str>,
    limit: i64,
) -> Result<Vec<ApprovalRecord>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, run_id, task_id, step_id, summary, reason, status, created_at, resolved_at, resolved_by
         FROM approvals
         WHERE tenant_id = ?
           AND (? IS NULL OR status = ?)
           AND (? IS NULL OR run_id = ?)
           AND (? IS NULL OR task_id = ?)
         ORDER BY created_at ASC LIMIT ?",
    ))
    .bind(tenant_id)
    .bind(status)
    .bind(status)
    .bind(run_id)
    .bind(run_id)
    .bind(task_id)
    .bind(task_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(approval_from_row).collect()
}

pub async fn resolve_approval(
    db: &Database,
    tenant_id: &str,
    id: &str,
    status: &str,
    resolved_by: Option<&str>,
) -> Result<ApprovalRecord> {
    if !matches!(status, "approved" | "denied") {
        return Err(StoreError::Invalid(
            "approval status must be approved or denied".into(),
        ));
    }
    let now = now_rfc3339();
    let result = sqlx::query(&db.sql(
        "UPDATE approvals SET status = ?, resolved_at = ?, resolved_by = ?
         WHERE tenant_id = ? AND id = ? AND status = 'pending'",
    ))
    .bind(status)
    .bind(&now)
    .bind(resolved_by)
    .bind(tenant_id)
    .bind(id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::Conflict(
            "approval is not pending or does not exist".into(),
        ));
    }
    get_approval(db, tenant_id, id).await
}

fn approval_from_row(row: &AnyRow) -> Result<ApprovalRecord> {
    Ok(ApprovalRecord {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        run_id: opt_text(row, "run_id")?,
        task_id: opt_text(row, "task_id")?,
        step_id: text(row, "step_id")?,
        summary: text(row, "summary")?,
        reason: text(row, "reason")?,
        status: text(row, "status")?,
        created_at: text(row, "created_at")?,
        resolved_at: opt_text(row, "resolved_at")?,
        resolved_by: opt_text(row, "resolved_by")?,
    })
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

pub async fn due_schedules(db: &Database, now: &str) -> Result<Vec<Schedule>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, workflow_id, cron, timezone, enabled, next_run_at, last_run_at, last_status, created_at
         FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?
         ORDER BY next_run_at ASC",
    ))
    .bind(now)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(schedule_from_row).collect()
}

pub async fn mark_schedule_run(
    db: &Database,
    id: &str,
    next_run: &str,
    last_run: &str,
    last_status: &str,
) -> Result<()> {
    sqlx::query(&db.sql(
        "UPDATE schedules SET next_run_at = ?, last_run_at = ?, last_status = ? WHERE id = ?",
    ))
    .bind(next_run)
    .bind(last_run)
    .bind(last_status)
    .bind(id)
    .execute(db.pool())
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

pub async fn create_webhook(
    db: &Database,
    tenant_id: &str,
    url: &str,
    secret_encrypted: &str,
    events: &[String],
) -> Result<Webhook> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO webhooks (id, tenant_id, url, secret_encrypted, events, enabled, created_at)
         VALUES (?, ?, ?, ?, ?, 1, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(url)
    .bind(secret_encrypted)
    .bind(encode(&serde_json::json!(events))?)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_webhook(db, tenant_id, &id).await
}

pub async fn get_webhook(db: &Database, tenant_id: &str, id: &str) -> Result<Webhook> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, url, secret_encrypted, events, enabled, created_at
         FROM webhooks WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("webhook {id}")))?;
    webhook_from_row(&row)
}

pub async fn delete_webhook(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let result = sqlx::query(&db.sql("DELETE FROM webhooks WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(id)
        .execute(db.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("webhook {id}")));
    }
    Ok(())
}

pub async fn enabled_webhooks_for_event(
    db: &Database,
    tenant_id: &str,
    event: &str,
) -> Result<Vec<Webhook>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, url, secret_encrypted, events, enabled, created_at
         FROM webhooks WHERE tenant_id = ? AND enabled = 1 ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    let mut webhooks = Vec::new();
    for row in rows {
        let webhook = webhook_from_row(&row)?;
        if webhook
            .events
            .iter()
            .any(|value| value == event || value == "*")
        {
            webhooks.push(webhook);
        }
    }
    Ok(webhooks)
}

fn webhook_from_row(row: &AnyRow) -> Result<Webhook> {
    Ok(Webhook {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        url: text(row, "url")?,
        secret_encrypted: text(row, "secret_encrypted")?,
        events: strings_opt(row, "events")?,
        enabled: integer(row, "enabled")? != 0,
        created_at: text(row, "created_at")?,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn record_webhook_delivery(
    db: &Database,
    tenant_id: &str,
    webhook_id: &str,
    event: &str,
    payload: &Value,
    status: &str,
    attempts: i64,
    response_code: Option<i64>,
    last_error: Option<&str>,
) -> Result<String> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO webhook_deliveries (id, tenant_id, webhook_id, event, payload, status, attempts, response_code, last_error, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(webhook_id)
    .bind(event)
    .bind(encode(payload)?)
    .bind(status)
    .bind(attempts)
    .bind(response_code)
    .bind(last_error)
    .bind(&now)
    .execute(db.pool())
    .await?;
    Ok(id)
}

pub async fn update_webhook_delivery(
    db: &Database,
    id: &str,
    status: &str,
    attempts: i64,
    response_code: Option<i64>,
    last_error: Option<&str>,
) -> Result<()> {
    sqlx::query(&db.sql(
        "UPDATE webhook_deliveries SET status = ?, attempts = ?, response_code = ?, last_error = ? WHERE id = ?",
    ))
    .bind(status)
    .bind(attempts)
    .bind(response_code)
    .bind(last_error)
    .bind(id)
    .execute(db.pool())
    .await?;
    Ok(())
}

pub async fn list_webhook_deliveries(
    db: &Database,
    tenant_id: &str,
    webhook_id: &str,
    limit: i64,
) -> Result<Vec<WebhookDelivery>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, webhook_id, event, payload, status, attempts, response_code, last_error, created_at
         FROM webhook_deliveries WHERE tenant_id = ? AND webhook_id = ? ORDER BY created_at DESC LIMIT ?",
    ))
    .bind(tenant_id)
    .bind(webhook_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(WebhookDelivery {
                id: text(row, "id")?,
                tenant_id: text(row, "tenant_id")?,
                webhook_id: text(row, "webhook_id")?,
                event: text(row, "event")?,
                payload: json_value(row, "payload")?,
                status: text(row, "status")?,
                attempts: integer(row, "attempts")?,
                response_code: opt_i64(row, "response_code")?,
                last_error: opt_text(row, "last_error")?,
                created_at: text(row, "created_at")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Hybrid connectors and outbound job leases
// ---------------------------------------------------------------------------

fn connector_from_row(row: &AnyRow) -> Result<Connector> {
    Ok(Connector {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        name: text(row, "name")?,
        region: text(row, "region")?,
        status: text(row, "status")?,
        version: opt_text(row, "version")?,
        last_seen_at: opt_text(row, "last_seen_at")?,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

fn connector_job_from_row(row: &AnyRow) -> Result<ConnectorJob> {
    Ok(ConnectorJob {
        id: text(row, "id")?,
        tenant_id: text(row, "tenant_id")?,
        connector_id: text(row, "connector_id")?,
        kind: text(row, "kind")?,
        run_id: opt_text(row, "run_id")?,
        task_id: opt_text(row, "task_id")?,
        payload: json_value(row, "payload")?,
        status: text(row, "status")?,
        attempts: integer(row, "attempts")?,
        result: json_opt(row, "result")?,
        error: opt_text(row, "error")?,
        created_at: text(row, "created_at")?,
        updated_at: text(row, "updated_at")?,
    })
}

async fn connector_for_tenant_tx(
    db: &Database,
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    tenant_id: &str,
) -> Result<Option<Connector>> {
    let row =
        sqlx::query(&db.sql(
            "SELECT value FROM settings WHERE tenant_id = ? AND key = 'execution.connector_id'",
        ))
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let connector_id = text(&row, "value")?;
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, region, status, version, last_seen_at, created_at, updated_at
         FROM connectors WHERE id = ? AND tenant_id = ? AND status = 'active'",
    ))
    .bind(&connector_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some(row) => Ok(Some(connector_from_row(&row)?)),
        None => Err(StoreError::Invalid(format!(
            "execution connector {connector_id} is not active"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_connector_job_tx(
    db: &Database,
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    tenant_id: &str,
    connector_id: &str,
    kind: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    payload: &Value,
    now: &str,
) -> Result<()> {
    sqlx::query(&db.sql(
        "INSERT INTO connector_jobs (id, tenant_id, connector_id, kind, run_id, task_id, payload,
                                     status, attempts, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', 0, ?, ?)",
    ))
    .bind(new_id())
    .bind(tenant_id)
    .bind(connector_id)
    .bind(kind)
    .bind(run_id)
    .bind(task_id)
    .bind(encode(payload)?)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn create_connector(
    db: &Database,
    tenant_id: &str,
    name: &str,
    prefix: &str,
    token_hash: &str,
    region: &str,
) -> Result<Connector> {
    require_tenant(tenant_id)?;
    if name.trim().is_empty() {
        return Err(StoreError::Invalid(
            "connector name must not be empty".into(),
        ));
    }
    if prefix.trim().is_empty() || token_hash.trim().is_empty() {
        return Err(StoreError::Invalid(
            "connector token material must not be empty".into(),
        ));
    }
    let region = region.trim();
    if region.is_empty() {
        return Err(StoreError::Invalid(
            "connector region must not be empty".into(),
        ));
    }
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(&db.sql(
        "INSERT INTO connectors (id, tenant_id, name, prefix, token_hash, region, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?)",
    ))
    .bind(&id)
    .bind(tenant_id)
    .bind(name.trim())
    .bind(prefix)
    .bind(token_hash)
    .bind(region)
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await?;
    get_connector(db, tenant_id, &id).await
}

pub async fn get_connector(db: &Database, tenant_id: &str, id: &str) -> Result<Connector> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, region, status, version, last_seen_at, created_at, updated_at
         FROM connectors WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("connector {id}")))?;
    connector_from_row(&row)
}

pub async fn find_connector_by_prefix(
    db: &Database,
    prefix: &str,
) -> Result<Option<(Connector, String)>> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, region, status, version, last_seen_at, created_at, updated_at, token_hash
         FROM connectors WHERE prefix = ?",
    ))
    .bind(prefix)
    .fetch_optional(db.pool())
    .await?;
    row.map(|row| {
        let hash = text(&row, "token_hash")?;
        Ok((connector_from_row(&row)?, hash))
    })
    .transpose()
}

pub async fn list_connectors(db: &Database, tenant_id: &str) -> Result<Vec<Connector>> {
    let rows = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, region, status, version, last_seen_at, created_at, updated_at
         FROM connectors WHERE tenant_id = ? ORDER BY created_at ASC",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    rows.iter().map(connector_from_row).collect()
}

pub async fn delete_connector(db: &Database, tenant_id: &str, id: &str) -> Result<()> {
    let now = now_rfc3339();
    let mut tx = db.pool().begin().await?;
    let rows = sqlx::query(&db.sql(
        "SELECT run_id, task_id FROM connector_jobs
         WHERE tenant_id = ? AND connector_id = ? AND status IN ('queued', 'running')",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    for row in rows {
        if let Some(run_id) = opt_text(&row, "run_id")? {
            sqlx::query(&db.sql(
                "UPDATE workflow_runs
                 SET status = 'cancelled', error = 'execution connector was deleted',
                     finished_at = COALESCE(finished_at, ?)
                 WHERE tenant_id = ? AND id = ? AND status NOT IN ('completed', 'failed', 'cancelled')",
            ))
            .bind(&now)
            .bind(tenant_id)
            .bind(&run_id)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(task_id) = opt_text(&row, "task_id")? {
            sqlx::query(&db.sql(
                "UPDATE tasks
                 SET status = 'cancelled', failure_reason = 'execution connector was deleted',
                     finished_at = COALESCE(finished_at, ?)
                 WHERE tenant_id = ? AND id = ? AND status NOT IN ('completed', 'failed', 'cancelled')",
            ))
            .bind(&now)
            .bind(tenant_id)
            .bind(&task_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query(&db.sql(
        "UPDATE connector_jobs SET status = 'cancelled', locked_until = NULL, updated_at = ?
         WHERE tenant_id = ? AND connector_id = ? AND status IN ('queued', 'running')",
    ))
    .bind(&now)
    .bind(tenant_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query(&db.sql("DELETE FROM connectors WHERE tenant_id = ? AND id = ?"))
        .bind(tenant_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("connector {id}")));
    }
    tx.commit().await?;
    Ok(())
}

pub async fn heartbeat_connector(
    db: &Database,
    connector_id: &str,
    version: Option<&str>,
) -> Result<Connector> {
    let now = now_rfc3339();
    let result = sqlx::query(&db.sql(
        "UPDATE connectors SET last_seen_at = ?, version = COALESCE(?, version), updated_at = ?
         WHERE id = ? AND status = 'active'",
    ))
    .bind(&now)
    .bind(version)
    .bind(&now)
    .bind(connector_id)
    .execute(db.pool())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("connector {connector_id}")));
    }
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, name, region, status, version, last_seen_at, created_at, updated_at
         FROM connectors WHERE id = ?",
    ))
    .bind(connector_id)
    .fetch_one(db.pool())
    .await?;
    connector_from_row(&row)
}

pub async fn pending_connector_jobs(db: &Database, connector_id: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql(
        "SELECT COUNT(*) AS n FROM connector_jobs
         WHERE connector_id = ? AND status IN ('queued', 'running')",
    ))
    .bind(connector_id)
    .fetch_one(db.pool())
    .await?;
    integer(&row, "n")
}

pub async fn claim_connector_job(
    db: &Database,
    connector: &Connector,
    lease_secs: i64,
) -> Result<Option<ConnectorJob>> {
    let lease_secs = lease_secs.clamp(30, 3600);
    let now = now_rfc3339();
    let locked_until = (chrono::Utc::now() + chrono::Duration::seconds(lease_secs))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    for _ in 0..5 {
        let row = sqlx::query(&db.sql(
            "SELECT id FROM connector_jobs
             WHERE tenant_id = ? AND connector_id = ? AND status IN ('queued', 'running')
               AND (status = 'queued' OR locked_until IS NULL OR locked_until < ?)
             ORDER BY created_at ASC LIMIT 1",
        ))
        .bind(&connector.tenant_id)
        .bind(&connector.id)
        .bind(&now)
        .fetch_optional(db.pool())
        .await?;
        let Some(row) = row else { return Ok(None) };
        let id = text(&row, "id")?;
        let result = sqlx::query(&db.sql(
            "UPDATE connector_jobs
             SET status = 'running', attempts = attempts + 1, locked_at = ?, locked_until = ?, updated_at = ?
             WHERE id = ? AND connector_id = ?
               AND (status = 'queued' OR (status = 'running' AND (locked_until IS NULL OR locked_until < ?)))",
        ))
        .bind(&now)
        .bind(&locked_until)
        .bind(&now)
        .bind(&id)
        .bind(&connector.id)
        .bind(&now)
        .execute(db.pool())
        .await?;
        if result.rows_affected() == 0 {
            continue;
        }
        let job = connector_job(db, &connector.tenant_id, &id).await?;
        if let Some(run_id) = &job.run_id {
            let run = get_workflow_run(db, &connector.tenant_id, run_id).await?;
            if run.status == "cancelled" {
                cancel_connector_job(db, &connector.tenant_id, &id).await?;
                continue;
            }
            if run.status == "queued" {
                let _ = set_workflow_run_status(
                    db,
                    &connector.tenant_id,
                    run_id,
                    "running",
                    None,
                    None,
                    Some(&now),
                    None,
                )
                .await?;
            }
        }
        if let Some(task_id) = &job.task_id {
            let task = get_task(db, &connector.tenant_id, task_id).await?;
            if task.status == "cancelled" {
                cancel_connector_job(db, &connector.tenant_id, &id).await?;
                continue;
            }
            if task.status == "queued" {
                let _ = set_task_status(
                    db,
                    &connector.tenant_id,
                    task_id,
                    TaskStatus::Running,
                    None,
                    None,
                    Some(&now),
                    None,
                )
                .await?;
            }
        }
        return Ok(Some(job));
    }
    Ok(None)
}

pub async fn connector_job(db: &Database, tenant_id: &str, id: &str) -> Result<ConnectorJob> {
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, connector_id, kind, run_id, task_id, payload, status, attempts,
                result, error, created_at, updated_at
         FROM connector_jobs WHERE tenant_id = ? AND id = ?",
    ))
    .bind(tenant_id)
    .bind(id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("connector job {id}")))?;
    connector_job_from_row(&row)
}

pub async fn extend_connector_job(
    db: &Database,
    connector_id: &str,
    job_id: &str,
    lease_secs: i64,
) -> Result<bool> {
    let locked_until = (chrono::Utc::now() + chrono::Duration::seconds(lease_secs.clamp(30, 3600)))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let result = sqlx::query(&db.sql(
        "UPDATE connector_jobs SET locked_until = ?, updated_at = ?
         WHERE id = ? AND connector_id = ? AND status = 'running'",
    ))
    .bind(&locked_until)
    .bind(now_rfc3339())
    .bind(job_id)
    .bind(connector_id)
    .execute(db.pool())
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn complete_connector_job(
    db: &Database,
    connector_id: &str,
    job_id: &str,
    status: &str,
    result: Option<&Value>,
    error: Option<&str>,
) -> Result<ConnectorJob> {
    if !matches!(status, "completed" | "failed" | "cancelled") {
        return Err(StoreError::Invalid(format!(
            "invalid connector job status {status}"
        )));
    }
    let row = sqlx::query(&db.sql(
        "SELECT id, tenant_id, connector_id, kind, run_id, task_id, payload, status, attempts,
                result, error, created_at, updated_at
         FROM connector_jobs WHERE id = ? AND connector_id = ?",
    ))
    .bind(job_id)
    .bind(connector_id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("connector job {job_id}")))?;
    let job = connector_job_from_row(&row)?;
    if matches!(job.status.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(job);
    }
    let now = now_rfc3339();
    let encoded = result.map(encode).transpose()?;
    sqlx::query(&db.sql(
        "UPDATE connector_jobs SET status = ?, result = ?, error = ?, locked_until = NULL, updated_at = ?
         WHERE id = ? AND connector_id = ?",
    ))
    .bind(status)
    .bind(encoded)
    .bind(error)
    .bind(&now)
    .bind(job_id)
    .bind(connector_id)
    .execute(db.pool())
    .await?;

    if let Some(run_id) = &job.run_id {
        let run = get_workflow_run(db, &job.tenant_id, run_id).await?;
        if run.status != "cancelled" {
            let _ = set_workflow_run_status(
                db,
                &job.tenant_id,
                run_id,
                status,
                result,
                error,
                None,
                Some(&now),
            )
            .await?;
        }
    }
    if let Some(task_id) = &job.task_id {
        let task = get_task(db, &job.tenant_id, task_id).await?;
        if task.status != "cancelled" {
            let task_status = match status {
                "completed" => TaskStatus::Completed,
                "cancelled" => TaskStatus::Cancelled,
                _ => TaskStatus::Failed,
            };
            let _ = set_task_status(
                db,
                &job.tenant_id,
                task_id,
                task_status,
                result,
                error,
                None,
                Some(&now),
            )
            .await?;
        }
    }
    connector_job(db, &job.tenant_id, job_id).await
}

async fn cancel_connector_job(db: &Database, tenant_id: &str, job_id: &str) -> Result<()> {
    sqlx::query(&db.sql(
        "UPDATE connector_jobs SET status = 'cancelled', locked_until = NULL, updated_at = ?
         WHERE tenant_id = ? AND id = ? AND status IN ('queued', 'running')",
    ))
    .bind(now_rfc3339())
    .bind(tenant_id)
    .bind(job_id)
    .execute(db.pool())
    .await?;
    Ok(())
}

pub async fn cancel_connector_jobs_for_run(
    db: &Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<()> {
    sqlx::query(&db.sql(
        "UPDATE connector_jobs SET status = 'cancelled', locked_until = NULL, updated_at = ?
         WHERE tenant_id = ? AND run_id = ? AND status IN ('queued', 'running')",
    ))
    .bind(now_rfc3339())
    .bind(tenant_id)
    .bind(run_id)
    .execute(db.pool())
    .await?;
    Ok(())
}

pub async fn cancel_connector_jobs_for_task(
    db: &Database,
    tenant_id: &str,
    task_id: &str,
) -> Result<()> {
    sqlx::query(&db.sql(
        "UPDATE connector_jobs SET status = 'cancelled', locked_until = NULL, updated_at = ?
         WHERE tenant_id = ? AND task_id = ? AND status IN ('queued', 'running')",
    ))
    .bind(now_rfc3339())
    .bind(tenant_id)
    .bind(task_id)
    .execute(db.pool())
    .await?;
    Ok(())
}
