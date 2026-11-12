//! Deterministic workflow execution for the block types that are replayable
//! without an LLM.
//!
//! Phase 2 supports `browserAction`, `wait`, `humanApproval`, `terminate` and
//! `goto`. AI, HTTP, loop and code blocks return `unsupported` until their
//! respective phases land. The executor deliberately fails closed: nothing is
//! skipped silently.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use boa_engine::{Context as JsContext, Source as JsSource};
use doonce_browser::{BrowserError, BrowserSession, DownloadInfo};
use doonce_core::risk::approval_reason;
use doonce_core::template;
use doonce_core::validation::{is_allowed_url, is_safe_outbound_url};
use doonce_core::{
    Block, BlockKind, BrowserAction, CodeLanguage, Condition, FileParser, Target, TerminateStatus,
    WorkflowDefinition,
};
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub element_timeout: Duration,
    pub navigation_timeout: Duration,
    pub download_timeout: Duration,
    pub start_block: Option<String>,
    pub max_steps: usize,
    /// When false, values are used literally (the AI agent already resolved
    /// them). Workflow replay keeps this enabled.
    pub render_templates: bool,
    /// Directory that fileParse blocks may read from.
    pub artifact_dir: PathBuf,
    /// HTTP client and timeout for httpRequest blocks.
    pub http: reqwest::Client,
    /// Wall-clock limit for a code block.
    pub code_timeout: Duration,
    /// Allow httpRequest blocks to reach private/loopback addresses. This is
    /// intended for local integrations and tests, not for untrusted workflows.
    pub allow_private_http: bool,
    pub cancel: CancellationToken,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            element_timeout: Duration::from_secs(10),
            navigation_timeout: Duration::from_secs(30),
            download_timeout: Duration::from_secs(60),
            start_block: None,
            max_steps: 1000,
            render_templates: true,
            artifact_dir: PathBuf::from("."),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            code_timeout: Duration::from_secs(5),
            allow_private_http: false,
            cancel: CancellationToken::new(),
        }
    }
}

struct LoopFrame {
    block_index: usize,
    body_index: usize,
    items: Vec<Value>,
    index: usize,
    item_name: String,
    collected: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    AwaitingApproval,
    Cancelled,
    Skipped,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Completed => "completed",
            StepStatus::Failed => "failed",
            StepStatus::AwaitingApproval => "awaiting_approval",
            StepStatus::Cancelled => "cancelled",
            StepStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    ElementNotFound,
    ConditionNotMet,
    ActionFailed,
    Navigation,
    Download,
    Browser,
    InvalidInput,
    Unsupported,
    ApprovalDenied,
    Cancelled,
    LoopDetected,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepFailure {
    pub kind: FailureKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Value>,
    /// True when the page may already have reacted, so automatic retries are
    /// unsafe.
    pub dispatched: bool,
}

impl StepFailure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        StepFailure {
            kind,
            message: message.into(),
            diagnostics: None,
            dispatched: false,
        }
    }

    fn dispatched(kind: FailureKind, message: impl Into<String>) -> Self {
        StepFailure {
            kind,
            message: message.into(),
            diagnostics: None,
            dispatched: true,
        }
    }

    fn from_browser(kind: FailureKind, error: BrowserError) -> Self {
        let kind = if error.is_disconnected() {
            FailureKind::Browser
        } else {
            kind
        };
        StepFailure::new(kind, error.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum ExecEvent {
    #[serde(rename_all = "camelCase")]
    Step {
        index: usize,
        block_id: String,
        status: StepStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        duration_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Download {
        block_id: String,
        info: DownloadInfo,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub block_id: String,
    pub summary: String,
    pub reason: String,
}

pub trait Observer: Send + Sync {
    fn on_event(&self, event: &ExecEvent);
    fn approve(&self, request: ApprovalRequest) -> impl Future<Output = bool> + Send;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub status: RunStatus,
    pub failed_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_block_index: Option<usize>,
    pub failure: Option<StepFailure>,
    pub downloads: Vec<DownloadInfo>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub outputs: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    pub username: Option<String>,
    pub secret: String,
    pub totp_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub struct AiTaskRequest<'a> {
    pub prompt: &'a str,
    pub url: Option<&'a str>,
    pub data_schema: Option<&'a Value>,
    pub error_codes: &'a [String],
    pub max_steps: u32,
    pub model: Option<&'a str>,
}

pub trait ApprovalCallback: Send + Sync {
    fn approve<'a>(&'a self, request: ApprovalRequest) -> BoxFuture<'a, bool>;
}

pub trait AiExecutor: Send + Sync {
    fn run_task<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        request: AiTaskRequest<'a>,
        approvals: &'a dyn ApprovalCallback,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;

    fn extract<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
        schema: &'a Value,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;

    fn validate<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
    ) -> BoxFuture<'a, std::result::Result<bool, String>>;

    fn prompt<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
        schema: Option<&'a Value>,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;

    fn text_prompt<'a>(
        &'a self,
        prompt: &'a str,
        model: Option<&'a str>,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;

    fn download_files<'a>(
        &'a self,
        browser: &'a mut BrowserSession,
        prompt: &'a str,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;
}

pub trait CredentialResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        credential_id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<ResolvedCredential, String>>;
}

pub trait EmailSender: Send + Sync {
    fn send<'a>(
        &'a self,
        message: EmailMessage,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;
}

pub trait ArtifactStore: Send + Sync {
    fn put<'a>(
        &'a self,
        artifact: &'a str,
        destination: &'a str,
    ) -> BoxFuture<'a, std::result::Result<Value, String>>;
}

pub trait WorkflowResolver: Send + Sync {
    fn load<'a>(
        &'a self,
        workflow_id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<WorkflowDefinition, String>>;
}

#[derive(Clone)]
pub struct Services {
    pub ai: Option<Arc<dyn AiExecutor>>,
    pub credentials: Option<Arc<dyn CredentialResolver>>,
    pub email: Option<Arc<dyn EmailSender>>,
    pub artifacts: Option<Arc<dyn ArtifactStore>>,
    pub workflows: Option<Arc<dyn WorkflowResolver>>,
    pub max_depth: usize,
}

impl Default for Services {
    fn default() -> Self {
        Services {
            ai: None,
            credentials: None,
            email: None,
            artifacts: None,
            workflows: None,
            max_depth: 3,
        }
    }
}

struct ObserverApproval<'a, O: Observer> {
    observer: &'a O,
}

impl<O: Observer> ApprovalCallback for ObserverApproval<'_, O> {
    fn approve<'a>(&'a self, request: ApprovalRequest) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.observer.approve(request).await })
    }
}

pub async fn run<O: Observer>(
    workflow: &WorkflowDefinition,
    params: &HashMap<String, String>,
    browser: &mut BrowserSession,
    observer: &O,
    options: &RunOptions,
) -> RunOutcome {
    run_with_services(
        workflow,
        params,
        browser,
        observer,
        options,
        &Services::default(),
    )
    .await
}

