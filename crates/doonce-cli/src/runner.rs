//! Running a stored workflow and recording its history. Shared by the CLI
//! and the browser extension host so both behave identically.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use doonce_core::ResolvedParams;
use doonce_engine::{ApprovalRequest, Driver, ExecEvent, Observer, RunOptions, RunOutcome, StepStatus};
use doonce_storage::{Storage, WorkflowRecord};

pub type SharedStorage = Arc<Mutex<Storage>>;

/// Wraps an observer and writes finished steps to the execution history.
struct HistoryObserver<'a, O> {
    storage: &'a SharedStorage,
    execution_id: i64,
    inner: &'a O,
}

impl<O: Observer> Observer for HistoryObserver<'_, O> {
    fn on_event(&self, event: &ExecEvent) {
        if let ExecEvent::Step { step_id, status, error, duration_ms, .. } = event {
            if matches!(status, StepStatus::Completed | StepStatus::Failed | StepStatus::Cancelled) {
                if let Ok(mut s) = self.storage.lock() {
                    let _ = s.record_step(
                        self.execution_id,
                        step_id,
                        status.as_str(),
                        error.as_deref(),
                        *duration_ms as i64,
                    );
                }
            }
        }
        self.inner.on_event(event);
    }

    fn approve(&self, request: ApprovalRequest) -> impl Future<Output = bool> + Send {
        self.inner.approve(request)
    }
}

/// Validates parameters and opens an execution record.
pub fn prepare(
    storage: &SharedStorage,
    record: &WorkflowRecord,
    provided: &HashMap<String, String>,
    source: &str,
) -> Result<(ResolvedParams, i64)> {
    let params = ResolvedParams::resolve(&record.workflow, provided)?;
    let id = storage.lock().expect("storage lock").start_execution(record.id, record.version, source)?;
    Ok((params, id))
}

pub async fn execute<D: Driver, O: Observer>(
    storage: &SharedStorage,
    execution_id: i64,
    record: &WorkflowRecord,
    params: &ResolvedParams,
    driver: &mut D,
    observer: &O,
    options: &RunOptions,
) -> RunOutcome {
    let history = HistoryObserver { storage, execution_id, inner: observer };
    let outcome = doonce_engine::run(&record.workflow, params, driver, &history, options).await;
    if let Ok(mut s) = storage.lock() {
        let error = outcome.failure.as_ref().map(|f| f.message.as_str());
        let _ = s.finish_execution(execution_id, outcome.status.as_str(), error);
    }
    outcome
}
