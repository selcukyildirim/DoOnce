//! Outbound-only hybrid data-plane connector.
//!
//! The connector polls a control plane for jobs, forwards each job to a local
//! DoOnce data plane (API + worker), waits for the result, and reports it back.
//! It never opens an inbound port and keeps browser data, credentials and
//! artifacts inside the data-plane network.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    pub control_url: String,
    pub token: String,
    pub local_url: String,
    pub local_api_key: Option<String>,
    pub poll_interval: Duration,
    pub lease_secs: i64,
    pub auto_approve: bool,
    pub max_job_wait: Duration,
}

impl ConnectorConfig {
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("DOONCE_CONNECTOR_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("set DOONCE_CONNECTOR_TOKEN to the connector token"))?;
        let control_url = std::env::var("DOONCE_CONTROL_URL")
            .ok()
            .unwrap_or_else(|| "http://127.0.0.1:8080".into());
        let local_url = std::env::var("DOONCE_CONNECTOR_LOCAL_URL")
            .ok()
            .unwrap_or_else(|| "http://127.0.0.1:8080".into());
        Ok(Self {
            control_url: api_base(&control_url),
            token,
            local_url: api_base(&local_url),
            local_api_key: std::env::var("DOONCE_CONNECTOR_LOCAL_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            poll_interval: Duration::from_secs(
                env_u64("DOONCE_CONNECTOR_POLL_SECS").unwrap_or(2).max(1),
            ),
            lease_secs: env_u64("DOONCE_CONNECTOR_LEASE_SECS")
                .unwrap_or(900)
                .max(30) as i64,
            auto_approve: env_bool("DOONCE_CONNECTOR_AUTO_APPROVE", false),
            max_job_wait: Duration::from_secs(
                env_u64("DOONCE_CONNECTOR_JOB_TIMEOUT_SECS")
                    .unwrap_or(3600)
                    .max(30),
            ),
        })
    }
}

pub async fn serve(config: ConnectorConfig) -> Result<()> {
    if config.token.trim().is_empty() {
        bail!("connector token must not be empty");
    }
    if doonce_core::DeploymentMode::from_env().is_air_gapped()
        && !is_loopback_url(&config.control_url)
    {
        bail!("air-gapped mode requires a loopback control-plane URL");
    }
    let control = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("could not build the control-plane client")?;
    let local = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("could not build the local data-plane client")?;

    println!(
        "DoOnce connector started: control={} local={} poll={}s lease={}s",
        config.control_url,
        config.local_url,
        config.poll_interval.as_secs(),
        config.lease_secs
    );

    let stop = Arc::new(AtomicBool::new(false));
    let heartbeat = {
        let control = control.clone();
        let config = config.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match control_heartbeat(&control, &config, None).await {
                    Ok(pending) => {
                        tracing::debug!(pending, "connector heartbeat");
                    }
                    Err(error) => tracing::warn!(%error, "connector heartbeat failed"),
                }
                sleep(Duration::from_secs(20)).await;
            }
        })
    };

    let result = run_loop(&control, &local, &config).await;
    stop.store(true, Ordering::Relaxed);
    heartbeat.abort();
    result
}

async fn run_loop(control: &Client, local: &Client, config: &ConnectorConfig) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("DoOnce connector stopped");
                return Ok(());
            }
            _ = sleep(config.poll_interval) => {}
        }
        let job = match claim_job(control, config).await {
            Ok(job) => job,
            Err(error) => {
                tracing::warn!(%error, "could not claim a connector job");
                continue;
            }
        };
        let Some(job) = job else { continue };
        let job_id = job
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        println!("Connector job {job_id}: started");
        let (status, result, error) = match execute_job(local, config, &job).await {
            Ok(result) => {
                let status = result
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed")
                    .to_string();
                (status, Some(result), None)
            }
            Err(error) => {
                tracing::error!(job = %job_id, %error, "connector job failed");
                ("failed".to_string(), None, Some(format!("{error:#}")))
            }
        };
        match report_complete(
            control,
            config,
            &job_id,
            &status,
            result.as_ref(),
            error.as_deref(),
        )
        .await
        {
            Ok(()) => println!("Connector job {job_id}: {status}"),
            Err(error) => {
                tracing::error!(job = %job_id, %error, "could not report connector job result");
            }
        }
    }
}

async fn execute_job(local: &Client, config: &ConnectorConfig, job: &Value) -> Result<Value> {
    let payload = job
        .get("payload")
        .ok_or_else(|| anyhow!("connector job has no payload"))?;
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "workflow" => execute_workflow(local, config, job, payload).await,
        "task" => execute_task(local, config, job, payload).await,
        other => bail!("unsupported connector job kind `{other}`"),
    }
}

