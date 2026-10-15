//! Chrome Native Messaging host. Chrome starts `doonce` with the calling
//! extension's origin as the first argument and exchanges length-prefixed
//! JSON messages over stdin/stdout.
//!
//! Extension → host: `{ id: number, method, params }`, answered with
//! `{ id, result }` or `{ id, error: { message } }`.
//! Host → extension: events `{ event, ... }` and driver calls
//! `{ id: "h<n>", driver, params }`, answered like requests.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use doonce_core::recording::{RecordedEvent, compile};
use doonce_core::risk::approval_reason;
use doonce_core::sanitize::sanitize_workflow;
use doonce_core::validate::parse_workflow_value;
use doonce_engine::{ApprovalRequest, DownloadInfo, Driver, DriverError, ExecEvent, Observer, RunOptions};
use doonce_storage::{Storage, WorkflowRecord};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::runner::{self, SharedStorage};

const MAX_INCOMING: usize = 64 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<std::result::Result<Value, (String, bool)>>>>>;

#[derive(Clone)]
struct Link {
    out: mpsc::UnboundedSender<Value>,
    pending: Pending,
    next: Arc<AtomicU64>,
}

impl Link {
    fn send(&self, msg: Value) {
        let _ = self.out.send(msg);
    }

    async fn call(&self, op: &str, params: Value, timeout: Duration) -> Result<Value, DriverError> {
        let id = format!("h{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        self.send(json!({ "id": id, "driver": op, "params": params }));
        // The extension enforces the operation timeout; allow some slack.
        match tokio::time::timeout(timeout + Duration::from_secs(5), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err((message, true)))) => Err(DriverError::Transient(message)),
            Ok(Ok(Err((message, false)))) => Err(DriverError::Browser(message)),
            Ok(Err(_)) => Err(DriverError::Disconnected),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(DriverError::Timeout(format!("the browser did not answer ({op})")))
            }
        }
    }
}

struct ActiveRun {
    execution_id: i64,
    cancel: CancellationToken,
    approval: Option<oneshot::Sender<bool>>,
}

#[derive(Clone)]
struct Host {
    link: Link,
    storage: SharedStorage,
    active: Arc<Mutex<Option<ActiveRun>>>,
}

pub async fn run(origin: &str) -> Result<()> {
    eprintln!("doonce native host started for {origin}");
    let storage = Storage::open_default()?;
    let storage = Arc::new(Mutex::new(storage));
    let _ = storage.lock().unwrap().mark_interrupted(0);

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(msg) = out_rx.recv().await {
            let bytes = serde_json::to_vec(&msg).expect("json");
            if stdout.write_all(&(bytes.len() as u32).to_ne_bytes()).await.is_err()
                || stdout.write_all(&bytes).await.is_err()
                || stdout.flush().await.is_err()
            {
                break;
            }
        }
    });

    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Value>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        loop {
            let mut len = [0u8; 4];
            if stdin.read_exact(&mut len).is_err() {
                break;
            }
            let len = u32::from_ne_bytes(len) as usize;
            if len > MAX_INCOMING {
                break;
            }
            let mut buf = vec![0u8; len];
            if stdin.read_exact(&mut buf).is_err() {
                break;
            }
            match serde_json::from_slice(&buf) {
                Ok(v) => {
                    if in_tx.send(v).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("doonce host: ignoring malformed message: {e}"),
            }
        }
    });

    let host = Host {
        link: Link { out: out_tx, pending: Arc::default(), next: Arc::new(AtomicU64::new(1)) },
        storage,
        active: Arc::default(),
    };

    while let Some(msg) = in_rx.recv().await {
        // Answers to our driver calls.
        if let Some(id) = msg.get("id").and_then(Value::as_str) {
            if let Some(tx) = host.link.pending.lock().unwrap().remove(id) {
                let result = match msg.get("error") {
                    Some(e) => Err((
                        e.get("message").and_then(Value::as_str).unwrap_or("browser error").to_string(),
                        e.get("transient").and_then(Value::as_bool).unwrap_or(false),
                    )),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
            continue;
        }
        let Some(method) = msg.get("method").and_then(Value::as_str).map(str::to_string) else { continue };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let host = host.clone();
        tokio::spawn(async move {
            let reply = match host.handle(&method, params).await {
                Ok(result) => json!({ "id": id, "result": result }),
                Err(e) => json!({ "id": id, "error": { "message": format!("{e:#}") } }),
            };
            host.link.send(reply);
        });
    }

    // Chrome closed the port: stop whatever is running.
    if let Some(run) = host.active.lock().unwrap().as_ref() {
        run.cancel.cancel();
    }
    Ok(())
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params.get(key).and_then(Value::as_str).ok_or_else(|| anyhow!("missing \"{key}\""))
}

fn workflow_view(record: &WorkflowRecord) -> Value {
    let steps: Vec<Value> = record
        .workflow
        .steps
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "type": s.action.kind(),
                "summary": s.summary(),
                "approval": approval_reason(s),
            })
        })
        .collect();
    json!({ "record": record, "steps": steps })
}

