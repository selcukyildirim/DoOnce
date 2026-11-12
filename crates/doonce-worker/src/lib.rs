//! Background worker: claims queued workflow runs and tasks, executes them in
//! headless Chrome, records step history, waits for approvals and delivers
//! webhooks. The scheduler turns due schedules into queued runs.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::{SecondsFormat, Utc};
use doonce_agent::{
    ApprovalRequest as AgentApproval, TaskEvent, TaskObserver, TaskOptions, TaskSpec,
    TaskStatus as AgentTaskStatus,
};
use doonce_browser::{BrowserSession, LaunchOptions, find_chrome};
use doonce_core::{ArtifactKind, Schedule, Task, TaskStatus, WorkflowRun, now_rfc3339};
use doonce_engine::{ApprovalRequest, ExecEvent, Observer, RunOptions, RunStatus, Services};
use doonce_llm::{Provider, ProviderConfig, ProviderKind};
use doonce_security::MasterKey;
use doonce_services::services_from_env_with_llm;
use doonce_store::{Database, repo};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub deployment_mode: doonce_core::DeploymentMode,
    pub region: String,
    pub worker_id: String,
    pub concurrency: usize,
    pub poll_interval: Duration,
    pub lease_secs: i64,
    pub approval_timeout: Duration,
    pub auto_approve: bool,
    pub headless: bool,
    pub vision: bool,
    pub chrome_profile: Option<PathBuf>,
    pub cdp_url: Option<String>,
    pub data_dir: PathBuf,
    pub artifact_dir: PathBuf,
    pub webhook_allow_private: bool,
}

impl WorkerConfig {
    pub fn from_env() -> Self {
        let data_dir = data_dir();
        WorkerConfig {
            deployment_mode: doonce_core::DeploymentMode::from_env(),
            region: doonce_core::region_from_env(),
            worker_id: std::env::var("DOONCE_WORKER_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("worker-{}", std::process::id())),
            concurrency: env_u64("DOONCE_WORKER_CONCURRENCY")
                .unwrap_or(2)
                .clamp(1, 32) as usize,
            poll_interval: Duration::from_secs(
                env_u64("DOONCE_WORKER_POLL_SECS").unwrap_or(2).max(1),
            ),
            lease_secs: env_u64("DOONCE_WORKER_LEASE_SECS").unwrap_or(300).max(30) as i64,
            approval_timeout: Duration::from_secs(
                env_u64("DOONCE_WORKER_APPROVAL_TIMEOUT_SECS")
                    .unwrap_or(600)
                    .max(30),
            ),
            auto_approve: env_bool("DOONCE_WORKER_AUTO_APPROVE", false),
            headless: env_bool("DOONCE_HEADLESS", true),
            vision: env_bool("DOONCE_LLM_VISION", true),
            chrome_profile: std::env::var("DOONCE_CHROME_PROFILE")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            cdp_url: std::env::var("DOONCE_CDP_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            artifact_dir: std::env::var("DOONCE_ARTIFACT_DIR")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("artifacts")),
            data_dir,
            webhook_allow_private: env_bool("DOONCE_WEBHOOK_ALLOW_PRIVATE", false),
        }
    }
}

fn is_loopback_endpoint(value: Option<&str>) -> bool {
    let Some(value) = value else { return false };
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://127.0.0.1")
        || value.starts_with("http://localhost")
        || value.starts_with("http://[::1]")
        || value.starts_with("https://127.0.0.1")
        || value.starts_with("https://localhost")
        || value.starts_with("https://[::1]")
}

pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("DOONCE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DoOnce")
}

pub fn database_url() -> String {
    if let Some(url) = std::env::var("DOONCE_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return url;
    }
    format!(
        "sqlite://{}?mode=rwc",
        data_dir().join("doonce.db").display()
    )
}

struct Worker {
    config: WorkerConfig,
    db: Database,
    encryptor: doonce_security::Encryptor,
    http: reqwest::Client,
}

pub async fn run(mut config: WorkerConfig) -> Result<()> {
    if config.chrome_profile.is_some() || config.cdp_url.is_some() {
        config.concurrency = 1;
    }
    let db = Database::connect(&database_url())
        .await
        .context("could not open the worker database")?;
    db.migrate()
        .await
        .context("worker database migration failed")?;
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("could not create {}", config.data_dir.display()))?;
    let master = MasterKey::load_or_create(&config.data_dir.join("master.key"))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("could not build the HTTP client")?;
    let worker = Arc::new(Worker {
        config: config.clone(),
        db,
        encryptor: master.encryptor(),
        http,
    });