pub async fn run_with_services<O: Observer>(
    workflow: &WorkflowDefinition,
    params: &HashMap<String, String>,
    browser: &mut BrowserSession,
    observer: &O,
    options: &RunOptions,
    services: &Services,
) -> RunOutcome {
    let mut prepared = workflow.clone();
    if let Some(prompt) = prepared
        .prompt
        .clone()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        let mut id = "prompt".to_string();
        let mut suffix = 2u32;
        while prepared.block(&id).is_some() {
            id = format!("prompt-{suffix}");
            suffix += 1;
        }
        let next = prepared.blocks.first().map(|block| block.id.clone());
        prepared.blocks.insert(
            0,
            Block {
                id,
                kind: BlockKind::BrowserTask {
                    prompt,
                    url: prepared.url.clone(),
                    navigation_goal: None,
                    data_schema: prepared.data_schema.clone(),
                    error_codes: Vec::new(),
                    max_steps: prepared.max_steps,
                },
                label: Some("Workflow prompt".into()),
                description: None,
                next,
                on_error: None,
                retry: None,
                timeout_ms: None,
                continue_on_failure: None,
                requires_approval: None,
            },
        );
    }
    run_inner(&prepared, params, browser, observer, options, services, 0).await
}

async fn run_inner<O: Observer>(
    workflow: &WorkflowDefinition,
    params: &HashMap<String, String>,
    browser: &mut BrowserSession,
    observer: &O,
    options: &RunOptions,
    services: &Services,
    depth: usize,
) -> RunOutcome {
    let mut scope = params.clone();
    let mut outputs = HashMap::new();
    let mut downloads = Vec::new();
    let mut loop_frames: Vec<LoopFrame> = Vec::new();
    let mut last_output: Option<Value> = None;
    let index_by_id: HashMap<&str, usize> = workflow
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id.as_str(), index))
        .collect();

    let mut current = match &options.start_block {
        Some(id) => match index_by_id.get(id.as_str()) {
            Some(index) => *index,
            None => {
                return failed_outcome(
                    Some(id.clone()),
                    None,
                    StepFailure::new(
                        FailureKind::InvalidInput,
                        format!("start block \"{id}\" does not exist"),
                    ),
                    downloads,
                    outputs,
                );
            }
        },
        None => 0,
    };

    let mut executed = 0usize;
    loop {
        if options.cancel.is_cancelled() {
            return cancelled_outcome(
                workflow.blocks.get(current).map(|block| block.id.clone()),
                Some(current),
                StepFailure::new(FailureKind::Cancelled, "Run cancelled"),
                downloads,
                outputs,
            );
        }
        if executed >= options.max_steps {
            return failed_outcome(
                workflow.blocks.get(current).map(|block| block.id.clone()),
                Some(current),
                StepFailure::new(
                    FailureKind::LoopDetected,
                    format!("workflow exceeded the {} step limit", options.max_steps),
                ),
                downloads,
                outputs,
            );
        }
        if let Some(position) = loop_frames
            .iter()
            .position(|frame| frame.block_index == current)
        {
            let frame = &mut loop_frames[position];
            frame
                .collected
                .push(last_output.take().unwrap_or(Value::Null));
            frame.index += 1;
            if frame.index < frame.items.len() {
                let item = frame.items[frame.index].clone();
                set_scope_value(&mut scope, &frame.item_name, &item);
                scope.insert(format!("{}Index", frame.item_name), frame.index.to_string());
                scope.insert("itemIndex".to_string(), frame.index.to_string());
                current = frame.body_index;
                continue;
            }
            let frame = loop_frames.remove(position);
            let Some(block) = workflow.blocks.get(current) else {
                break;
            };
            record_output(
                &block.id,
                Value::Array(frame.collected),
                &mut scope,
                &mut outputs,
            );
            last_output = outputs.get(&block.id).cloned();
            let next = block
                .next
                .as_deref()
                .and_then(|id| index_by_id.get(id).copied())
                .unwrap_or(current + 1);
            if next >= workflow.blocks.len() {
                break;
            }
            current = next;
            continue;
        }

        let Some(block) = workflow.blocks.get(current) else {
            break;
        };

        let approval = if matches!(block.kind, BlockKind::HumanApproval { .. }) {
            Some("This block waits for explicit human approval.".to_string())
        } else {
            approval_reason(block)
        };
        if let Some(reason) = approval {
            emit(
                observer,
                current,
                block,
                StepStatus::AwaitingApproval,
                None,
                Instant::now(),
            );
            let request = ApprovalRequest {
                block_id: block.id.clone(),
                summary: block.summary(),
                reason,
            };
            let approved = tokio::select! {
                approved = observer.approve(request) => approved,
                _ = options.cancel.cancelled() => false,
            };
            if !approved {
                let message = if options.cancel.is_cancelled() {
                    "Run cancelled"
                } else {
                    "Not approved by the user"
                };
                let kind = if options.cancel.is_cancelled() {
                    FailureKind::Cancelled
                } else {
                    FailureKind::ApprovalDenied
                };
                emit(
                    observer,
                    current,
                    block,
                    StepStatus::Cancelled,
                    Some(message.into()),
                    Instant::now(),
                );
                return cancelled_outcome(
                    Some(block.id.clone()),
                    Some(current),
                    StepFailure::new(kind, message),
                    downloads,
                    outputs,
                );
            }
        }

        let started = Instant::now();
        emit(observer, current, block, StepStatus::Running, None, started);
        let result = tokio::select! {
            result = run_with_retry(block, &mut scope, &mut outputs, browser, observer, options, services, depth) => result,
            _ = options.cancel.cancelled() => Err(StepFailure::new(FailureKind::Cancelled, "Run cancelled")),
        };

        match result {
            Ok(Some(info)) => {
                observer.on_event(&ExecEvent::Download {
                    block_id: block.id.clone(),
                    info: info.clone(),
                });
                downloads.push(info);
                emit(
                    observer,
                    current,
                    block,
                    StepStatus::Completed,
                    None,
                    started,
                );
            }
            Ok(None) => {
                emit(
                    observer,
                    current,
                    block,
                    StepStatus::Completed,
                    None,
                    started,
                );
            }
            Err(failure) => {
                let cancelled = failure.kind == FailureKind::Cancelled;
                emit(
                    observer,
                    current,
                    block,
                    if cancelled {
                        StepStatus::Cancelled
                    } else {
                        StepStatus::Failed
                    },
                    Some(failure.message.clone()),
                    started,
                );
                return RunOutcome {
                    status: if cancelled {
                        RunStatus::Cancelled
                    } else {
                        RunStatus::Failed
                    },
                    failed_block: Some(block.id.clone()),
                    failed_block_index: Some(current),
                    failure: Some(failure),
                    downloads,
                    outputs,
                };
            }
        }

        last_output = outputs.get(&block.id).cloned();
        executed += 1;

        match &block.kind {
            BlockKind::Terminate { status, reason } => {
                return match status {
                    TerminateStatus::Completed => RunOutcome {
                        status: RunStatus::Completed,
                        failed_block: None,
                        failed_block_index: None,
                        failure: None,
                        downloads,
                        outputs,
                    },
                    TerminateStatus::Cancelled => cancelled_outcome(
                        Some(block.id.clone()),
                        Some(current),
                        StepFailure::new(
                            FailureKind::Cancelled,
                            reason
                                .clone()
                                .unwrap_or_else(|| "workflow terminated".into()),
                        ),
                        downloads,
                        outputs,
                    ),
                    TerminateStatus::Failed => failed_outcome(
                        Some(block.id.clone()),
                        Some(current),
                        StepFailure::new(
                            FailureKind::ActionFailed,
                            reason
                                .clone()
                                .unwrap_or_else(|| "workflow terminated with failure".into()),
                        ),
                        downloads,
                        outputs,
                    ),
                };
            }
            BlockKind::Conditional {
                condition,
                then,
                else_branch,
            } => {
                let result = match evaluate_condition(condition, &scope, &outputs) {
                    Ok(result) => result,
                    Err(failure) => {
                        return failed_outcome(
                            Some(block.id.clone()),
                            Some(current),
                            failure,
                            downloads,
                            outputs,
                        );
                    }
                };
                record_output(&block.id, Value::Bool(result), &mut scope, &mut outputs);
                last_output = outputs.get(&block.id).cloned();
                let target = if result {
                    Some(then.as_str())
                } else {
                    else_branch.as_deref()
                };
                let next = target
                    .and_then(|id| index_by_id.get(id).copied())
                    .or_else(|| {
                        block
                            .next
                            .as_deref()
                            .and_then(|id| index_by_id.get(id).copied())
                    })
                    .unwrap_or(current + 1);
                if next >= workflow.blocks.len() {
                    break;
                }
                current = next;
                continue;
            }
            BlockKind::ForLoop {
                over,
                body,
                item_name,
                max_iterations,
            } => {
                let mut items = match resolve_items(over, &scope) {
                    Ok(items) => items,
                    Err(failure) => {
                        return failed_outcome(
                            Some(block.id.clone()),
                            Some(current),
                            failure,
                            downloads,
                            outputs,
                        );
                    }
                };
                if let Some(max) = max_iterations {
                    items.truncate(*max as usize);
                }
                if items.is_empty() {
                    record_output(
                        &block.id,
                        Value::Array(Vec::new()),
                        &mut scope,
                        &mut outputs,
                    );
                    last_output = outputs.get(&block.id).cloned();
                    let next = block
                        .next
                        .as_deref()
                        .and_then(|id| index_by_id.get(id).copied())
                        .unwrap_or(current + 1);
                    if next >= workflow.blocks.len() {
                        break;
                    }
                    current = next;
                    continue;
                }
                let Some(body_index) = index_by_id.get(body.as_str()).copied() else {
                    return failed_outcome(
                        Some(block.id.clone()),
                        Some(current),
                        StepFailure::new(
                            FailureKind::InvalidInput,
                            format!("loop body \"{body}\" does not exist"),
                        ),
                        downloads,
                        outputs,
                    );
                };
                let item_name = item_name.clone().unwrap_or_else(|| "item".to_string());
                set_scope_value(&mut scope, &item_name, &items[0]);
                scope.insert(format!("{item_name}Index"), "0".to_string());
                scope.insert("itemIndex".to_string(), "0".to_string());
                loop_frames.push(LoopFrame {
                    block_index: current,
                    body_index,
                    items,
                    index: 0,
                    item_name,
                    collected: Vec::new(),
                });
                current = body_index;
                continue;
            }
            BlockKind::Goto { target } => {
                let Some(next) = index_by_id.get(target.as_str()) else {
                    return failed_outcome(
                        Some(block.id.clone()),
                        Some(current),
                        StepFailure::new(
                            FailureKind::InvalidInput,
                            format!("goto target \"{target}\" does not exist"),
                        ),
                        downloads,
                        outputs,
                    );
                };
                current = *next;
            }
            _ => {
                let next = block
                    .next
                    .as_deref()
                    .and_then(|id| index_by_id.get(id).copied())
                    .unwrap_or(current + 1);
                if next >= workflow.blocks.len() {
                    break;
                }
                current = next;
            }
        }
    }

    RunOutcome {
        status: RunStatus::Completed,
        failed_block: None,
        failed_block_index: None,
        failure: None,
        downloads,
        outputs,
    }
}