impl Host {
    fn store<T>(&self, f: impl FnOnce(&mut Storage) -> doonce_storage::Result<T>) -> Result<T> {
        let mut s = self.storage.lock().map_err(|_| anyhow!("storage unavailable"))?;
        Ok(f(&mut s)?)
    }

    async fn handle(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "hello" => Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "dataDir": doonce_storage::data_dir(),
            })),
            "listWorkflows" => Ok(serde_json::to_value(self.store(|s| s.list_workflows())?)?),
            "getWorkflow" => {
                let reference = str_param(&params, "ref")?.to_string();
                Ok(workflow_view(&self.store(|s| s.get_workflow(&reference))?))
            }
            "compileRecording" => {
                let events: Vec<RecordedEvent> =
                    serde_json::from_value(params.get("events").cloned().unwrap_or_default())
                        .context("invalid recording")?;
                let name = params.get("name").and_then(Value::as_str);
                let compiled = compile(name, &events);
                let steps: Vec<Value> = compiled
                    .workflow
                    .steps
                    .iter()
                    .map(|s| json!({ "id": s.id, "type": s.action.kind(), "summary": s.summary() }))
                    .collect();
                Ok(json!({ "workflow": compiled.workflow, "warnings": compiled.warnings, "steps": steps }))
            }
            "saveWorkflow" => {
                let mut workflow = parse_workflow_value(params.get("workflow").cloned().unwrap_or_default())
                    .map_err(|issues| {
                        anyhow!("{}", issues.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))
                    })?;
                sanitize_workflow(&mut workflow);
                let reason = params.get("reason").and_then(Value::as_str).unwrap_or("Saved from browser").to_string();
                let record = match params.get("ref").and_then(Value::as_str) {
                    Some(r) => {
                        let r = r.to_string();
                        self.store(|s| s.update_workflow(&r, &workflow, &reason))?
                    }
                    None => self.store(|s| s.create_workflow(&workflow, &reason))?,
                };
                Ok(workflow_view(&record))
            }
            "deleteWorkflow" => {
                let reference = str_param(&params, "ref")?.to_string();
                self.store(|s| s.delete_workflow(&reference))?;
                Ok(Value::Null)
            }
            "history" => {
                let reference = str_param(&params, "ref")?.to_string();
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                Ok(serde_json::to_value(self.store(|s| s.history(&reference, limit))?)?)
            }
            "run" => self.start_run(&params),
            "approve" => {
                let approved = params.get("approved").and_then(Value::as_bool).unwrap_or(false);
                let mut active = self.active.lock().unwrap();
                let run = active.as_mut().ok_or_else(|| anyhow!("nothing is running"))?;
                let tx = run.approval.take().ok_or_else(|| anyhow!("no step is waiting for approval"))?;
                let _ = tx.send(approved);
                Ok(Value::Null)
            }
            "cancel" => {
                if let Some(run) = self.active.lock().unwrap().as_ref() {
                    run.cancel.cancel();
                }
                Ok(Value::Null)
            }
            other => Err(anyhow!("unknown method \"{other}\"")),
        }
    }

    fn start_run(&self, params: &Value) -> Result<Value> {
        let reference = str_param(params, "ref")?.to_string();
        let provided: HashMap<String, String> = params
            .get("params")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
            .unwrap_or_default();
        let record = self.store(|s| s.get_workflow(&reference))?;

        let cancel = CancellationToken::new();
        let execution_id = {
            let mut active = self.active.lock().unwrap();
            if active.is_some() {
                return Err(anyhow!("another automation is already running"));
            }
            let (resolved, id) = runner::prepare(&self.storage, &record, &provided, "extension")?;
            *active = Some(ActiveRun { execution_id: id, cancel: cancel.clone(), approval: None });
            let host = self.clone();
            let record = record.clone();
            tokio::spawn(async move {
                let mut driver = ExtensionDriver { link: host.link.clone(), tab_id: None };
                let observer = HostObserver { host: host.clone(), execution_id: id };
                let options = RunOptions { cancel, ..RunOptions::default() };
                let outcome =
                    runner::execute(&host.storage, id, &record, &resolved, &mut driver, &observer, &options).await;
                host.active.lock().unwrap().take();
                host.link.send(json!({ "event": "finished", "executionId": id, "outcome": outcome }));
            });
            id
        };
        let steps: Vec<Value> =
            record.workflow.steps.iter().map(|s| json!({ "id": s.id, "summary": s.summary() })).collect();
        Ok(json!({ "executionId": execution_id, "steps": steps, "name": record.workflow.name, "slug": record.slug }))
    }
}

