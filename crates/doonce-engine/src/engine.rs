//! Deterministic replay: runs workflow steps one by one through a
//! [`Driver`], waiting on page conditions instead of fixed sleeps.

use std::future::Future;
use std::time::{Duration, Instant};

use doonce_core::risk::approval_reason;
use doonce_core::validate::is_allowed_url;
use doonce_core::{Action, Condition, ResolvedParams, Step, Target, Workflow};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::driver::{DownloadInfo, Driver, DriverError};

const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// How long to wait for an element or condition.
    pub element_timeout: Duration,
    pub navigation_timeout: Duration,
    pub download_timeout: Duration,
    pub cancel: CancellationToken,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            element_timeout: Duration::from_secs(10),
            navigation_timeout: Duration::from_secs(30),
            download_timeout: Duration::from_secs(60),
            cancel: CancellationToken::new(),
        }
    }
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

/// Why a step failed. Messages never contain parameter values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    /// The target element could not be found unambiguously. This is the
    /// case self-healing can repair.
    ElementNotFound,
    ConditionNotMet,
    ActionFailed,
    Navigation,
    Download,
    Browser,
    InvalidInput,
    ApprovalDenied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepFailure {
    pub kind: FailureKind,
    pub message: String,
    /// Locator diagnostics (candidate summaries) for element failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Value>,
    /// True when the page may already have reacted to the action, which
    /// rules out automatic retries.
    pub dispatched: bool,
}