    tracing::info!(
        worker = %config.worker_id,
        concurrency = config.concurrency,
        "DoOnce worker started"
    );
    let mut handles = Vec::new();
    {
        let worker = worker.clone();
        handles.push(tokio::spawn(async move {
            worker.scheduler_loop().await;
        }));
    }
    for index in 0..config.concurrency {
        let worker = worker.clone();
        handles.push(tokio::spawn(async move {
            worker.worker_loop(index).await;
        }));
    }

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("stopping worker");
    for handle in handles {
        handle.abort();
    }
    Ok(())
}

impl Worker {
    async fn worker_loop(&self, index: usize) {
        tracing::info!(worker = %self.config.worker_id, slot = index, "worker slot ready");
        loop {
            let stale_before = stale_before(self.config.lease_secs);
            let _ = repo::release_stale_workflow_runs(&self.db, &stale_before).await;
            let _ = repo::release_stale_tasks(&self.db, &stale_before).await;

            match repo::claim_next_workflow_run(&self.db, &self.config.worker_id, &stale_before)
                .await
            {
                Ok(Some(run)) => {
                    let id = run.id.clone();
                    if let Err(error) = self.run_workflow(run).await {
                        tracing::error!(run = %id, %error, "workflow run failed");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "could not claim a workflow run"),
            }

            match repo::claim_next_task(&self.db, &self.config.worker_id, &stale_before).await {
                Ok(Some(task)) => {
                    let id = task.id.clone();
                    if let Err(error) = self.run_task(task).await {
                        tracing::error!(task = %id, %error, "task run failed");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "could not claim a task"),
            }

            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    async fn scheduler_loop(&self) {
        loop {
            let now = now_rfc3339();
            match repo::due_schedules(&self.db, &now).await {
                Ok(schedules) => {
                    for schedule in schedules {
                        if let Err(error) = self.queue_schedule(&schedule).await {
                            tracing::error!(schedule = %schedule.id, %error, "could not queue schedule");
                        }
                    }
                }
                Err(error) => tracing::error!(%error, "could not read due schedules"),
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    async fn queue_schedule(&self, schedule: &Schedule) -> Result<()> {
        let workflow =
            repo::get_workflow(&self.db, &schedule.tenant_id, &schedule.workflow_id).await?;
        repo::create_workflow_run(
            &self.db,
            &schedule.tenant_id,
            &workflow.id,
            workflow.current_version,
            &json!({}),
            "schedule",
            None,
            None,
        )
        .await?;
        let next = doonce_store::cron::next_run(&schedule.cron, &schedule.timezone)?;
        repo::mark_schedule_run(&self.db, &schedule.id, &next, &now_rfc3339(), "queued").await?;
        tracing::info!(schedule = %schedule.id, "queued scheduled workflow run");
        Ok(())
    }

    async fn run_workflow(&self, run: WorkflowRun) -> Result<()> {
        let tenant = run.tenant_id.clone();
        let run_id = run.id.clone();
        let (workflow, definition) =
            match repo::current_workflow_definition(&self.db, &tenant, &run.workflow_id).await {
                Ok(value) => value,
                Err(error) => {
                    self.fail_run(&run, &format!("could not load workflow: {error}"))
                        .await;
                    return Ok(());
                }
            };
        let params = params_from_inputs(&run.inputs);
        let mut browser = match self.open_browser(&run_id).await {
            Ok(browser) => browser,
            Err(error) => {
                self.fail_run(&run, &format!("could not start browser: {error:#}"))
                    .await;
                return Ok(());
            }
        };
        let services = self.services_for(&tenant).await;
        let cancel = CancellationToken::new();
        let watcher = self.spawn_run_watcher(&tenant, &run_id, cancel.clone());
        let observer = WorkflowObserver::new(
            self.db.clone(),
            tenant.clone(),
            run_id.clone(),
            self.config.auto_approve,
            self.config.approval_timeout,
            cancel.clone(),
        );
        let options = RunOptions {
            cancel: cancel.clone(),
            ..RunOptions::default()
        };
        let outcome = doonce_engine::run_with_services(
            &definition,
            &params,
            &mut browser,
            &observer,
            &options,
            &services,
        )
        .await;
        watcher.abort();
        if let Err(error) = self
            .capture_screenshot(&tenant, Some(&run_id), None, &mut browser)
            .await
        {
            tracing::warn!(run = %run_id, %error, "could not capture workflow screenshot");
        }
        browser.close().await;

        let current = repo::get_workflow_run(&self.db, &tenant, &run_id)
            .await
            .ok();
        let final_status = if current
            .as_ref()
            .is_some_and(|run| run.status == "cancelled")
        {
            RunStatus::Cancelled
        } else {
            outcome.status
        };
        let error = outcome
            .failure
            .as_ref()
            .map(|failure| failure.message.clone());
        let outputs = outcome.outputs.clone();
        let _ = repo::set_workflow_run_status(
            &self.db,
            &tenant,
            &run_id,
            final_status.as_str(),
            Some(&json!(outputs)),
            error.as_deref(),
            None,
            Some(&now_rfc3339()),
        )
        .await;
        let _ = repo::clear_workflow_run_lease(&self.db, &run_id).await;
        let event = match final_status {
            RunStatus::Completed => "run.completed",
            RunStatus::Cancelled => "run.cancelled",
            RunStatus::Failed => "run.failed",
        };
        self.dispatch_event(
            &tenant,
            event,
            json!({
                "runId": run_id,
                "workflowId": workflow.id,
                "status": final_status.as_str(),
                "outputs": outputs,
                "error": error,
            }),
        )
        .await;
        Ok(())
    }

    async fn run_task(&self, task: Task) -> Result<()> {
        let tenant = task.tenant_id.clone();
        let task_id = task.id.clone();
        let Some(provider_config) = self.llm_provider_config(&tenant).await else {
            self.fail_task(&task, "AI provider is not configured").await;
            return Ok(());
        };
        if self.config.deployment_mode.is_air_gapped()
            && !is_loopback_endpoint(provider_config.base_url.as_deref())
        {
            self.fail_task(
                &task,
                "air-gapped mode requires a local LLM endpoint on loopback",
            )
            .await;
            return Ok(());
        }
        let Some(provider) = Provider::new(provider_config).ok() else {
            self.fail_task(&task, "AI provider is not configured").await;
            return Ok(());
        };
        let spec = TaskSpec {
            prompt: task.prompt.clone(),
            url: task.url.clone(),
            data_schema: task.data_schema.clone(),
            error_codes: task.error_codes.clone(),
            max_steps: task.max_steps,
            model: task.model.clone(),
        };
        let mut browser = match self.open_browser(&task_id).await {
            Ok(browser) => browser,
            Err(error) => {
                self.fail_task(&task, &format!("could not start browser: {error:#}"))
                    .await;
                return Ok(());
            }
        };
        let cancel = CancellationToken::new();
        let watcher = self.spawn_task_watcher(&tenant, &task_id, cancel.clone());
        let observer = TaskWorkerObserver::new(
            self.db.clone(),
            tenant.clone(),
            task_id.clone(),
            self.config.auto_approve,
            self.config.approval_timeout,
            cancel.clone(),
        );
        let options = TaskOptions {
            element_timeout: Duration::from_secs(15),
            navigation_timeout: Duration::from_secs(30),
            max_steps: task.max_steps,
            vision: self.config.vision,
            cancel: cancel.clone(),
            ..TaskOptions::default()
        };
        let outcome =
            doonce_agent::run_task(&provider, &spec, &mut browser, &observer, &options).await;
        watcher.abort();
        if let Err(error) = self
            .capture_screenshot(&tenant, None, Some(&task_id), &mut browser)
            .await
        {
            tracing::warn!(task = %task_id, %error, "could not capture task screenshot");
        }
        browser.close().await;

        let current = repo::get_task(&self.db, &tenant, &task_id).await.ok();
        let final_status = if current
            .as_ref()
            .is_some_and(|task| task.status == "cancelled")
        {
            TaskStatus::Cancelled
        } else {
            match outcome.status {
                AgentTaskStatus::Completed => TaskStatus::Completed,
                AgentTaskStatus::Cancelled => TaskStatus::Cancelled,
                AgentTaskStatus::Failed => TaskStatus::Failed,
            }
        };
        let failure = outcome.failure.map(|failure| failure.message);
        let _ = repo::set_task_status(
            &self.db,
            &tenant,
            &task_id,
            final_status,
            outcome.output.as_ref(),
            failure.as_deref(),
            None,
            Some(&now_rfc3339()),
        )
        .await;
        let _ = repo::clear_task_lease(&self.db, &task_id).await;
        let event = match final_status {
            TaskStatus::Completed => "task.completed",
            TaskStatus::Cancelled => "task.cancelled",
            TaskStatus::Failed => "task.failed",
            _ => "task.failed",
        };
        self.dispatch_event(
            &tenant,
            event,
            json!({
                "taskId": task_id,
                "status": final_status.as_str(),
                "output": outcome.output,
                "error": failure,
            }),
        )
        .await;
        Ok(())
    }

    async fn fail_run(&self, run: &WorkflowRun, message: &str) {
        let _ = repo::set_workflow_run_status(
            &self.db,
            &run.tenant_id,
            &run.id,
            "failed",
            None,
            Some(message),
            None,
            Some(&now_rfc3339()),
        )
        .await;
        let _ = repo::clear_workflow_run_lease(&self.db, &run.id).await;
        self.dispatch_event(
            &run.tenant_id,
            "run.failed",
            json!({ "runId": run.id, "workflowId": run.workflow_id, "error": message }),
        )
        .await;
    }

    async fn fail_task(&self, task: &Task, message: &str) {
        let _ = repo::set_task_status(
            &self.db,
            &task.tenant_id,
            &task.id,
            TaskStatus::Failed,
            None,
            Some(message),
            None,
            Some(&now_rfc3339()),
        )
        .await;
        let _ = repo::clear_task_lease(&self.db, &task.id).await;
        self.dispatch_event(
            &task.tenant_id,
            "task.failed",
            json!({ "taskId": task.id, "error": message }),
        )
        .await;
    }

    async fn services_for(&self, tenant_id: &str) -> Services {
        let task_options = TaskOptions {
            element_timeout: Duration::from_secs(15),
            navigation_timeout: Duration::from_secs(30),
            max_steps: 30,
            vision: self.config.vision,
            ..TaskOptions::default()
        };
        let llm_config = self.llm_provider_config(tenant_id).await;
        services_from_env_with_llm(
            self.db.clone(),
            self.encryptor.clone(),
            tenant_id.to_string(),
            task_options,
            self.config.data_dir.join("worker/uploads"),
            llm_config,
        )
    }

    async fn llm_provider_config(&self, tenant_id: &str) -> Option<ProviderConfig> {
        // Try DB-stored default config first.
        if let Ok(Some(cfg)) = repo::default_llm_config(&self.db, tenant_id).await {
            let kind = ProviderKind::parse(&cfg.provider)?;
            let api_key = cfg
                .api_key_encrypted
                .as_deref()
                .and_then(|enc| self.encryptor.decrypt_str(enc).ok());
            let caps = &cfg.capabilities;
            let vision = caps
                .get("vision")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| kind.supports_vision());
            let json_mode = caps
                .get("jsonMode")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            return Some(ProviderConfig {
                kind,
                model: cfg.model,
                base_url: cfg.base_url,
                api_key,
                timeout: Duration::from_secs(60),
                max_output_tokens: 4096,
                temperature: 0.2,
                vision,
                json_mode,
            });
        }
        // Fall back to environment.
        ProviderConfig::from_env().ok()
    }

    async fn capture_screenshot(
        &self,
        tenant_id: &str,
        run_id: Option<&str>,
        task_id: Option<&str>,
        browser: &mut BrowserSession,
    ) -> Result<()> {
        let png = browser
            .screenshot()
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let root = &self.config.artifact_dir;
        let job = run_id.or(task_id).unwrap_or("misc");
        let dir = root.join(tenant_id).join(job);
        std::fs::create_dir_all(&dir)?;
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ");
        let filename = format!("final-{stamp}.png");
        let path = dir.join(&filename);
        std::fs::write(&path, &png)?;
        let storage_key = path
            .strip_prefix(root)
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|_| filename.clone());
        let sha256 = hex::encode(Sha256::digest(&png));
        repo::create_artifact(
            &self.db,
            tenant_id,
            run_id,
            task_id,
            None,
            ArtifactKind::Screenshot,
            &filename,
            "image/png",
            png.len() as i64,
            &storage_key,
            &sha256,
        )
        .await?;
        Ok(())
    }

    async fn open_browser(&self, job_id: &str) -> Result<BrowserSession> {
        let downloads = self.config.data_dir.join("worker/downloads").join(job_id);
        if let Some(cdp) = &self.config.cdp_url {
            return BrowserSession::connect(cdp, &downloads)
                .await
                .map_err(|error| anyhow!("{error}"));
        }
        let chrome = find_chrome().ok_or_else(|| anyhow!("Google Chrome was not found"))?;
        let profile = self
            .config
            .chrome_profile
            .clone()
            .unwrap_or_else(|| self.config.data_dir.join("worker/chrome").join(job_id));
        BrowserSession::launch(
            &LaunchOptions {
                executable: chrome,
                user_data_dir: profile,
                headless: self.config.headless,
            },
            &downloads,
        )
        .await
        .map_err(|error| anyhow!("{error}"))
    }

    fn spawn_run_watcher(
        &self,
        tenant: &str,
        run_id: &str,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let db = self.db.clone();
        let tenant = tenant.to_string();
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let _ = repo::heartbeat_workflow_run(&db, &run_id).await;
                if let Ok(run) = repo::get_workflow_run(&db, &tenant, &run_id).await
                    && run.status == "cancelled"
                {
                    cancel.cancel();
                    break;
                }
            }
        })
    }

    fn spawn_task_watcher(
        &self,
        tenant: &str,
        task_id: &str,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let db = self.db.clone();
        let tenant = tenant.to_string();
        let task_id = task_id.to_string();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let _ = repo::heartbeat_task(&db, &task_id).await;
                if let Ok(task) = repo::get_task(&db, &tenant, &task_id).await
                    && task.status == "cancelled"
                {
                    cancel.cancel();
                    break;
                }
            }
        })
    }

    async fn dispatch_event(&self, tenant_id: &str, event: &str, payload: Value) {
        let webhooks = match repo::enabled_webhooks_for_event(&self.db, tenant_id, event).await {
            Ok(webhooks) => webhooks,
            Err(error) => {
                tracing::error!(%error, "could not read webhooks");
                return;
            }
        };
        for webhook in webhooks {
            if !self.config.webhook_allow_private
                && !doonce_core::validation::is_safe_outbound_url(&webhook.url)
            {
                tracing::warn!(webhook = %webhook.id, "refusing private webhook URL");
                continue;
            }
            let secret = match self.encryptor.decrypt_str(&webhook.secret_encrypted) {
                Ok(secret) => secret,
                Err(error) => {
                    tracing::error!(webhook = %webhook.id, %error, "could not decrypt webhook secret");
                    continue;
                }
            };
            let body = json!({
                "event": event,
                "tenantId": tenant_id,
                "timestamp": now_rfc3339(),
                "payload": payload,
            });
            let body_text = serde_json::to_string(&body).unwrap_or_default();
            let signature = sign(&secret, &body_text);
            let mut attempts = 0i64;
            let mut response_code = None;
            let mut last_error = None;
            let mut status = "failed";
            for attempt in 1..=3 {
                attempts = attempt;
                match self
                    .http
                    .post(&webhook.url)
                    .header("content-type", "application/json")
                    .header("x-doonce-event", event)
                    .header("x-doonce-signature", format!("sha256={signature}"))
                    .body(body_text.clone())
                    .send()
                    .await
                {
                    Ok(response) => {
                        response_code = Some(response.status().as_u16() as i64);
                        if response.status().is_success() {
                            status = "delivered";
                            break;
                        }
                        last_error = Some(format!("HTTP {}", response.status().as_u16()));
                    }
                    Err(error) => last_error = Some(error.to_string()),
                }
                tokio::time::sleep(Duration::from_millis(500 * (1 << (attempt - 1)))).await;
            }
            let _ = repo::record_webhook_delivery(
                &self.db,
                tenant_id,
                &webhook.id,
                event,
                &body,
                status,
                attempts,
                response_code,
                last_error.as_deref(),
            )
            .await;
        }
    }
}

