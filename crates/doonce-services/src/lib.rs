//! Concrete implementations of the engine's service traits:
//! AI (agent + LLM), credentials (DB vault + TOTP), SMTP email, local artifact
//! storage and database-backed nested workflows.

#![forbid(unsafe_code)]

pub mod providers;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use doonce_agent::{
    ApprovalRequest as AgentApproval, AutoApprove, TaskEvent, TaskObserver, TaskOptions, TaskSpec,
    TaskStatus, act as agent_act, extract as agent_extract, prompt as agent_prompt, run_task,
    validate as agent_validate,
};
use doonce_browser::BrowserSession;
use doonce_engine::{
    AiExecutor, AiTaskRequest, ApprovalCallback, ArtifactStore, CredentialResolver, EmailMessage,
    EmailSender, ResolvedCredential, Services, WorkflowResolver,
};
use doonce_llm::{CompletionRequest, LlmProvider, Message, Provider, ProviderConfig, Role};
use doonce_security::Encryptor;
use doonce_store::{Database, repo};
use futures_util::future::BoxFuture;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, Message as EmailBuilder, MultiPart, SinglePart};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{AsyncTransport, Tokio1Executor};
use serde_json::{Value, json};
use totp_rs::{Algorithm, Secret, TOTP};

use crate::providers::{CredentialProviders, ExternalCredential};

// ---------------------------------------------------------------------------
// AI service
// ---------------------------------------------------------------------------

pub struct AiService {
    provider: Arc<Provider>,
    options: TaskOptions,
}

impl AiService {
    pub fn from_env(options: TaskOptions) -> Option<Self> {
        let config = ProviderConfig::from_env().ok()?;
        let provider = Provider::new(config).ok()?;
        Some(AiService {
            provider: Arc::new(provider),
            options,
        })
    }

    pub fn new(provider: Provider, options: TaskOptions) -> Self {
        AiService {
            provider: Arc::new(provider),
            options,
        }
    }
}

struct ApprovalBridge<'a> {
    approvals: &'a dyn ApprovalCallback,
}

impl TaskObserver for ApprovalBridge<'_> {
    fn on_event(&self, _: &TaskEvent) {}

    async fn approve(&self, request: AgentApproval) -> bool {
        self.approvals
            .approve(doonce_engine::ApprovalRequest {
                block_id: "browserTask".into(),
                summary: request.summary,
                reason: request.reason,
            })
            .await
    }
}

impl AiExecutor for AiService {
    fn run_task<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        request: AiTaskRequest<'a>,
        approvals: &'a dyn ApprovalCallback,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            let spec = TaskSpec {
                prompt: request.prompt.to_string(),
                url: request.url.map(str::to_string),
                data_schema: request.data_schema.cloned(),
                error_codes: request.error_codes.to_vec(),
                max_steps: request.max_steps,
                model: request.model.map(str::to_string),
            };
            let options = TaskOptions {
                max_steps: request.max_steps,
                ..self.options.clone()
            };
            let observer = ApprovalBridge { approvals };
            let outcome =
                run_task(self.provider.as_ref(), &spec, browser, &observer, &options).await;
            match outcome.status {
                TaskStatus::Completed => Ok(outcome
                    .output
                    .unwrap_or_else(|| json!({ "completed": true }))),
                TaskStatus::Cancelled => Err("task was cancelled".into()),
                TaskStatus::Failed => Err(outcome
                    .failure
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "task failed".into())),
            }
        })
    }

    fn extract<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
        schema: &'a Value,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            agent_extract(
                self.provider.as_ref(),
                browser,
                prompt,
                schema,
                &self.options,
            )
            .await
            .map_err(|error| error.to_string())
        })
    }

    fn validate<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
    ) -> BoxFuture<'a, std::result::Result<bool, String>> {
        Box::pin(async move {
            agent_validate(self.provider.as_ref(), browser, prompt, &self.options)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn prompt<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
        schema: Option<&'a Value>,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            agent_prompt(
                self.provider.as_ref(),
                browser,
                prompt,
                schema,
                &self.options,
            )
            .await
            .map_err(|error| error.to_string())
        })
    }

    fn text_prompt<'a>(
        &'a self,
        prompt: &'a str,
        model: Option<&'a str>,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            let _ = model;
            let request = CompletionRequest {
                messages: vec![
                    Message::text(
                        Role::System,
                        "You are a precise assistant. Return only the requested output.",
                    ),
                    Message::text(Role::User, prompt),
                ],
                response_schema: None,
            };
            let completion = self
                .provider
                .complete(&request)
                .await
                .map_err(|error| error.to_string())?;
            Ok(doonce_agent::extract_json(&completion.text)
                .unwrap_or(Value::String(completion.text)))
        })
    }

    fn download_files<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            let dir = browser.download_dir().to_path_buf();
            let before = list_files(&dir);
            let outcome = agent_act(
                self.provider.as_ref(),
                prompt,
                browser,
                &AutoApprove,
                &self.options,
            )
            .await;
            if outcome.status != TaskStatus::Completed {
                return Err(outcome
                    .failure
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "download task failed".into()));
            }
            let after = list_files(&dir);
            let files: Vec<String> = after
                .difference(&before)
                .map(|path| path.display().to_string())
                .collect();
            Ok(json!({ "status": "completed", "files": files, "downloadDir": dir }))
        })
    }
}

