//! Persisted domain records shared by the API, worker and CLI.
//!
//! Timestamps are RFC 3339 UTC strings. This keeps repository code portable
//! across SQLite and Postgres; the API layer may parse them into richer types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow::WorkflowDefinition;

// ---------------------------------------------------------------------------
// Identity and tenancy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    Suspended,
    Deleted,
}

impl TenantStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
            TenantStatus::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub status: String,
    pub region: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Operator,
    Viewer,
    Auditor,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Viewer => "viewer",
            Role::Auditor => "auditor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "operator" => Some(Role::Operator),
            "viewer" => Some(Role::Viewer),
            "auditor" => Some(Role::Auditor),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Suspended,
    Deleted,
}

impl UserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserStatus::Active => "active",
            UserStatus::Suspended => "suspended",
            UserStatus::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub tenant_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyRecord {
    pub id: String,
    pub tenant_id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub prefix: String,
    pub key_hash: String,
    pub scopes: Vec<String>,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Tasks (natural-language browser runs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    AwaitingApproval,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Queued => "queued",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::AwaitingApproval => "awaiting_approval",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub tenant_id: String,
    pub created_by: Option<String>,
    pub prompt: String,
    pub url: Option<String>,
    pub navigation_goal: Option<String>,
    pub data_schema: Option<Value>,
    pub error_codes: Vec<String>,
    pub status: String,
    pub output: Option<Value>,
    pub failure_reason: Option<String>,
    pub model: Option<String>,
    pub max_steps: u32,
    pub browser_session_id: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAction {
    pub id: String,
    pub tenant_id: String,
    pub task_id: String,
    pub seq: i64,
    pub action: Value,
    pub reasoning: Option<String>,
    pub screenshot_artifact_id: Option<String>,
    pub status: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecord {
    pub id: String,
    pub tenant_id: String,
    pub slug: String,
    pub name: String,
    pub current_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowVersionRecord {
    pub id: String,
    pub tenant_id: String,
    pub workflow_id: String,
    pub version: i64,
    pub definition: WorkflowDefinition,
    pub reason: String,
    pub created_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    AwaitingApproval,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::AwaitingApproval => "awaiting_approval",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub tenant_id: String,
    pub workflow_id: String,
    pub workflow_version: i64,
    pub status: String,
    pub inputs: Value,
    pub outputs: Option<Value>,
    pub error: Option<String>,
    pub trigger: String,
    pub parent_run_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBlockRun {
    pub id: String,
    pub tenant_id: String,
    pub run_id: String,
    pub block_id: String,
    pub seq: i64,
    pub status: String,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub attempt: i64,
    pub created_at: String,
    pub finished_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Artifacts, sessions, credentials
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Screenshot,
    Video,
    Har,
    Download,
    Dom,
    Console,
    Extracted,
    Other,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Screenshot => "screenshot",
            ArtifactKind::Video => "video",
            ArtifactKind::Har => "har",
            ArtifactKind::Download => "download",
            ArtifactKind::Dom => "dom",
            ArtifactKind::Console => "console",
            ArtifactKind::Extracted => "extracted",
            ArtifactKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub tenant_id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub block_run_id: Option<String>,
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub storage_key: String,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    UsernamePassword,
    Totp,
    ApiKey,
    CreditCard,
    Custom,
}

impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialKind::UsernamePassword => "username_password",
            CredentialKind::Totp => "totp",
            CredentialKind::ApiKey => "api_key",
            CredentialKind::CreditCard => "credit_card",
            CredentialKind::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "username_password" | "username-password" | "password" => Some(Self::UsernamePassword),
            "totp" => Some(Self::Totp),
            "api_key" | "api-key" => Some(Self::ApiKey),
            "credit_card" | "credit-card" | "card" => Some(Self::CreditCard),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub kind: String,
    /// `local`, `bitwarden`, `1password`, `lastpass` or `http`.
    pub provider: String,
    /// Item identifier for external providers.
    pub external_ref: Option<String>,
    pub username: Option<String>,
    /// AES-GCM token; never returned by the API or written to logs.
    pub secret_encrypted: String,
    pub totp_secret_encrypted: Option<String>,
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub user_data_dir: Option<String>,
    pub cdp_url: Option<String>,
    pub proxy_url: Option<String>,
    pub created_at: String,
    pub closed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Scheduling and integrations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub tenant_id: String,
    pub workflow_id: String,
    pub cron: String,
    pub timezone: String,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Webhook {
    pub id: String,
    pub tenant_id: String,
    pub url: String,
    pub secret_encrypted: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDelivery {
    pub id: String,
    pub tenant_id: String,
    pub webhook_id: String,
    pub event: String,
    pub payload: Value,
    pub status: String,
    pub attempts: i64,
    pub response_code: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub region: String,
    pub status: String,
    pub version: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorJob {
    pub id: String,
    pub tenant_id: String,
    pub connector_id: String,
    pub kind: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub payload: Value,
    pub status: String,
    pub attempts: i64,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfig {
    pub id: String,
    pub tenant_id: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    /// Encrypted; write-only through the API.
    pub api_key_encrypted: Option<String>,
    pub capabilities: Value,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub tenant_id: String,
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLog {
    pub id: String,
    pub tenant_id: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub target: String,
    pub metadata: Value,
    pub prev_hash: String,
    pub hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCacheEntry {
    pub tenant_id: String,
    pub cache_key: String,
    pub value: Value,
    pub expires_at: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Identity, sessions, MFA and SSO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProvider {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub kind: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret_encrypted: Option<String>,
    pub scopes: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIdentity {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub token_hash: String,
    pub csrf_hash: String,
    pub mfa_verified: bool,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub expires_at: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcStateRecord {
    pub id: String,
    pub tenant_id: String,
    pub provider_id: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier_encrypted: String,
    pub redirect_uri: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaTotp {
    pub tenant_id: String,
    pub user_id: String,
    pub secret_encrypted: String,
    pub confirmed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    pub id: String,
    pub tenant_id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub step_id: String,
    pub summary: String,
    pub reason: String,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
}