impl StepFailure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        StepFailure { kind, message: message.into(), diagnostics: None, dispatched: false }
    }

    fn from_driver(kind: FailureKind, e: DriverError) -> Self {
        let kind = if matches!(e, DriverError::Disconnected) { FailureKind::Browser } else { kind };
        StepFailure::new(kind, e.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum ExecEvent {
    #[serde(rename_all = "camelCase")]
    Step {
        index: usize,
        step_id: String,
        status: StepStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        duration_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Download { step_id: String, info: DownloadInfo },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub step_id: String,
    pub summary: String,
    pub reason: String,
}

/// Receives progress and answers approval requests (CLI prompt, extension UI).
pub trait Observer: Send + Sync {
    fn on_event(&self, event: &ExecEvent);
    fn approve(&self, request: ApprovalRequest) -> impl Future<Output = bool> + Send;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub status: RunStatus,
    pub failed_step: Option<String>,
    pub failure: Option<StepFailure>,
    pub downloads: Vec<DownloadInfo>,
}

pub async fn run<D: Driver, O: Observer>(
    workflow: &Workflow,
    params: &ResolvedParams,
    driver: &mut D,
    observer: &O,
    options: &RunOptions,
) -> RunOutcome {
    let mut downloads = Vec::new();
    for (index, step) in workflow.steps.iter().enumerate() {
        let started = Instant::now();
        let emit = |status: StepStatus, error: Option<String>| {
            observer.on_event(&ExecEvent::Step {
                index,
                step_id: step.id.clone(),
                status,
                error,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        };

        if let Some(reason) = approval_reason(step) {
            emit(StepStatus::AwaitingApproval, None);
            let request = ApprovalRequest { step_id: step.id.clone(), summary: step.summary(), reason };
            let approved = tokio::select! {
                approved = observer.approve(request) => approved,
                _ = options.cancel.cancelled() => false,
            };
            if !approved {
                let message = if options.cancel.is_cancelled() { "Run cancelled" } else { "Not approved by the user" };
                emit(StepStatus::Cancelled, Some(message.into()));
                let kind = if options.cancel.is_cancelled() { FailureKind::Cancelled } else { FailureKind::ApprovalDenied };
                return RunOutcome {
                    status: RunStatus::Cancelled,
                    failed_step: Some(step.id.clone()),
                    failure: Some(StepFailure::new(kind, message)),
                    downloads,
                };
            }
        }

        emit(StepStatus::Running, None);
        let result = tokio::select! {
            r = run_with_retry(step, params, driver, options) => r,
            _ = options.cancel.cancelled() => Err(StepFailure::new(FailureKind::Cancelled, "Run cancelled")),
        };
        match result {
            Ok(download) => {
                if let Some(info) = download {
                    observer.on_event(&ExecEvent::Download { step_id: step.id.clone(), info: info.clone() });
                    downloads.push(info);
                }
                emit(StepStatus::Completed, None);
            }
            Err(mut failure) => {
                failure.message = params.scrub(&failure.message);
                let cancelled = failure.kind == FailureKind::Cancelled;
                emit(if cancelled { StepStatus::Cancelled } else { StepStatus::Failed }, Some(failure.message.clone()));
                return RunOutcome {
                    status: if cancelled { RunStatus::Cancelled } else { RunStatus::Failed },
                    failed_step: Some(step.id.clone()),
                    failure: Some(failure),
                    downloads,
                };
            }
        }
    }
    RunOutcome { status: RunStatus::Completed, failed_step: None, failure: None, downloads }
}

async fn run_with_retry<D: Driver>(
    step: &Step,
    params: &ResolvedParams,
    driver: &mut D,
    options: &RunOptions,
) -> Result<Option<DownloadInfo>, StepFailure> {
    let attempts = step.retry.as_ref().map_or(1, |r| r.attempts.max(1));
    let delay = Duration::from_millis(step.retry.as_ref().map_or(0, |r| r.delay_ms));
    let mut attempt = 1;
    loop {
        match run_step(step, params, driver, options).await {
            Ok(v) => return Ok(v),
            Err(f) => {
                // Never repeat something the page may already have acted on
                // (e.g. a submitted form) unless the action is idempotent.
                let safe = !f.dispatched || step.action.is_idempotent();
                let retryable = !matches!(f.kind, FailureKind::InvalidInput | FailureKind::Cancelled);
                if attempt >= attempts || !safe || !retryable {
                    return Err(f);
                }
                attempt += 1;
                tokio::time::sleep(delay).await;
            }
        }
    }
}

fn render(params: &ResolvedParams, s: &str) -> Result<String, StepFailure> {
    params.render(s).map_err(|e| StepFailure::new(FailureKind::InvalidInput, e.to_string()))
}

fn step_timeout(step: &Step, default: Duration) -> Duration {
    step.timeout_ms.map(Duration::from_millis).unwrap_or(default)
}

async fn run_step<D: Driver>(
    step: &Step,
    params: &ResolvedParams,
    driver: &mut D,
    options: &RunOptions,
) -> Result<Option<DownloadInfo>, StepFailure> {
    let timeout = step_timeout(step, options.element_timeout);
    match &step.action {
        Action::Navigate { url } => {
            let url = render(params, url)?;
            if !is_allowed_url(&url) {
                return Err(StepFailure::new(FailureKind::InvalidInput, "only http(s) URLs can be opened"));
            }
            driver
                .navigate(&url, step_timeout(step, options.navigation_timeout))
                .await
                .map_err(|e| StepFailure::from_driver(FailureKind::Navigation, e))?;
        }
        Action::Click { target } => {
            act(driver, target, json!({ "action": "click" }), timeout, options).await?;
        }
        Action::Fill { target, value } => {
            let value = render(params, value)?;
            act(driver, target, json!({ "action": "fill", "value": value }), timeout, options).await?;
        }
        Action::Select { target, value } => {
            let value = render(params, value)?;
            act(driver, target, json!({ "action": "select", "value": value }), timeout, options).await?;
        }
        Action::Check { target, checked } => {
            act(driver, target, json!({ "action": "check", "checked": checked }), timeout, options).await?;
        }
        Action::Press { target, key } => match target {
            Some(t) => act(driver, t, json!({ "action": "press", "key": key }), timeout, options).await?,
            None => {
                let r = driver
                    .perform(json!({ "action": "press", "key": key }), options.navigation_timeout)
                    .await
                    .map_err(|e| StepFailure::from_driver(FailureKind::ActionFailed, e))?;
                check_perform(&r, "press")?;
            }
        },
        Action::Wait { condition } => {
            wait_condition(driver, params, condition, timeout).await?;
        }
        Action::Assert { condition } => {
            wait_condition(driver, params, condition, timeout).await.map_err(|mut f| {
                f.message = format!("Assertion failed: {}", f.message);
                f
            })?;
        }
        Action::Download { target } => {
            driver.expect_download().await.map_err(|e| StepFailure::from_driver(FailureKind::Download, e))?;
            if let Some(t) = target {
                act(driver, t, json!({ "action": "click" }), timeout, options).await?;
            }
            let info = driver
                .wait_download(step_timeout(step, options.download_timeout))
                .await
                .map_err(|e| StepFailure { dispatched: true, ..StepFailure::from_driver(FailureKind::Download, e) })?;
            return Ok(Some(info));
        }
        Action::SwitchTab { url_contains } => {
            let pattern = render(params, url_contains)?;
            driver
                .switch_tab(&pattern, timeout)
                .await
                .map_err(|e| StepFailure::from_driver(FailureKind::Navigation, e))?;
        }
    }
    Ok(None)
}

fn check_perform(result: &Value, action: &str) -> Result<(), StepFailure> {
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let message = result.get("message").and_then(Value::as_str).unwrap_or("action failed");
    Err(StepFailure {
        kind: FailureKind::ActionFailed,
        message: format!("{action} failed: {message}"),
        diagnostics: None,
        dispatched: result.get("dispatched").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Locator outcomes that may resolve themselves if we wait a bit.
const WAITABLE: &[&str] = &["notFound", "hidden", "disabled", "ambiguous", "obscured"];

/// Waits for `target` to be uniquely found, then performs the action. If the
/// element disappears between locating and acting, it waits again.
async fn act<D: Driver>(
    driver: &mut D,
    target: &Target,
    mut args: Value,
    timeout: Duration,
    options: &RunOptions,
) -> Result<(), StepFailure> {
    let action = args["action"].as_str().unwrap_or("action").to_string();
    args["target"] = serde_json::to_value(target).expect("target serializes");
    let deadline = Instant::now() + timeout;
    loop {
        wait_for_element(driver, target, deadline).await?;
        let result = driver
            .perform(args.clone(), options.navigation_timeout)
            .await
            .map_err(|e| StepFailure { dispatched: true, ..StepFailure::from_driver(FailureKind::ActionFailed, e) })?;
        let code = result.get("code").and_then(Value::as_str).unwrap_or_default();
        let dispatched = result.get("dispatched").and_then(Value::as_bool).unwrap_or(true);
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

async fn wait_for_element<D: Driver>(driver: &mut D, target: &Target, deadline: Instant) -> Result<(), StepFailure> {
    let args = json!({ "target": target });
    let mut last: Option<Value> = None;
    let mut last_error: Option<DriverError> = None;
    loop {
        match driver.query("locate", args.clone()).await {
            Ok(r) if r.get("status").and_then(Value::as_str) == Some("found") => return Ok(()),
            Ok(r) => last = Some(r),
            Err(e @ DriverError::Disconnected) => return Err(StepFailure::from_driver(FailureKind::Browser, e)),
            Err(e) => last_error = Some(e),
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    let what = target.describe();
    let (message, diagnostics) = match last {
        Some(r) => {
            let status = r.get("status").and_then(Value::as_str).unwrap_or("notFound");
            let detail = match status {
                "ambiguous" => "several similar elements match",
                "hidden" => "it exists but is not visible",
                "disabled" => "it is disabled",
                _ => "no matching element on the page",
            };
            (format!("Could not find {what}: {detail}"), Some(r))
        }
        None => (
            format!("Could not find {what}: {}", last_error.map_or("page not ready".into(), |e| e.to_string())),
            None,
        ),
    };
    Err(StepFailure { kind: FailureKind::ElementNotFound, message, diagnostics, dispatched: false })
}

async fn wait_condition<D: Driver>(
    driver: &mut D,
    params: &ResolvedParams,
    condition: &Condition,
    timeout: Duration,
) -> Result<(), StepFailure> {
    let mut rendered = condition.clone();
    if let Some(u) = &condition.url_contains {
        rendered.url_contains = Some(render(params, u)?);
    }
    if let Some(t) = &condition.text_contains {
        rendered.text_contains = Some(render(params, t)?);
    }
    let args = json!({ "condition": rendered });
    let deadline = Instant::now() + timeout;
    let mut failed = String::from("condition");
    loop {
        match driver.query("check", args.clone()).await {
            Ok(r) if r.get("ok").and_then(Value::as_bool) == Some(true) => return Ok(()),
            Ok(r) => failed = r.get("failed").and_then(Value::as_str).unwrap_or("condition").to_string(),
            Err(e @ DriverError::Disconnected) => return Err(StepFailure::from_driver(FailureKind::Browser, e)),
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
            return Err(StepFailure::new(FailureKind::ConditionNotMet, format!("{what} within {}s", timeout.as_secs())));
        }
        tokio::time::sleep(POLL).await;
    }
}