fn list_files(dir: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect()
}

// ---------------------------------------------------------------------------
// Credentials + TOTP
// ---------------------------------------------------------------------------

pub struct CredentialService {
    db: Database,
    encryptor: Encryptor,
    tenant_id: String,
    providers: Arc<CredentialProviders>,
}

impl CredentialService {
    pub fn new(db: Database, encryptor: Encryptor, tenant_id: String) -> Self {
        CredentialService {
            db,
            encryptor,
            tenant_id,
            providers: Arc::new(CredentialProviders::from_env()),
        }
    }

    pub fn with_providers(
        db: Database,
        encryptor: Encryptor,
        tenant_id: String,
        providers: CredentialProviders,
    ) -> Self {
        CredentialService {
            db,
            encryptor,
            tenant_id,
            providers: Arc::new(providers),
        }
    }
}

fn external_totp_code(
    credential: &ExternalCredential,
    account: &str,
) -> std::result::Result<Option<String>, String> {
    match credential.totp_secret.as_deref() {
        Some(secret) => Ok(Some(
            build_totp(secret, account)?
                .generate_current()
                .map_err(|error| format!("could not generate TOTP code: {error}"))?,
        )),
        None => Ok(None),
    }
}

impl CredentialResolver for CredentialService {
    fn resolve<'a>(
        &'a self,
        credential_id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<ResolvedCredential, String>> {
        Box::pin(async move {
            let credential = repo::get_credential(&self.db, &self.tenant_id, credential_id)
                .await
                .map_err(|error| error.to_string())?;
            if credential.provider != "local" {
                let external_ref = credential
                    .external_ref
                    .as_deref()
                    .ok_or_else(|| "external credential has no external reference".to_string())?;
                let external = self
                    .providers
                    .resolve(&credential.provider, external_ref)
                    .await?;
                let account = external
                    .username
                    .as_deref()
                    .unwrap_or(&credential.name)
                    .replace(':', "_");
                let totp_code = external_totp_code(&external, &account)?;
                return Ok(ResolvedCredential {
                    username: external.username.or(credential.username),
                    secret: external.password,
                    totp_code,
                });
            }
            let secret = self
                .encryptor
                .decrypt_str(&credential.secret_encrypted)
                .map_err(|error| format!("could not decrypt credential: {error}"))?;
            let totp_code = match credential.totp_secret_encrypted.as_deref() {
                Some(encrypted) => {
                    let secret = self
                        .encryptor
                        .decrypt_str(encrypted)
                        .map_err(|error| format!("could not decrypt TOTP secret: {error}"))?;
                    let account = credential
                        .username
                        .as_deref()
                        .unwrap_or(&credential.name)
                        .replace(':', "_");
                    let totp = build_totp(&secret, &account)?;
                    Some(
                        totp.generate_current()
                            .map_err(|error| format!("could not generate TOTP code: {error}"))?,
                    )
                }
                None => None,
            };
            Ok(ResolvedCredential {
                username: credential.username,
                secret,
                totp_code,
            })
        })
    }
}

fn build_totp(secret: &str, account: &str) -> std::result::Result<TOTP, String> {
    let secret = Secret::Encoded(secret.to_string());
    let bytes = secret
        .to_bytes()
        .map_err(|error| format!("invalid TOTP secret: {error}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("DoOnce".to_string()),
        account.to_string(),
    )
    .map_err(|error| format!("could not build TOTP: {error}"))
}

// ---------------------------------------------------------------------------
// SMTP email
// ---------------------------------------------------------------------------

pub struct SmtpEmailService {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmailService {
    pub fn from_env() -> std::result::Result<Option<Self>, String> {
        let Some(url) = std::env::var("DOONCE_SMTP_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let from = std::env::var("DOONCE_EMAIL_FROM")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "doonce@localhost".to_string());
        let from = from
            .parse::<Mailbox>()
            .map_err(|error| format!("invalid DOONCE_EMAIL_FROM: {error}"))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url.trim())
            .map_err(|error| format!("invalid DOONCE_SMTP_URL: {error}"))?
            .build();
        Ok(Some(SmtpEmailService { transport, from }))
    }
}

impl EmailSender for SmtpEmailService {
    fn send<'a>(
        &'a self,
        message: EmailMessage,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            let mut builder = EmailBuilder::builder()
                .from(self.from.clone())
                .subject(message.subject);
            for recipient in &message.to {
                let mailbox = recipient
                    .parse::<Mailbox>()
                    .map_err(|error| format!("invalid recipient \"{recipient}\": {error}"))?;
                builder = builder.to(mailbox);
            }
            let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(message.body));
            for attachment_path in &message.attachments {
                let filename = attachment_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "attachment".to_string());
                let content_type = mime_guess::from_path(attachment_path)
                    .first_or_octet_stream()
                    .to_string()
                    .parse::<ContentType>()
                    .unwrap_or_else(|_| {
                        ContentType::parse("application/octet-stream").expect("valid content type")
                    });
                let bytes = std::fs::read(attachment_path)
                    .map_err(|error| format!("could not read attachment: {error}"))?;
                multipart =
                    multipart.singlepart(Attachment::new(filename).body(bytes, content_type));
            }
            let email = builder
                .multipart(multipart)
                .map_err(|error| format!("could not build email: {error}"))?;
            self.transport
                .send(email)
                .await
                .map_err(|error| format!("SMTP send failed: {error}"))?;
            Ok(json!({ "sent": true, "to": message.to }))
        })
    }
}