fn sign(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn params_from_inputs(inputs: &Value) -> HashMap<String, String> {
    inputs
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn stale_before(lease_secs: i64) -> String {
    (Utc::now() - chrono::Duration::seconds(lease_secs))
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|value| value.parse().ok())
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

struct WorkflowObserver {
    db: Database,
    tenant_id: String,
    run_id: String,
    auto_approve: bool,
    approval_timeout: Duration,
    cancel: CancellationToken,
    tx: mpsc::UnboundedSender<ExecEvent>,
}

impl WorkflowObserver {
    fn new(
        db: Database,
        tenant_id: String,
        run_id: String,
        auto_approve: bool,
        approval_timeout: Duration,
        cancel: CancellationToken,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let recorder_db = db.clone();
        let recorder_tenant = tenant_id.clone();
        let recorder_run = run_id.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let ExecEvent::Step {
                    index,
                    block_id,
                    status,
                    error,
                    ..
                } = event
                {
                    let _ = repo::record_block_run(
                        &recorder_db,
                        &recorder_tenant,
                        &recorder_run,
                        &block_id,
                        index as i64,
                        status.as_str(),
                        None,
                        None,
                        error.as_deref(),
                        1,
                    )
                    .await;
                }
            }
        });
        WorkflowObserver {
            db,
            tenant_id,
            run_id,
            auto_approve,
            approval_timeout,
            cancel,
            tx,
        }
    }
}