async fn execute_workflow(
    local: &Client,
    config: &ConnectorConfig,
    job: &Value,
    payload: &Value,
) -> Result<Value> {
    let definition = payload
        .get("definition")
        .cloned()
        .ok_or_else(|| anyhow!("workflow job has no definition"))?;
    let slug = payload
        .get("workflowSlug")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            definition
                .get("name")
                .and_then(Value::as_str)
                .map(doonce_core::slugify)
        })
        .ok_or_else(|| anyhow!("workflow job has no slug or name"))?;

    let existing = local_get(local, config, &format!("/workflows/{slug}")).await;
    if existing.is_ok() {
        local_request(
            local,
            config,
            Method::PUT,
            &format!("/workflows/{slug}"),
            Some(json!({ "definition": definition, "reason": "Synced by hybrid connector" })),
        )
        .await?;
    } else {
        let created = local_request(
            local,
            config,
            Method::POST,
            "/workflows",
            Some(json!({ "definition": definition, "reason": "Synced by hybrid connector" })),
        )
        .await?;
        if created
            .pointer("/workflow/slug")
            .and_then(Value::as_str)
            .is_none()
        {
            bail!("local data plane did not return a workflow slug");
        }
    }

    let inputs = payload.get("inputs").cloned().unwrap_or_else(|| json!({}));
    let run = local_request(
        local,
        config,
        Method::POST,
        &format!("/workflows/{slug}/runs"),
        Some(json!({ "inputs": inputs })),
    )
    .await?;
    let run_id = run
        .pointer("/run/id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("local data plane did not return a run id"))?
        .to_string();

    let (status, run) = wait_for_job(local, config, job, "run", &run_id).await?;
    let artifacts = local_get(local, config, &format!("/runs/{run_id}/artifacts"))
        .await
        .ok()
        .and_then(|value| value.get("artifacts").cloned())
        .map(|artifacts| summarize_artifacts(&artifacts))
        .unwrap_or_else(|| json!([]));
    Ok(json!({
        "status": status,
        "outputs": run.get("run").and_then(|run| run.get("outputs")).cloned(),
        "error": run.get("run").and_then(|run| run.get("error")).cloned(),
        "artifacts": artifacts,
        "localRunId": run_id,
    }))
}

async fn execute_task(
    local: &Client,
    config: &ConnectorConfig,
    job: &Value,
    payload: &Value,
) -> Result<Value> {
    let mut body = json!({
        "prompt": payload.get("prompt").cloned().unwrap_or(Value::Null)
    });
    for field in [
        "url",
        "navigationGoal",
        "dataSchema",
        "errorCodes",
        "model",
        "maxSteps",
    ] {
        if let Some(value) = payload.get(field)
            && !value.is_null()
        {
            body[field] = value.clone();
        }
    }
    let created = local_request(local, config, Method::POST, "/tasks", Some(body)).await?;
    let task_id = created
        .pointer("/task/id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("local data plane did not return a task id"))?
        .to_string();
    let (status, task) = wait_for_job(local, config, job, "task", &task_id).await?;
    let artifacts = local_get(local, config, &format!("/tasks/{task_id}/artifacts"))
        .await
        .ok()
        .and_then(|value| value.get("artifacts").cloned())
        .map(|artifacts| summarize_artifacts(&artifacts))
        .unwrap_or_else(|| json!([]));
    Ok(json!({
        "status": status,
        "output": task.get("task").and_then(|task| task.get("output")).cloned(),
        "error": task.get("task").and_then(|task| task.get("failureReason")).cloned(),
        "artifacts": artifacts,
        "localTaskId": task_id,
    }))
}

async fn wait_for_job(
    local: &Client,
    config: &ConnectorConfig,
    job: &Value,
    kind: &str,
    id: &str,
) -> Result<(String, Value)> {
    let deadline = Instant::now() + config.max_job_wait;
    let mut last_extend = Instant::now();
    loop {
        let value = if kind == "run" {
            local_get(local, config, &format!("/runs/{id}")).await?
        } else {
            local_get(local, config, &format!("/tasks/{id}")).await?
        };
        let status = value
            .get(kind)
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match status.as_str() {
            "completed" | "failed" | "cancelled" => return Ok((status, value)),
            "awaiting_approval" if config.auto_approve => {
                approve_pending(local, config, kind, id).await?;
            }
            "awaiting_approval" => {
                bail!(
                    "{kind} {id} is awaiting approval; run the connector with --auto-approve or resolve it locally"
                )
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            bail!(
                "{kind} {id} did not finish within {:?}",
                config.max_job_wait
            );
        }
        if last_extend.elapsed() >= Duration::from_secs(300) {
            if let Some(job_id) = job.get("id").and_then(Value::as_str) {
                let _ = control_request(
                    local,
                    config,
                    Method::POST,
                    &format!("/connector/jobs/{job_id}/heartbeat"),
                    Some(json!({ "leaseSecs": config.lease_secs })),
                )
                .await;
            }
            last_extend = Instant::now();
        }
        sleep(Duration::from_secs(2)).await;
    }
}

async fn approve_pending(
    local: &Client,
    config: &ConnectorConfig,
    kind: &str,
    id: &str,
) -> Result<()> {
    let query = if kind == "run" {
        format!("/approvals?status=pending&run_id={id}")
    } else {
        format!("/approvals?status=pending&task_id={id}")
    };
    let value = local_get(local, config, &query).await?;
    let Some(approvals) = value.get("approvals").and_then(Value::as_array) else {
        return Ok(());
    };
    for approval in approvals {
        if let Some(approval_id) = approval.get("id").and_then(Value::as_str) {
            local_request(
                local,
                config,
                Method::POST,
                &format!("/approvals/{approval_id}"),
                Some(json!({ "decision": "approve" })),
            )
            .await?;
            println!("Approved {kind} approval {approval_id}");
        }
    }
    Ok(())
}

fn summarize_artifacts(artifacts: &Value) -> Value {
    let Some(items) = artifacts.as_array() else {
        return json!([]);
    };
    Value::Array(
        items
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact.get("id").cloned().unwrap_or(Value::Null),
                    "kind": artifact.get("kind").cloned().unwrap_or(Value::Null),
                    "filename": artifact.get("filename").cloned().unwrap_or(Value::Null),
                    "contentType": artifact.get("contentType").cloned().unwrap_or(Value::Null),
                    "size": artifact.get("size").cloned().unwrap_or(Value::Null),
                })
            })
            .collect(),
    )
}