fn emit<O: Observer>(
    observer: &O,
    index: usize,
    block: &Block,
    status: StepStatus,
    error: Option<String>,
    started: Instant,
) {
    observer.on_event(&ExecEvent::Step {
        index,
        block_id: block.id.clone(),
        status,
        error,
        duration_ms: started.elapsed().as_millis() as u64,
    });
}

fn failed_outcome(
    block_id: Option<String>,
    index: Option<usize>,
    failure: StepFailure,
    downloads: Vec<DownloadInfo>,
    outputs: HashMap<String, Value>,
) -> RunOutcome {
    RunOutcome {
        status: RunStatus::Failed,
        failed_block: block_id,
        failed_block_index: index,
        failure: Some(failure),
        downloads,
        outputs,
    }
}

fn cancelled_outcome(
    block_id: Option<String>,
    index: Option<usize>,
    failure: StepFailure,
    downloads: Vec<DownloadInfo>,
    outputs: HashMap<String, Value>,
) -> RunOutcome {
    RunOutcome {
        status: RunStatus::Cancelled,
        failed_block: block_id,
        failed_block_index: index,
        failure: Some(failure),
        downloads,
        outputs,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_with_retry<O: Observer>(
    block: &Block,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    browser: &mut BrowserSession,
    observer: &O,
    options: &RunOptions,
    services: &Services,
    depth: usize,
) -> Result<Option<DownloadInfo>, StepFailure> {
    let attempts = block
        .retry
        .as_ref()
        .map_or(1, |retry| retry.attempts.max(1));
    let delay = Duration::from_millis(block.retry.as_ref().map_or(0, |retry| retry.delay_ms));
    let mut attempt = 1;
    loop {
        match run_block(
            block, scope, outputs, browser, observer, options, services, depth,
        )
        .await
        {
            Ok(value) => return Ok(value),
            Err(failure) => {
                let safe = !failure.dispatched;
                let retryable = !matches!(
                    failure.kind,
                    FailureKind::InvalidInput | FailureKind::Cancelled | FailureKind::Unsupported
                );
                if attempt >= attempts || !safe || !retryable {
                    return Err(failure);
                }
                attempt += 1;
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_block<O: Observer>(
    block: &Block,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    browser: &mut BrowserSession,
    observer: &O,
    options: &RunOptions,
    services: &Services,
    depth: usize,
) -> Result<Option<DownloadInfo>, StepFailure> {
    match &block.kind {
        BlockKind::BrowserAction { action } => {
            let action = if options.render_templates {
                render_action(action, scope)?
            } else {
                action.clone()
            };
            execute_action(block, &action, scope, outputs, browser, options).await
        }
        BlockKind::Wait { condition } => {
            let condition = if options.render_templates {
                render_condition(condition, scope)?
            } else {
                condition.clone()
            };
            wait_condition(
                block,
                browser,
                &condition,
                step_timeout(block, options.element_timeout),
                options,
            )
            .await?;
            Ok(None)
        }
        BlockKind::HumanApproval { .. }
        | BlockKind::Terminate { .. }
        | BlockKind::Goto { .. }
        | BlockKind::ForLoop { .. }
        | BlockKind::Conditional { .. } => Ok(None),
        BlockKind::HttpRequest {
            method,
            url,
            headers,
            body,
            save_as,
            auth,
        } => {
            execute_http_request(
                block,
                method,
                url,
                headers.as_ref(),
                body.as_ref(),
                save_as.as_deref(),
                auth.as_ref(),
                scope,
                outputs,
                options,
            )
            .await?;
            Ok(None)
        }
        BlockKind::Code {
            language,
            code,
            save_as,
        } => {
            execute_code(
                block,
                *language,
                code,
                save_as.as_deref(),
                scope,
                outputs,
                options,
            )
            .await?;
            Ok(None)
        }
        BlockKind::FileParse {
            artifact,
            parser,
            save_as,
        } => {
            execute_file_parse(
                block,
                artifact,
                *parser,
                save_as.as_deref(),
                scope,
                outputs,
                options,
            )?;
            Ok(None)
        }
        BlockKind::BrowserTask {
            prompt,
            url,
            data_schema,
            error_codes,
            max_steps,
            ..
        } => {
            let ai = services
                .ai
                .as_ref()
                .ok_or_else(|| missing_service("AI provider"))?;
            let prompt = render(prompt, scope)?;
            let url = url.as_ref().map(|value| render(value, scope)).transpose()?;
            let request = AiTaskRequest {
                prompt: &prompt,
                url: url.as_deref(),
                data_schema: data_schema.as_ref(),
                error_codes,
                max_steps: max_steps.unwrap_or(30),
                model: None,
            };
            let approvals = ObserverApproval { observer };
            let value = ai
                .run_task(browser, request, &approvals)
                .await
                .map_err(|error| service_failure("AI task", error))?;
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::Extract { prompt, schema, .. } => {
            let ai = services
                .ai
                .as_ref()
                .ok_or_else(|| missing_service("AI provider"))?;
            let prompt = render(prompt, scope)?;
            let value = ai
                .extract(browser, &prompt, schema)
                .await
                .map_err(|error| service_failure("AI extract", error))?;
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::Validate { prompt, .. } => {
            let ai = services
                .ai
                .as_ref()
                .ok_or_else(|| missing_service("AI provider"))?;
            let prompt = render(prompt, scope)?;
            let valid = ai
                .validate(browser, &prompt)
                .await
                .map_err(|error| service_failure("AI validate", error))?;
            record_output(&block.id, Value::Bool(valid), scope, outputs);
            Ok(None)
        }
        BlockKind::TextPrompt {
            prompt,
            model,
            save_as,
        } => {
            let ai = services
                .ai
                .as_ref()
                .ok_or_else(|| missing_service("AI provider"))?;
            let prompt = render(prompt, scope)?;
            let value = ai
                .text_prompt(&prompt, model.as_deref())
                .await
                .map_err(|error| service_failure("AI text prompt", error))?;
            if let Some(save_as) = save_as {
                outputs.insert(save_as.clone(), value.clone());
                set_scope_value(scope, save_as, &value);
            }
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::DownloadFiles { prompt, save_to } => {
            let ai = services
                .ai
                .as_ref()
                .ok_or_else(|| missing_service("AI provider"))?;
            let prompt = render(prompt, scope)?;
            let value = ai
                .download_files(browser, &prompt)
                .await
                .map_err(|error| service_failure("downloadFiles", error))?;
            if let Some(save_to) = save_to {
                outputs.insert(save_to.clone(), value.clone());
                set_scope_value(scope, save_to, &value);
            }
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::SendEmail {
            to,
            subject,
            body,
            attachments,
        } => {
            let email = services
                .email
                .as_ref()
                .ok_or_else(|| missing_service("email sender"))?;
            let to: Vec<String> = to
                .iter()
                .map(|address| render(address, scope))
                .collect::<Result<Vec<_>, _>>()?;
            let subject = render(subject, scope)?;
            let body = render(body, scope)?;
            let attachments = attachments
                .iter()
                .map(|attachment| {
                    let attachment = render(attachment, scope)?;
                    safe_artifact_path(&options.artifact_dir, &attachment)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let message = EmailMessage {
                to,
                subject,
                body,
                attachments,
            };
            let value = email
                .send(message)
                .await
                .map_err(|error| service_failure("sendEmail", error))?;
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::Upload {
            artifact,
            destination,
        } => {
            let store = services
                .artifacts
                .as_ref()
                .ok_or_else(|| missing_service("artifact store"))?;
            let artifact = render(artifact, scope)?;
            let path = safe_artifact_path(&options.artifact_dir, &artifact)?;
            let destination = render(destination, scope)?;
            let value = store
                .put(path.to_string_lossy().as_ref(), &destination)
                .await
                .map_err(|error| service_failure("upload", error))?;
            record_output(&block.id, value, scope, outputs);
            Ok(None)
        }
        BlockKind::Login {
            credential_id,
            totp,
        } => {
            let resolver = services
                .credentials
                .as_ref()
                .ok_or_else(|| missing_service("credential vault"))?;
            let credential_id = render(credential_id, scope)?;
            let credential = resolver
                .resolve(&credential_id)
                .await
                .map_err(|error| service_failure("login", error))?;
            execute_login(block, *totp, credential, browser, scope, outputs, options).await?;
            Ok(None)
        }
        BlockKind::RunWorkflow {
            workflow_id,
            inputs,
        } => {
            let resolver = services
                .workflows
                .as_ref()
                .ok_or_else(|| missing_service("workflow resolver"))?;
            if depth >= services.max_depth {
                return Err(StepFailure::new(
                    FailureKind::LoopDetected,
                    "nested workflow depth exceeded",
                ));
            }
            let workflow_id = render(workflow_id, scope)?;
            let definition = resolver
                .load(&workflow_id)
                .await
                .map_err(|error| service_failure("runWorkflow", error))?;
            let nested_params = nested_params(inputs.as_ref(), scope)?;
            let outcome = Box::pin(run_inner(
                &definition,
                &nested_params,
                browser,
                observer,
                options,
                services,
                depth + 1,
            ))
            .await;
            let RunOutcome {
                status,
                outputs: nested_outputs,
                failure,
                ..
            } = outcome;
            let mut output = json!({ "status": status.as_str(), "outputs": nested_outputs });
            if let Some(failure) = &failure {
                output["error"] = json!(failure.message);
            }
            record_output(&block.id, output, scope, outputs);
            if status != RunStatus::Completed {
                return Err(StepFailure::new(
                    FailureKind::ActionFailed,
                    format!("nested workflow {workflow_id} did not complete"),
                ));
            }
            Ok(None)
        }
    }
}

async fn execute_action(
    block: &Block,
    action: &BrowserAction,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    browser: &mut BrowserSession,
    options: &RunOptions,
) -> Result<Option<DownloadInfo>, StepFailure> {
    let timeout = step_timeout(block, options.element_timeout);
    match action {
        BrowserAction::Navigate { url } => {
            if !is_allowed_url(url) {
                return Err(StepFailure::new(
                    FailureKind::InvalidInput,
                    "only http(s) URLs can be opened",
                ));
            }
            browser
                .navigate(url, step_timeout(block, options.navigation_timeout))
                .await
                .map_err(|error| StepFailure::from_browser(FailureKind::Navigation, error))?;
        }
        BrowserAction::Click { target } => {
            act(
                browser,
                target,
                json!({ "action": "click" }),
                timeout,
                options,
            )
            .await?;
        }
        BrowserAction::Hover { target } => {
            act(
                browser,
                target,
                json!({ "action": "hover" }),
                timeout,
                options,
            )
            .await?;
        }
        BrowserAction::Fill { target, value } => {
            act(
                browser,
                target,
                json!({ "action": "fill", "value": value }),
                timeout,
                options,
            )
            .await?;
        }
        BrowserAction::Select { target, value } => {
            act(
                browser,
                target,
                json!({ "action": "select", "value": value }),
                timeout,
                options,
            )
            .await?;
        }
        BrowserAction::Check { target, checked } => {
            act(
                browser,
                target,
                json!({ "action": "check", "checked": checked }),
                timeout,
                options,
            )
            .await?;
        }
        BrowserAction::Press { target, key } => match target {
            Some(target) => {
                act(
                    browser,
                    target,
                    json!({ "action": "press", "key": key }),
                    timeout,
                    options,
                )
                .await?;
            }
            None => {
                let result = browser
                    .perform(
                        json!({ "action": "press", "key": key }),
                        options.navigation_timeout,
                    )
                    .await
                    .map_err(|error| {
                        StepFailure::dispatched(FailureKind::ActionFailed, error.to_string())
                    })?;
                check_perform(&result, "press")?;
            }
        },
        BrowserAction::Download { target } => {
            browser.expect_download();
            if let Some(target) = target {
                act(
                    browser,
                    target,
                    json!({ "action": "click" }),
                    timeout,
                    options,
                )
                .await?;
            }
            let info = browser
                .wait_download(step_timeout(block, options.download_timeout))
                .await
                .map_err(|error| {
                    StepFailure::dispatched(FailureKind::Download, error.to_string())
                })?;
            return Ok(Some(info));
        }
        BrowserAction::SwitchTab { url_contains } => {
            browser
                .switch_tab(url_contains, timeout)
                .await
                .map_err(|error| StepFailure::from_browser(FailureKind::Navigation, error))?;
        }
        BrowserAction::Wait { condition } => {
            let condition = if options.render_templates {
                render_condition(condition, scope)?
            } else {
                condition.clone()
            };
            wait_condition(block, browser, &condition, timeout, options).await?;
        }
        BrowserAction::Assert { condition } => {
            let condition = if options.render_templates {
                render_condition(condition, scope)?
            } else {
                condition.clone()
            };
            wait_condition(block, browser, &condition, timeout, options)
                .await
                .map_err(|mut failure| {
                    failure.message = format!("Assertion failed: {}", failure.message);
                    failure
                })?;
        }
        BrowserAction::Extract {
            target,
            var,
            attribute,
        } => {
            wait_for_element(browser, target, Instant::now() + timeout).await?;
            let result = browser
                .query(
                    "extract",
                    json!({ "target": target, "attribute": attribute }),
                )
                .await
                .map_err(|error| StepFailure::from_browser(FailureKind::ActionFailed, error))?;
            let value = result.get("value").and_then(Value::as_str).ok_or_else(|| {
                StepFailure::new(FailureKind::ActionFailed, "extract returned no value")
            })?;
            outputs.insert(var.clone(), Value::String(value.to_string()));
            scope.insert(var.clone(), value.to_string());
        }
        BrowserAction::Upload { .. } => {
            return Err(StepFailure::new(
                FailureKind::Unsupported,
                "file upload replay is not available yet",
            ));
        }
    }
    Ok(None)
}

fn step_timeout(block: &Block, default: Duration) -> Duration {
    block
        .timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn render(value: &str, scope: &HashMap<String, String>) -> Result<String, StepFailure> {
    template::render(value, scope)
        .map_err(|error| StepFailure::new(FailureKind::InvalidInput, error.to_string()))
}

fn render_action(
    action: &BrowserAction,
    scope: &HashMap<String, String>,
) -> Result<BrowserAction, StepFailure> {
    Ok(match action.clone() {
        BrowserAction::Navigate { url } => BrowserAction::Navigate {
            url: render(&url, scope)?,
        },
        BrowserAction::Fill { target, value } => BrowserAction::Fill {
            target,
            value: render(&value, scope)?,
        },
        BrowserAction::Select { target, value } => BrowserAction::Select {
            target,
            value: render(&value, scope)?,
        },
        BrowserAction::SwitchTab { url_contains } => BrowserAction::SwitchTab {
            url_contains: render(&url_contains, scope)?,
        },
        BrowserAction::Wait { condition } => BrowserAction::Wait {
            condition: render_condition(&condition, scope)?,
        },
        BrowserAction::Assert { condition } => BrowserAction::Assert {
            condition: render_condition(&condition, scope)?,
        },
        other => other,
    })
}

fn render_condition(
    condition: &Condition,
    scope: &HashMap<String, String>,
) -> Result<Condition, StepFailure> {
    let mut rendered = condition.clone();
    if let Some(url) = &condition.url_contains {
        rendered.url_contains = Some(render(url, scope)?);
    }
    if let Some(text) = &condition.text_contains {
        rendered.text_contains = Some(render(text, scope)?);
    }
    Ok(rendered)
}

const WAITABLE: &[&str] = &["notFound", "hidden", "disabled", "ambiguous", "obscured"];

async fn act(
    browser: &mut BrowserSession,
    target: &doonce_core::Target,
    mut args: Value,
    timeout: Duration,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    let action = args["action"].as_str().unwrap_or("action").to_string();
    args["target"] = serde_json::to_value(target).expect("target serializes");
    let deadline = Instant::now() + timeout;
    loop {
        wait_for_element(browser, target, deadline).await?;
        let result = browser
            .perform(args.clone(), options.navigation_timeout)
            .await
            .map_err(|error| {
                StepFailure::dispatched(FailureKind::ActionFailed, error.to_string())
            })?;
        let code = result
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let dispatched = result
            .get("dispatched")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if result.get("ok").and_then(Value::as_bool) != Some(true)
            && !dispatched
            && WAITABLE.contains(&code)
            && Instant::now() < deadline
        {
            tokio::time::sleep(POLL).await;
            continue;
        }
        return check_perform(&result, &action);
    }
}

fn check_perform(result: &Value, action: &str) -> Result<(), StepFailure> {
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let message = result
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("action failed");
    Err(StepFailure {
        kind: FailureKind::ActionFailed,
        message: format!("{action} failed: {message}"),
        diagnostics: None,
        dispatched: result
            .get("dispatched")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

async fn wait_for_element(
    browser: &mut BrowserSession,
    target: &doonce_core::Target,
    deadline: Instant,
) -> Result<(), StepFailure> {
    let args = json!({ "target": target });
    let mut last: Option<Value> = None;
    let mut last_error: Option<BrowserError> = None;
    loop {
        match browser.query("locate", args.clone()).await {
            Ok(result) if result.get("status").and_then(Value::as_str) == Some("found") => {
                return Ok(());
            }
            Ok(result) => last = Some(result),
            Err(error) if error.is_disconnected() => {
                return Err(StepFailure::from_browser(FailureKind::Browser, error));
            }
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    let what = target.describe();
    let (message, diagnostics) = match last {
        Some(result) => {
            let status = result
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("notFound");
            let detail = match status {
                "ambiguous" => "several similar elements match",
                "hidden" => "it exists but is not visible",
                "disabled" => "it is disabled",
                _ => "no matching element on the page",
            };
            (format!("Could not find {what}: {detail}"), Some(result))
        }
        None => (
            format!(
                "Could not find {what}: {}",
                last_error.map_or("page not ready".into(), |error| error.to_string())
            ),
            None,
        ),
    };
    Err(StepFailure {
        kind: FailureKind::ElementNotFound,
        message,
        diagnostics,
        dispatched: false,
    })
}

async fn wait_condition(
    block: &Block,
    browser: &mut BrowserSession,
    condition: &Condition,
    timeout: Duration,
    _options: &RunOptions,
) -> Result<(), StepFailure> {
    let args = json!({ "condition": condition });
    let deadline = Instant::now() + timeout;
    let mut failed = String::from("condition");
    loop {
        match browser.query("check", args.clone()).await {
            Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(true) => return Ok(()),
            Ok(result) => {
                failed = result
                    .get("failed")
                    .and_then(Value::as_str)
                    .unwrap_or("condition")
                    .to_string();
            }
            Err(error) if error.is_disconnected() => {
                return Err(StepFailure::from_browser(FailureKind::Browser, error));
            }
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            let what = match failed.as_str() {
                "urlContains" => "the page URL did not match",
                "textContains" => "the expected text did not appear",
                "visible" => "the expected element did not appear",
                "hidden" => "the element did not go away",
                _ => "the condition was not met",
            };
            return Err(StepFailure::new(
                FailureKind::ConditionNotMet,
                format!("{what} within {}s ({})", timeout.as_secs(), block.summary()),
            ));
        }
        tokio::time::sleep(POLL).await;
    }
}

fn record_output(
    block_id: &str,
    value: Value,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
) {
    outputs.insert(block_id.to_string(), value.clone());
    set_scope_value(scope, &format!("{block_id}.output"), &value);
}

fn set_scope_value(scope: &mut HashMap<String, String>, prefix: &str, value: &Value) {
    let raw = match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    scope.insert(prefix.to_string(), raw);
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                set_scope_value(scope, &format!("{prefix}.{key}"), child);
            }
        }
        Value::Array(_) => {
            // The whole array is available as JSON under `prefix`. Numeric
            // index keys would turn the array into an object in
            // `scope_to_json`; use a code block for index access.
        }
        _ => {}
    }
}

fn scope_to_json(scope: &HashMap<String, String>, outputs: &HashMap<String, Value>) -> Value {
    let mut root = Map::new();
    let mut entries: Vec<(&String, &String)> = scope.iter().collect();
    // Insert less specific paths first so a later `block.output.field` can
    // refine `block.output` instead of replacing it.
    entries.sort_by(|left, right| {
        left.0
            .matches('.')
            .count()
            .cmp(&right.0.matches('.').count())
            .then_with(|| left.0.cmp(right.0))
    });
    for (key, raw) in entries {
        let value =
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.clone()));
        insert_path(&mut root, key, value);
    }
    if !outputs.is_empty() {
        root.insert(
            "outputs".to_string(),
            serde_json::to_value(outputs).unwrap_or(Value::Null),
        );
    }
    Value::Object(root)
}

fn insert_path(root: &mut Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    insert_path_parts(root, &parts, value);
}

fn insert_path_parts(map: &mut Map<String, Value>, parts: &[&str], value: Value) {
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        map.insert(parts[0].to_string(), value);
        return;
    }
    let entry = map
        .entry(parts[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Value::Object(child) = entry {
        insert_path_parts(child, &parts[1..], value);
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn evaluate_condition(
    condition: &Value,
    scope: &HashMap<String, String>,
    outputs: &HashMap<String, Value>,
) -> Result<bool, StepFailure> {
    let data = scope_to_json(scope, outputs);
    let result = jsonlogic::apply(condition, &data).map_err(|error| {
        StepFailure::new(FailureKind::InvalidInput, format!("condition: {error}"))
    })?;
    Ok(truthy(&result))
}

fn resolve_items(over: &str, scope: &HashMap<String, String>) -> Result<Vec<Value>, StepFailure> {
    let raw = if over.contains("{{") {
        template::render(over, scope)
            .map_err(|error| StepFailure::new(FailureKind::InvalidInput, error.to_string()))?
    } else if let Some(value) = scope.get(over) {
        value.clone()
    } else {
        over.to_string()
    };
    let value: Value = serde_json::from_str(&raw).map_err(|_| {
        StepFailure::new(
            FailureKind::InvalidInput,
            format!(
                "loop source does not resolve to a JSON array: {}",
                truncate_text(&raw, 120)
            ),
        )
    })?;
    match value {
        Value::Array(items) => Ok(items),
        other => Ok(vec![other]),
    }
}

fn render_json_value(value: &Value, scope: &HashMap<String, String>) -> Result<Value, StepFailure> {
    Ok(match value {
        Value::String(text) => Value::String(render(text, scope)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_json_value(item, scope))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(map) => {
            let mut output = Map::new();
            for (key, child) in map {
                output.insert(key.clone(), render_json_value(child, scope)?);
            }
            Value::Object(output)
        }
        other => other.clone(),
    })
}

fn truncate_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect::<String>() + "…"
    }
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

async fn assert_public_host(url: &reqwest::Url) -> Result<(), StepFailure> {
    let host = url
        .host_str()
        .ok_or_else(|| StepFailure::new(FailureKind::InvalidInput, "HTTP URL has no host"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            StepFailure::new(
                FailureKind::ActionFailed,
                format!("DNS lookup failed: {error}"),
            )
        })?
        .collect();
    if addresses.is_empty() {
        return Err(StepFailure::new(
            FailureKind::ActionFailed,
            "DNS lookup returned no addresses",
        ));
    }
    if addresses.iter().any(|address| is_blocked_ip(address.ip())) {
        return Err(StepFailure::new(
            FailureKind::InvalidInput,
            "HTTP request resolved to a private, loopback or link-local address",
        ));
    }
    Ok(())
}

fn forbidden_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection"
    )
}

#[allow(clippy::too_many_arguments)]
async fn execute_http_request(
    block: &Block,
    method: &str,
    url: &str,
    headers: Option<&std::collections::BTreeMap<String, String>>,
    body: Option<&Value>,
    save_as: Option<&str>,
    auth: Option<&doonce_core::HttpAuth>,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    let url = render(url, scope)?;
    let parsed = reqwest::Url::parse(&url).map_err(|error| {
        StepFailure::new(FailureKind::InvalidInput, format!("invalid URL: {error}"))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(StepFailure::new(
            FailureKind::InvalidInput,
            "HTTP URL must be http(s)",
        ));
    }
    if !options.allow_private_http {
        if !is_safe_outbound_url(&url) {
            return Err(StepFailure::new(
                FailureKind::InvalidInput,
                "HTTP URL must be public http(s)",
            ));
        }
        assert_public_host(&parsed).await?;
    }
    let method =
        reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes()).map_err(|error| {
            StepFailure::new(
                FailureKind::InvalidInput,
                format!("invalid method: {error}"),
            )
        })?;
    let mut request = options
        .http
        .request(method, parsed)
        .timeout(step_timeout(block, Duration::from_secs(30)));

    if let Some(headers) = headers {
        for (name, value) in headers {
            if forbidden_header(name) {
                return Err(StepFailure::new(
                    FailureKind::InvalidInput,
                    format!("header \"{name}\" is not allowed"),
                ));
            }
            let name =
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                    StepFailure::new(
                        FailureKind::InvalidInput,
                        format!("invalid header name: {error}"),
                    )
                })?;
            let value = reqwest::header::HeaderValue::from_str(&render(value, scope)?).map_err(
                |error| {
                    StepFailure::new(
                        FailureKind::InvalidInput,
                        format!("invalid header value: {error}"),
                    )
                },
            )?;
            request = request.header(name, value);
        }
    }

    if let Some(auth) = auth {
        match auth {
            doonce_core::HttpAuth::Basic { username, password } => {
                request =
                    request.basic_auth(render(username, scope)?, Some(render(password, scope)?));
            }
            doonce_core::HttpAuth::Bearer { token } => {
                request = request.bearer_auth(render(token, scope)?);
            }
            doonce_core::HttpAuth::Header { name, value } => {
                if forbidden_header(name) {
                    return Err(StepFailure::new(
                        FailureKind::InvalidInput,
                        "forbidden auth header",
                    ));
                }
                request = request.header(name, render(value, scope)?);
            }
        }
    }

    if let Some(body) = body {
        request = request.json(&render_json_value(body, scope)?);
    }

    let response = request.send().await.map_err(|error| {
        StepFailure::new(
            FailureKind::ActionFailed,
            format!("HTTP request failed: {error}"),
        )
    })?;
    let status = response.status();
    let headers: Map<String, Value> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                Value::String(value.to_str().unwrap_or_default().to_string()),
            )
        })
        .collect();
    let bytes = response.bytes().await.map_err(|error| {
        StepFailure::new(
            FailureKind::ActionFailed,
            format!("could not read response: {error}"),
        )
    })?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(StepFailure::new(
            FailureKind::ActionFailed,
            "HTTP response exceeds 2 MB",
        ));
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body_value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    let output = json!({
        "status": status.as_u16(),
        "headers": headers,
        "body": body_value
    });
    if !status.is_success() {
        return Err(StepFailure::new(
            FailureKind::ActionFailed,
            format!(
                "HTTP {}: {}",
                status.as_u16(),
                truncate_text(&output["body"].to_string(), 200)
            ),
        ));
    }
    if let Some(save_as) = save_as {
        outputs.insert(save_as.to_string(), output.clone());
        set_scope_value(scope, save_as, &output);
    }
    record_output(&block.id, output, scope, outputs);
    Ok(())
}

fn run_js(code: &str, inputs: &str) -> Result<Value, String> {
    let mut context = JsContext::default();
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(100_000);
    context.runtime_limits_mut().set_recursion_limit(1_000);
    let setup = format!("globalThis.input = {inputs};");
    context
        .eval(JsSource::from_bytes(setup.as_bytes()))
        .map_err(|error| error.to_string())?;
    let result = context
        .eval(JsSource::from_bytes(code.as_bytes()))
        .map_err(|error| error.to_string())?;
    let global = context
        .eval(JsSource::from_bytes(b"globalThis.result"))
        .map_err(|error| error.to_string())?;
    let value = if global.is_undefined() {
        result
    } else {
        global
    };
    value
        .to_json(&mut context)
        .map_err(|error| error.to_string())
}

async fn execute_code(
    block: &Block,
    language: CodeLanguage,
    code: &str,
    save_as: Option<&str>,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    if language != CodeLanguage::Javascript {
        return Err(StepFailure::new(
            FailureKind::Unsupported,
            "only JavaScript code blocks are supported in this build",
        ));
    }
    let inputs = scope_to_json(scope, outputs);
    let inputs = serde_json::to_string(&inputs)
        .map_err(|error| StepFailure::new(FailureKind::InvalidInput, error.to_string()))?;
    let code = code.to_string();
    let task = tokio::task::spawn_blocking(move || run_js(&code, &inputs));
    let value = match tokio::time::timeout(options.code_timeout, task).await {
        Ok(Ok(Ok(value))) => value,
        Ok(Ok(Err(error))) => {
            return Err(StepFailure::new(
                FailureKind::ActionFailed,
                format!("code error: {error}"),
            ));
        }
        Ok(Err(error)) => {
            return Err(StepFailure::new(
                FailureKind::ActionFailed,
                format!("code task failed: {error}"),
            ));
        }
        Err(_) => {
            return Err(StepFailure::new(
                FailureKind::ActionFailed,
                "code execution timed out",
            ));
        }
    };
    if let Some(save_as) = save_as {
        outputs.insert(save_as.to_string(), value.clone());
        set_scope_value(scope, save_as, &value);
    }
    record_output(&block.id, value, scope, outputs);
    Ok(())
}

fn safe_artifact_path(base: &Path, artifact: &str) -> Result<PathBuf, StepFailure> {
    use std::path::Component;
    let relative = Path::new(artifact);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(StepFailure::new(
            FailureKind::InvalidInput,
            "artifact path must stay inside the artifact directory",
        ));
    }
    let base = std::fs::canonicalize(base).map_err(|error| {
        StepFailure::new(
            FailureKind::InvalidInput,
            format!("artifact directory: {error}"),
        )
    })?;
    let path = std::fs::canonicalize(base.join(relative)).map_err(|error| {
        StepFailure::new(FailureKind::InvalidInput, format!("artifact: {error}"))
    })?;
    if !path.starts_with(&base) {
        return Err(StepFailure::new(
            FailureKind::InvalidInput,
            "artifact path escaped the artifact directory",
        ));
    }
    Ok(path)
}

fn execute_file_parse(
    block: &Block,
    artifact: &str,
    parser: FileParser,
    save_as: Option<&str>,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    let artifact = render(artifact, scope)?;
    let path = safe_artifact_path(&options.artifact_dir, &artifact)?;
    let value = match parser {
        FileParser::Json => {
            let text = std::fs::read_to_string(&path).map_err(|error| {
                StepFailure::new(
                    FailureKind::ActionFailed,
                    format!("could not read file: {error}"),
                )
            })?;
            serde_json::from_str(&text).map_err(|error| {
                StepFailure::new(
                    FailureKind::InvalidInput,
                    format!("invalid JSON file: {error}"),
                )
            })?
        }
        FileParser::Csv => {
            let mut reader = csv::Reader::from_path(&path).map_err(|error| {
                StepFailure::new(
                    FailureKind::ActionFailed,
                    format!("could not read CSV: {error}"),
                )
            })?;
            let headers = reader
                .headers()
                .map_err(|error| {
                    StepFailure::new(FailureKind::InvalidInput, format!("CSV headers: {error}"))
                })?
                .clone();
            let mut rows = Vec::new();
            for record in reader.records() {
                let record = record.map_err(|error| {
                    StepFailure::new(FailureKind::InvalidInput, format!("CSV row: {error}"))
                })?;
                let mut row = Map::new();
                for (index, header) in headers.iter().enumerate() {
                    row.insert(
                        header.to_string(),
                        Value::String(record.get(index).unwrap_or_default().to_string()),
                    );
                }
                rows.push(Value::Object(row));
            }
            Value::Array(rows)
        }
        FileParser::Xlsx | FileParser::Xml | FileParser::Pdf => {
            return Err(StepFailure::new(
                FailureKind::Unsupported,
                "this file parser is not available yet (csv/json supported)",
            ));
        }
    };
    if let Some(save_as) = save_as {
        outputs.insert(save_as.to_string(), value.clone());
        set_scope_value(scope, save_as, &value);
    }
    record_output(&block.id, value, scope, outputs);
    Ok(())
}

fn missing_service(name: &str) -> StepFailure {
    StepFailure::new(
        FailureKind::Unsupported,
        format!("{name} is not configured for this workflow runner"),
    )
}

fn service_failure(name: &str, detail: String) -> StepFailure {
    StepFailure::new(FailureKind::ActionFailed, format!("{name}: {detail}"))
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn nested_params(
    inputs: Option<&Value>,
    scope: &HashMap<String, String>,
) -> Result<HashMap<String, String>, StepFailure> {
    match inputs {
        None => Ok(scope.clone()),
        Some(Value::Object(map)) => Ok(map
            .iter()
            .map(|(key, value)| (key.clone(), value_to_string(value)))
            .collect()),
        Some(_) => Err(StepFailure::new(
            FailureKind::InvalidInput,
            "runWorkflow inputs must be an object",
        )),
    }
}

async fn execute_login(
    block: &Block,
    totp: bool,
    credential: ResolvedCredential,
    browser: &mut BrowserSession,
    scope: &mut HashMap<String, String>,
    outputs: &mut HashMap<String, Value>,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    let timeout = step_timeout(block, options.element_timeout);
    let short = Duration::from_secs(3).min(timeout);
    let mut username_filled = false;
    if let Some(username) = credential.username.as_deref() {
        let candidates = [
            Target {
                role: Some("textbox".into()),
                attributes: [("autocomplete".to_string(), "username".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            Target {
                label: Some("Email".into()),
                ..Default::default()
            },
            Target {
                label: Some("Username".into()),
                ..Default::default()
            },
            Target {
                label: Some("E-posta".into()),
                ..Default::default()
            },
            Target {
                label: Some("Kullanıcı Adı".into()),
                ..Default::default()
            },
            Target {
                selectors: vec![
                    "input[type=email]".into(),
                    "input[autocomplete=username]".into(),
                ],
                ..Default::default()
            },
        ];
        for target in candidates {
            if act(
                browser,
                &target,
                json!({ "action": "fill", "value": username }),
                short,
                options,
            )
            .await
            .is_ok()
            {
                username_filled = true;
                break;
            }
        }
    }

    let password_target = Target {
        selectors: vec!["input[type=password]".into()],
        ..Default::default()
    };
    act(
        browser,
        &password_target,
        json!({ "action": "fill", "value": credential.secret }),
        timeout,
        options,
    )
    .await?;

    let otp_target = Target {
        selectors: vec![
            "input[autocomplete=\"one-time-code\"]".into(),
            "input[name*=\"otp\"]".into(),
            "input[id*=\"otp\"]".into(),
            "input[name*=\"code\"]".into(),
            "input[id*=\"code\"]".into(),
        ],
        ..Default::default()
    };
    if totp {
        let code = credential.totp_code.ok_or_else(|| {
            StepFailure::new(
                FailureKind::InvalidInput,
                "login requires a TOTP code but the credential has no TOTP secret",
            )
        })?;
        act(
            browser,
            &otp_target,
            json!({ "action": "fill", "value": code }),
            short,
            options,
        )
        .await?;
        let _ = act(
            browser,
            &otp_target,
            json!({ "action": "press", "key": "Enter" }),
            short,
            options,
        )
        .await;
    } else {
        let _ = act(
            browser,
            &password_target,
            json!({ "action": "press", "key": "Enter" }),
            short,
            options,
        )
        .await;
    }

    record_output(
        &block.id,
        json!({ "loggedIn": true, "usernameFilled": username_filled }),
        scope,
        outputs,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_action_replaces_parameters_and_rejects_unknowns() {
        let scope = HashMap::from([("date".to_string(), "2026-11-06".to_string())]);
        let action = BrowserAction::Navigate {
            url: "https://example.test/?d={{date}}".into(),
        };
        match render_action(&action, &scope).unwrap() {
            BrowserAction::Navigate { url } => {
                assert_eq!(url, "https://example.test/?d=2026-11-06")
            }
            other => panic!("unexpected action: {other:?}"),
        }
        let missing = BrowserAction::Fill {
            target: doonce_core::Target {
                label: Some("Name".into()),
                ..Default::default()
            },
            value: "{{missing}}".into(),
        };
        assert!(matches!(
            render_action(&missing, &scope),
            Err(StepFailure {
                kind: FailureKind::InvalidInput,
                ..
            })
        ));
    }

    #[test]
    fn js_can_read_nested_arrays() {
        let inputs = r#"{"data":{"body":{"items":[1,2,3]}}}"#;
        let value = run_js("globalThis.result = input.data.body.items.length;", inputs).unwrap();
        assert_eq!(value, 3);
    }

    #[test]
    fn scope_json_nests_outputs() {
        let mut scope = HashMap::new();
        let output = json!({"body":{"items":[1,2,3]}});
        set_scope_value(&mut scope, "data", &output);
        let data = scope_to_json(&scope, &HashMap::new());
        assert_eq!(data["data"]["body"]["items"], json!([1, 2, 3]));
    }

    #[test]
    fn terminal_status_maps() {
        assert_eq!(StepStatus::Completed.as_str(), "completed");
        assert_eq!(RunStatus::Cancelled.as_str(), "cancelled");
    }
}