struct HostObserver {
    host: Host,
    execution_id: i64,
}

impl Observer for HostObserver {
    fn on_event(&self, event: &ExecEvent) {
        let mut msg = serde_json::to_value(event).expect("event serializes");
        msg["executionId"] = json!(self.execution_id);
        msg["kind"] = msg["event"].take();
        msg["event"] = json!("progress");
        self.host.link.send(msg);
    }

    async fn approve(&self, request: ApprovalRequest) -> bool {
        let (tx, rx) = oneshot::channel();
        {
            let mut active = self.host.active.lock().unwrap();
            match active.as_mut() {
                Some(run) if run.execution_id == self.execution_id => run.approval = Some(tx),
                _ => return false,
            }
        }
        self.host.link.send(json!({
            "event": "approval",
            "executionId": self.execution_id,
            "stepId": request.step_id,
            "summary": request.summary,
            "reason": request.reason,
        }));
        rx.await.unwrap_or(false)
    }
}

/// Drives a tab of the user's own Chrome through the extension.
struct ExtensionDriver {
    link: Link,
    tab_id: Option<i64>,
}

impl ExtensionDriver {
    fn tab(&self) -> Result<i64, DriverError> {
        self.tab_id.ok_or_else(|| DriverError::Browser("no tab is open yet; workflows must start by opening a page".into()))
    }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

impl Driver for ExtensionDriver {
    async fn navigate(&mut self, url: &str, timeout: Duration) -> Result<(), DriverError> {
        let r = self
            .link
            .call("navigate", json!({ "tabId": self.tab_id, "url": url, "timeoutMs": ms(timeout) }), timeout)
            .await?;
        self.tab_id = r.get("tabId").and_then(Value::as_i64).or(self.tab_id);
        Ok(())
    }

    async fn query(&mut self, method: &str, args: Value) -> Result<Value, DriverError> {
        let tab = self.tab()?;
        self.link
            .call("call", json!({ "tabId": tab, "method": method, "args": args }), Duration::from_secs(15))
            .await
    }

    async fn perform(&mut self, args: Value, timeout: Duration) -> Result<Value, DriverError> {
        let tab = self.tab()?;
        self.link.call("perform", json!({ "tabId": tab, "args": args, "timeoutMs": ms(timeout) }), timeout).await
    }

    async fn expect_download(&mut self) -> Result<(), DriverError> {
        self.link.call("expectDownload", json!({}), Duration::from_secs(10)).await.map(|_| ())
    }

    async fn wait_download(&mut self, timeout: Duration) -> Result<DownloadInfo, DriverError> {
        let r = self.link.call("waitDownload", json!({ "timeoutMs": ms(timeout) }), timeout).await?;
        serde_json::from_value(r).map_err(|e| DriverError::Browser(format!("bad download info: {e}")))
    }

    async fn switch_tab(&mut self, url_contains: &str, timeout: Duration) -> Result<(), DriverError> {
        let r = self
            .link
            .call(
                "switchTab",
                json!({ "fromTabId": self.tab_id, "urlContains": url_contains, "timeoutMs": ms(timeout) }),
                timeout,
            )
            .await?;
        self.tab_id = r.get("tabId").and_then(Value::as_i64).or(self.tab_id);
        Ok(())
    }
}