async fn control_heartbeat(
    control: &Client,
    config: &ConnectorConfig,
    version: Option<&str>,
) -> Result<i64> {
    let value = control_request(
        control,
        config,
        Method::POST,
        "/connector/heartbeat",
        Some(json!({ "version": version.unwrap_or(env!("CARGO_PKG_VERSION")) })),
    )
    .await?;
    Ok(value
        .get("pendingJobs")
        .and_then(Value::as_i64)
        .unwrap_or(0))
}

async fn claim_job(control: &Client, config: &ConnectorConfig) -> Result<Option<Value>> {
    let value = control_request(
        control,
        config,
        Method::POST,
        "/connector/jobs/claim",
        Some(json!({ "leaseSecs": config.lease_secs })),
    )
    .await?;
    Ok(value.get("job").cloned().filter(|value| !value.is_null()))
}

async fn report_complete(
    control: &Client,
    config: &ConnectorConfig,
    job_id: &str,
    status: &str,
    result: Option<&Value>,
    error: Option<&str>,
) -> Result<()> {
    let body = json!({ "status": status, "result": result, "error": error });
    let mut last_error = None;
    for attempt in 0..3 {
        match control_request(
            control,
            config,
            Method::POST,
            &format!("/connector/jobs/{job_id}/complete"),
            Some(body.clone()),
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("could not report connector job")))
}

async fn local_get(client: &Client, config: &ConnectorConfig, path: &str) -> Result<Value> {
    local_request(client, config, Method::GET, path, None).await
}

async fn local_request(
    client: &Client,
    config: &ConnectorConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{}", config.local_url, path);
    let mut request = client
        .request(method, &url)
        .header("accept", "application/json");
    if let Some(api_key) = &config.local_api_key {
        request = request.header("x-api-key", api_key);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("local data-plane request to {url} failed"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("could not read local response")?;
    if !status.is_success() {
        bail!(
            "local data plane returned {status}: {}",
            truncate(&text, 2000)
        );
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).with_context(|| {
        format!(
            "invalid JSON from local data plane: {}",
            truncate(&text, 500)
        )
    })
}

async fn control_request(
    client: &Client,
    config: &ConnectorConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{}", config.control_url, path);
    let mut request = client
        .request(method, &url)
        .header("accept", "application/json")
        .header("x-connector-token", &config.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("control-plane request to {url} failed"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("could not read control response")?;
    if !status.is_success() {
        bail!("control plane returned {status}: {}", truncate(&text, 2000));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .with_context(|| format!("invalid JSON from control plane: {}", truncate(&text, 500)))
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

pub fn api_base(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.ends_with("/api/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/api/v1")
    }
}

fn is_loopback_url(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.starts_with("http://127.0.0.1")
        || value.starts_with("http://localhost")
        || value.starts_with("http://[::1]")
        || value.starts_with("https://127.0.0.1")
        || value.starts_with("https://localhost")
        || value.starts_with("https://[::1]")
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_api_base() {
        assert_eq!(
            api_base("http://localhost:8080"),
            "http://localhost:8080/api/v1"
        );
        assert_eq!(
            api_base("http://localhost:8080/api/v1/"),
            "http://localhost:8080/api/v1"
        );
    }

    #[test]
    fn loopback_check_is_conservative() {
        assert!(is_loopback_url("http://127.0.0.1:8080"));
        assert!(is_loopback_url("https://localhost"));
        assert!(!is_loopback_url("https://doonce.example.com"));
    }

    #[test]
    fn artifact_summary_drops_storage_keys() {
        let value = json!([{ "id": "a1", "kind": "screenshot", "filename": "x.png", "storageKey": "secret" }]);
        let summary = summarize_artifacts(&value);
        assert_eq!(summary[0]["id"], "a1");
        assert!(summary[0].get("storageKey").is_none());
    }
}