impl Observer for WorkflowObserver {
    fn on_event(&self, event: &ExecEvent) {
        let _ = self.tx.send(event.clone());
    }

    async fn approve(&self, request: ApprovalRequest) -> bool {
        if self.auto_approve {
            return true;
        }
        let approval = match repo::create_approval(
            &self.db,
            &self.tenant_id,
            Some(&self.run_id),
            None,
            &request.block_id,
            &request.summary,
            &request.reason,
        )
        .await
        {
            Ok(approval) => approval,
            Err(error) => {
                tracing::error!(%error, "could not create approval");
                return false;
            }
        };
        let _ = repo::set_workflow_run_status(
            &self.db,
            &self.tenant_id,
            &self.run_id,
            "awaiting_approval",
            None,
            None,
            None,
            None,
        )
        .await;
        let deadline = Instant::now() + self.approval_timeout;
        loop {
            if self.cancel.is_cancelled() {
                return false;
            }
            if let Ok(current) = repo::get_approval(&self.db, &self.tenant_id, &approval.id).await {
                match current.status.as_str() {
                    "approved" => {
                        let _ = repo::set_workflow_run_status(
                            &self.db,
                            &self.tenant_id,
                            &self.run_id,
                            "running",
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                        return true;
                    }
                    "denied" => {
                        let _ = repo::set_workflow_run_status(
                            &self.db,
                            &self.tenant_id,
                            &self.run_id,
                            "running",
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                        return false;
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                let _ = repo::resolve_approval(
                    &self.db,
                    &self.tenant_id,
                    &approval.id,
                    "denied",
                    Some("timeout"),
                )
                .await;
                return false;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

struct TaskWorkerObserver {
    db: Database,
    tenant_id: String,
    task_id: String,
    auto_approve: bool,
    approval_timeout: Duration,
    cancel: CancellationToken,
    tx: mpsc::UnboundedSender<TaskEvent>,
}

impl TaskWorkerObserver {
    fn new(
        db: Database,
        tenant_id: String,
        task_id: String,
        auto_approve: bool,
        approval_timeout: Duration,
        cancel: CancellationToken,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let recorder_db = db.clone();
        let recorder_tenant = tenant_id.clone();
        let recorder_task = task_id.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let TaskEvent::Step {
                    action,
                    reasoning,
                    status,
                    ..
                } = event
                {
                    let _ = repo::record_task_action(
                        &recorder_db,
                        &recorder_tenant,
                        &recorder_task,
                        &action,
                        Some(&reasoning),
                        None,
                        &status,
                    )
                    .await;
                }
            }
        });
        TaskWorkerObserver {
            db,
            tenant_id,
            task_id,
            auto_approve,
            approval_timeout,
            cancel,
            tx,
        }
    }
}

impl TaskObserver for TaskWorkerObserver {
    fn on_event(&self, event: &TaskEvent) {
        let _ = self.tx.send(event.clone());
    }

    async fn approve(&self, request: AgentApproval) -> bool {
        if self.auto_approve {
            return true;
        }
        let approval = match repo::create_approval(
            &self.db,
            &self.tenant_id,
            None,
            Some(&self.task_id),
            &request.summary,
            &request.summary,
            &request.reason,
        )
        .await
        {
            Ok(approval) => approval,
            Err(error) => {
                tracing::error!(%error, "could not create task approval");
                return false;
            }
        };
        let _ = repo::set_task_status(
            &self.db,
            &self.tenant_id,
            &self.task_id,
            TaskStatus::AwaitingApproval,
            None,
            None,
            None,
            None,
        )
        .await;
        let deadline = Instant::now() + self.approval_timeout;
        loop {
            if self.cancel.is_cancelled() {
                return false;
            }
            if let Ok(current) = repo::get_approval(&self.db, &self.tenant_id, &approval.id).await {
                match current.status.as_str() {
                    "approved" => {
                        let _ = repo::set_task_status(
                            &self.db,
                            &self.tenant_id,
                            &self.task_id,
                            TaskStatus::Running,
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                        return true;
                    }
                    "denied" => {
                        let _ = repo::set_task_status(
                            &self.db,
                            &self.tenant_id,
                            &self.task_id,
                            TaskStatus::Running,
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                        return false;
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                let _ = repo::resolve_approval(
                    &self.db,
                    &self.tenant_id,
                    &approval.id,
                    "denied",
                    Some("timeout"),
                )
                .await;
                return false;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_stringified() {
        let params = params_from_inputs(&json!({"name":"Ada","count":3}));
        assert_eq!(params.get("name").map(String::as_str), Some("Ada"));
        assert_eq!(params.get("count").map(String::as_str), Some("3"));
    }

    #[test]
    fn signatures_are_stable() {
        assert_eq!(sign("secret", "body"), sign("secret", "body"));
        assert_ne!(sign("secret", "body"), sign("other", "body"));
    }

    #[tokio::test]
    async fn scheduler_queues_due_schedules() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().await.unwrap();
        let tenant = repo::create_tenant(&db, "Test", "test").await.unwrap();
        let definition = doonce_core::validation::parse_workflow(
            r#"{"schemaVersion":"2.0","name":"Scheduled","blocks":[{"id":"done","type":"terminate","status":"completed"}]}"#,
        )
        .unwrap();
        let workflow = repo::create_workflow(&db, &tenant.id, &definition, None, "test")
            .await
            .unwrap();
        let schedule = repo::create_schedule(&db, &tenant.id, &workflow.id, "* * * * *", "UTC")
            .await
            .unwrap();
        let config = WorkerConfig {
            deployment_mode: doonce_core::DeploymentMode::OnPrem,
            region: "test".into(),
            worker_id: "test-worker".into(),
            concurrency: 1,
            poll_interval: Duration::from_secs(1),
            lease_secs: 60,
            approval_timeout: Duration::from_secs(60),
            auto_approve: false,
            headless: true,
            vision: true,
            chrome_profile: None,
            cdp_url: None,
            data_dir: dir.path().to_path_buf(),
            artifact_dir: dir.path().join("artifacts"),
            webhook_allow_private: false,
        };
        let worker = Worker {
            config,
            db: db.clone(),
            encryptor: MasterKey::generate().encryptor(),
            http: reqwest::Client::new(),
        };
        worker.queue_schedule(&schedule).await.unwrap();
        let runs = repo::list_workflow_runs(&db, &tenant.id, Some(&workflow.id), 10)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].trigger, "schedule");
        let updated = repo::get_schedule(&db, &tenant.id, &schedule.id)
            .await
            .unwrap();
        assert_eq!(updated.last_status.as_deref(), Some("queued"));
    }

    #[tokio::test]
    async fn webhook_dispatch_signs_and_records_delivery() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().await.unwrap();
        let tenant = repo::create_tenant(&db, "Test", "test").await.unwrap();
        let encryptor = MasterKey::generate().encryptor();
        let secret = "webhook-secret";
        let encrypted = encryptor.encrypt_str(secret).unwrap();
        let webhook = repo::create_webhook(
            &db,
            &tenant.id,
            "http://127.0.0.1:9/hook",
            &encrypted,
            &["run.completed".to_string()],
        )
        .await
        .unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        // Update the webhook URL to the ephemeral port.
        let db2 = db.clone();
        let tenant2 = tenant.id.clone();
        let webhook_id = webhook.id.clone();
        tokio::spawn(async move {
            // The repository has no update-webhook-URL helper; insert a new one
            // and delete the placeholder before dispatching.
            repo::delete_webhook(&db2, &tenant2, &webhook_id).await.ok();
            repo::create_webhook(
                &db2,
                &tenant2,
                &format!("http://{address}/hook"),
                &encrypted,
                &["run.completed".to_string()],
            )
            .await
            .ok();
        })
        .await
        .unwrap();
        let (capture_tx, capture_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = vec![0u8; 8192];
            let n = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..n]).to_string();
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK
Content-Length: 2
Connection: close

ok",
                )
                .await;
            let _ = stream.shutdown().await;
            let _ = capture_tx.send(request);
        });

        let config = WorkerConfig {
            deployment_mode: doonce_core::DeploymentMode::OnPrem,
            region: "test".into(),
            worker_id: "test-worker".into(),
            concurrency: 1,
            poll_interval: Duration::from_secs(1),
            lease_secs: 60,
            approval_timeout: Duration::from_secs(60),
            auto_approve: false,
            headless: true,
            vision: true,
            chrome_profile: None,
            cdp_url: None,
            data_dir: dir.path().to_path_buf(),
            artifact_dir: dir.path().join("artifacts"),
            webhook_allow_private: true,
        };
        let worker = Worker {
            config,
            db: db.clone(),
            encryptor,
            http: reqwest::Client::new(),
        };
        worker
            .dispatch_event(&tenant.id, "run.completed", json!({"runId": "run-1"}))
            .await;
        let request = capture_rx.await.unwrap();
        assert!(request.contains("x-doonce-event: run.completed"));
        assert!(request.contains("x-doonce-signature: sha256="));
        assert!(request.contains("\"runId\":\"run-1\""));
        let webhooks = repo::list_webhooks(&db, &tenant.id).await.unwrap();
        let deliveries = repo::list_webhook_deliveries(&db, &tenant.id, &webhooks[0].id, 10)
            .await
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].status, "delivered");
        server.await.ok();
    }

    #[test]
    fn stale_timestamps_are_in_the_past() {
        let stale = stale_before(60);
        assert!(stale.as_str() < now_rfc3339().as_str());
    }
}