// ---------------------------------------------------------------------------
// Local artifact storage
// ---------------------------------------------------------------------------

pub struct LocalArtifactStore {
    root: PathBuf,
}

impl LocalArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        LocalArtifactStore { root }
    }
}

impl ArtifactStore for LocalArtifactStore {
    fn put<'a>(
        &'a self,
        artifact: &'a str,
        destination: &'a str,
    ) -> BoxFuture<'a, std::result::Result<Value, String>> {
        Box::pin(async move {
            let source = PathBuf::from(artifact);
            if !source.is_file() {
                return Err(format!("artifact not found: {}", source.display()));
            }
            std::fs::create_dir_all(&self.root)
                .map_err(|error| format!("could not create upload directory: {error}"))?;
            let root = std::fs::canonicalize(&self.root)
                .map_err(|error| format!("upload directory: {error}"))?;
            let relative = safe_relative(destination)?;
            let target = root.join(&relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create destination directory: {error}"))?;
                let parent = std::fs::canonicalize(parent)
                    .map_err(|error| format!("destination directory: {error}"))?;
                if !parent.starts_with(&root) {
                    return Err("destination escaped the upload directory".into());
                }
            }
            let bytes = std::fs::copy(&source, &target)
                .map_err(|error| format!("could not copy artifact: {error}"))?;
            Ok(json!({ "path": target, "bytes": bytes }))
        })
    }
}

fn safe_relative(destination: &str) -> std::result::Result<PathBuf, String> {
    let path = Path::new(destination);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("destination must be a relative path inside the upload directory".into());
    }
    if destination.trim().is_empty() {
        return Err("destination must not be empty".into());
    }
    Ok(path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Nested workflow resolver
// ---------------------------------------------------------------------------

pub struct DbWorkflowResolver {
    db: Database,
    tenant_id: String,
}

impl DbWorkflowResolver {
    pub fn new(db: Database, tenant_id: String) -> Self {
        DbWorkflowResolver { db, tenant_id }
    }
}

impl WorkflowResolver for DbWorkflowResolver {
    fn load<'a>(
        &'a self,
        workflow_id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<doonce_core::WorkflowDefinition, String>> {
        Box::pin(async move {
            repo::current_workflow_definition(&self.db, &self.tenant_id, workflow_id)
                .await
                .map(|(_, definition)| definition)
                .map_err(|error| error.to_string())
        })
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

pub fn services_from_env(
    db: Database,
    encryptor: Encryptor,
    tenant_id: String,
    task_options: TaskOptions,
    upload_root: PathBuf,
) -> Services {
    services_from_env_with_llm(db, encryptor, tenant_id, task_options, upload_root, None)
}

pub fn services_from_env_with_llm(
    db: Database,
    encryptor: Encryptor,
    tenant_id: String,
    task_options: TaskOptions,
    upload_root: PathBuf,
    llm_provider_config: Option<doonce_llm::ProviderConfig>,
) -> Services {
    let ai = if let Some(cfg) = llm_provider_config {
        doonce_llm::Provider::new(cfg)
            .ok()
            .map(|p| Arc::new(AiService::new(p, task_options)) as Arc<dyn AiExecutor>)
    } else {
        AiService::from_env(task_options).map(|service| Arc::new(service) as Arc<dyn AiExecutor>)
    };
    let credentials = Arc::new(CredentialService::new(
        db.clone(),
        encryptor,
        tenant_id.clone(),
    ));
    let artifacts = Arc::new(LocalArtifactStore::new(upload_root));
    let workflows = Arc::new(DbWorkflowResolver::new(db, tenant_id));
    let email = match SmtpEmailService::from_env() {
        Ok(Some(service)) => Some(Arc::new(service) as Arc<dyn EmailSender>),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "SMTP email is disabled");
            None
        }
    };
    Services {
        ai,
        credentials: Some(credentials),
        email,
        artifacts: Some(artifacts),
        workflows: Some(workflows),
        max_depth: 3,
    }
}

pub fn local_services(upload_root: PathBuf) -> Services {
    let email = match SmtpEmailService::from_env() {
        Ok(Some(service)) => Some(Arc::new(service) as Arc<dyn EmailSender>),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "SMTP email is disabled");
            None
        }
    };
    Services {
        ai: None,
        credentials: None,
        email,
        artifacts: Some(Arc::new(LocalArtifactStore::new(upload_root))),
        workflows: None,
        max_depth: 3,
    }
}
