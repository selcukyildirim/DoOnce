//! Minimal Model Context Protocol (MCP) server over stdio.
//!
//! The server exposes DoOnce workflows, runs, tasks, artifacts and metrics as
//! MCP tools and forwards calls to the local control-plane API. It is
//! deliberately stateless: credentials are read once from the CLI environment.
//!
//! Protocol: JSON-RPC 2.0, one object per line on stdin/stdout. Logs must never
//! be written to stdout.

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2024-11-05";
const WRITE_TOOLS: &[&str] = &[
    "doonce_create_workflow",
    "doonce_run_workflow",
    "doonce_cancel_run",
    "doonce_create_task",
    "doonce_cancel_task",
    "doonce_resolve_approval",
];

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub read_only: bool,
}

pub async fn serve_stdio(config: McpConfig) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("could not build the MCP HTTP client")?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                let response = error_response(None, -32700, format!("parse error: {error}"));
                write_response(&mut stdout, &response).await?;
                continue;
            }
        };
        if let Some(response) = handle_request(&client, &config, &request).await {
            write_response(&mut stdout, &response).await?;
        }
    }
    Ok(())
}

async fn write_response(stdout: &mut tokio::io::Stdout, response: &Value) -> Result<()> {
    let encoded = serde_json::to_vec(response)?;
    stdout.write_all(&encoded).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

async fn handle_request(
    client: &reqwest::Client,
    config: &McpConfig,
    request: &Value,
) -> Option<Value> {
    let id = request.get("id").cloned()?;
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result: Result<Value> = match method {
        "initialize" => Ok(initialize_result()),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools(config) })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call_tool(client, config, &name, &arguments).await {
                Ok(value) => Ok(json!({
                    "content": [{ "type": "text", "text": pretty(&value) }]
                })),
                Err(error) => Ok(json!({
                    "content": [{ "type": "text", "text": format!("{error:#}") }],
                    "isError": true
                })),
            }
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => {
            return Some(error_response(
                Some(id),
                -32601,
                format!("method not found: {method}"),
            ));
        }
    };
    match result {
        Ok(result) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        Err(error) => Some(error_response(Some(id), -32603, format!("{error:#}"))),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false },
            "prompts": { "listChanged": false }
        },
        "serverInfo": {
            "name": "doonce",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "DoOnce browser automation control plane. Read-only mode hides tools that create, run or cancel work. All calls use the configured DOONCE_API_KEY."
    })
}

fn error_response(id: Option<Value>, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn tools(config: &McpConfig) -> Vec<Value> {
    all_tools()
        .into_iter()
        .filter(|tool| {
            if !config.read_only {
                return true;
            }
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            !WRITE_TOOLS.contains(&name)
        })
        .collect()
}

fn all_tools() -> Vec<Value> {
    vec![
        tool(
            "doonce_health",
            "Check control-plane health.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "doonce_list_workflows",
            "List workflows visible to the API key.",
            json!({ "type": "object", "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 200 } } }),
        ),
        tool(
            "doonce_get_workflow",
            "Get a workflow by slug or id.",
            json!({ "type": "object", "properties": { "reference": { "type": "string" } }, "required": ["reference"] }),
        ),
        tool(
            "doonce_create_workflow",
            "Create a workflow from a workflow v2 definition.",
            json!({ "type": "object", "properties": { "definition": { "type": "object" }, "reason": { "type": "string" } }, "required": ["definition"] }),
        ),
        tool(
            "doonce_run_workflow",
            "Queue a workflow run.",
            json!({ "type": "object", "properties": { "reference": { "type": "string" }, "inputs": { "type": "object" } }, "required": ["reference"] }),
        ),
        tool(
            "doonce_list_runs",
            "List workflow runs, optionally filtered by workflow id.",
            json!({ "type": "object", "properties": { "workflowId": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 200 } } }),
        ),
        tool(
            "doonce_get_run",
            "Get one workflow run and its block history.",
            json!({ "type": "object", "properties": { "runId": { "type": "string" } }, "required": ["runId"] }),
        ),
        tool(
            "doonce_cancel_run",
            "Cancel a queued or running workflow run.",
            json!({ "type": "object", "properties": { "runId": { "type": "string" } }, "required": ["runId"] }),
        ),
        tool(
            "doonce_list_tasks",
            "List AI browser tasks.",
            json!({ "type": "object", "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 200 } } }),
        ),
        tool(
            "doonce_create_task",
            "Queue a natural-language browser task.",
            json!({ "type": "object", "properties": { "prompt": { "type": "string" }, "url": { "type": "string" }, "navigationGoal": { "type": "string" }, "dataSchema": { "type": "object" }, "errorCodes": { "type": "array", "items": { "type": "string" } }, "model": { "type": "string" }, "maxSteps": { "type": "integer", "minimum": 1, "maximum": 100 } }, "required": ["prompt"] }),
        ),
        tool(
            "doonce_get_task",
            "Get one AI task.",
            json!({ "type": "object", "properties": { "taskId": { "type": "string" } }, "required": ["taskId"] }),
        ),
        tool(
            "doonce_cancel_task",
            "Cancel a queued or running AI task.",
            json!({ "type": "object", "properties": { "taskId": { "type": "string" } }, "required": ["taskId"] }),
        ),
        tool(
            "doonce_list_artifacts",
            "List artifacts for a run or task.",
            json!({ "type": "object", "properties": { "runId": { "type": "string" }, "taskId": { "type": "string" } } }),
        ),
        tool(
            "doonce_list_approvals",
            "List pending or resolved approvals.",
            json!({ "type": "object", "properties": { "status": { "type": "string" }, "runId": { "type": "string" }, "taskId": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 200 } } }),
        ),
        tool(
            "doonce_resolve_approval",
            "Approve or deny a paused step.",
            json!({ "type": "object", "properties": { "approvalId": { "type": "string" }, "decision": { "type": "string", "enum": ["approve", "deny"] } }, "required": ["approvalId", "decision"] }),
        ),
        tool(
            "doonce_metrics",
            "Get tenant metrics (queue depth, success rate, latency, artifacts).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

async fn call_tool(
    client: &reqwest::Client,
    config: &McpConfig,
    name: &str,
    arguments: &Value,
) -> Result<Value> {
    if config.read_only && WRITE_TOOLS.contains(&name) {
        bail!("tool `{name}` is disabled in read-only mode");
    }
    match name {
        "doonce_health" => api_request(client, config, Method::GET, "/health", None).await,
        "doonce_list_workflows" => {
            let limit = arg_i64(arguments, "limit").unwrap_or(50);
            api_request(
                client,
                config,
                Method::GET,
                &format!("/workflows?limit={limit}"),
                None,
            )
            .await
        }
        "doonce_get_workflow" => {
            let reference = arg_str(arguments, "reference")?;
            api_request(
                client,
                config,
                Method::GET,
                &format!("/workflows/{reference}"),
                None,
            )
            .await
        }
        "doonce_create_workflow" => {
            let definition = arguments
                .get("definition")
                .cloned()
                .ok_or_else(|| anyhow!("argument `definition` is required"))?;
            let mut body = json!({ "definition": definition });
            if let Some(reason) = arg_opt_str(arguments, "reason") {
                body["reason"] = Value::String(reason);
            }
            api_request(client, config, Method::POST, "/workflows", Some(body)).await
        }
        "doonce_run_workflow" => {
            let reference = arg_str(arguments, "reference")?;
            let inputs = arguments
                .get("inputs")
                .cloned()
                .unwrap_or_else(|| json!({}));
            api_request(
                client,
                config,
                Method::POST,
                &format!("/workflows/{reference}/runs"),
                Some(json!({ "inputs": inputs })),
            )
            .await
        }
        "doonce_list_runs" => {
            let limit = arg_i64(arguments, "limit").unwrap_or(50);
            let workflow_id = arg_opt_str(arguments, "workflowId");
            let path = match workflow_id {
                Some(workflow_id) => format!("/runs?limit={limit}&workflow_id={workflow_id}"),
                None => format!("/runs?limit={limit}"),
            };
            api_request(client, config, Method::GET, &path, None).await
        }
        "doonce_get_run" => {
            let run_id = arg_str(arguments, "runId")?;
            api_request(
                client,
                config,
                Method::GET,
                &format!("/runs/{run_id}"),
                None,
            )
            .await
        }
        "doonce_cancel_run" => {
            let run_id = arg_str(arguments, "runId")?;
            api_request(
                client,
                config,
                Method::POST,
                &format!("/runs/{run_id}/cancel"),
                None,
            )
            .await
        }
        "doonce_list_tasks" => {
            let limit = arg_i64(arguments, "limit").unwrap_or(50);
            api_request(
                client,
                config,
                Method::GET,
                &format!("/tasks?limit={limit}"),
                None,
            )
            .await
        }
        "doonce_create_task" => {
            let prompt = arg_str(arguments, "prompt")?;
            let mut body = json!({ "prompt": prompt });
            for (argument, field) in [
                ("url", "url"),
                ("navigationGoal", "navigationGoal"),
                ("model", "model"),
            ] {
                if let Some(value) = arg_opt_str(arguments, argument) {
                    body[field] = Value::String(value);
                }
            }
            if let Some(value) = arguments.get("dataSchema") {
                body["dataSchema"] = value.clone();
            }
            if let Some(value) = arguments.get("errorCodes") {
                body["errorCodes"] = value.clone();
            }
            if let Some(value) = arguments.get("maxSteps") {
                body["maxSteps"] = value.clone();
            }
            api_request(client, config, Method::POST, "/tasks", Some(body)).await
        }
        "doonce_get_task" => {
            let task_id = arg_str(arguments, "taskId")?;
            api_request(
                client,
                config,
                Method::GET,
                &format!("/tasks/{task_id}"),
                None,
            )
            .await
        }
        "doonce_cancel_task" => {
            let task_id = arg_str(arguments, "taskId")?;
            api_request(
                client,
                config,
                Method::POST,
                &format!("/tasks/{task_id}/cancel"),
                None,
            )
            .await
        }
        "doonce_list_artifacts" => {
            let run_id = arg_opt_str(arguments, "runId");
            let task_id = arg_opt_str(arguments, "taskId");
            let path = match (run_id, task_id) {
                (Some(run_id), _) => format!("/runs/{run_id}/artifacts"),
                (_, Some(task_id)) => format!("/tasks/{task_id}/artifacts"),
                _ => bail!("provide `runId` or `taskId`"),
            };
            api_request(client, config, Method::GET, &path, None).await
        }
        "doonce_list_approvals" => {
            let limit = arg_i64(arguments, "limit").unwrap_or(50);
            let mut query = format!("?limit={limit}");
            for (argument, field) in [
                ("status", "status"),
                ("runId", "run_id"),
                ("taskId", "task_id"),
            ] {
                if let Some(value) = arg_opt_str(arguments, argument) {
                    query.push_str(&format!("&{field}={value}"));
                }
            }
            api_request(
                client,
                config,
                Method::GET,
                &format!("/approvals{query}"),
                None,
            )
            .await
        }
        "doonce_resolve_approval" => {
            let approval_id = arg_str(arguments, "approvalId")?;
            let decision = arg_str(arguments, "decision")?;
            api_request(
                client,
                config,
                Method::POST,
                &format!("/approvals/{approval_id}"),
                Some(json!({ "decision": decision })),
            )
            .await
        }
        "doonce_metrics" => api_request(client, config, Method::GET, "/metrics", None).await,
        _ => bail!("unknown tool `{name}`"),
    }
}

async fn api_request(
    client: &reqwest::Client,
    config: &McpConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{}", config.base_url.trim_end_matches('/'), path);
    let mut request = client
        .request(method, &url)
        .header("accept", "application/json");
    if let Some(api_key) = &config.api_key {
        request = request.header("x-api-key", api_key);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("request to {url} failed"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("could not read API response")?;
    if !status.is_success() {
        bail!("DoOnce API returned {status}: {}", truncate(&text, 2000));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .with_context(|| format!("invalid JSON from API: {}", truncate(&text, 500)))
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn arg_str(arguments: &Value, name: &str) -> Result<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("argument `{name}` is required"))
}

fn arg_opt_str(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn arg_i64(arguments: &Value, name: &str) -> Option<i64> {
    arguments
        .get(name)
        .and_then(Value::as_i64)
        .map(|value| value.clamp(1, 200))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(read_only: bool) -> McpConfig {
        McpConfig {
            base_url: "http://127.0.0.1:1/api/v1".into(),
            api_key: None,
            read_only,
        }
    }

    async fn request(config: &McpConfig, value: Value) -> Value {
        let client = reqwest::Client::new();
        handle_request(&client, config, &value)
            .await
            .expect("request has an id")
    }

    #[tokio::test]
    async fn initialize_reports_tools_capability() {
        let response = request(
            &config(false),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        )
        .await;
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], "doonce");
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
    }

    #[tokio::test]
    async fn tools_list_filters_write_tools_in_read_only_mode() {
        let response = request(
            &config(true),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await;
        let names: Vec<_> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"doonce_get_run"));
        assert!(!names.contains(&"doonce_run_workflow"));
        assert!(!names.contains(&"doonce_create_task"));
    }

    #[tokio::test]
    async fn notifications_do_not_get_a_response() {
        let client = reqwest::Client::new();
        let response = handle_request(
            &client,
            &config(false),
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )
        .await;
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn unknown_method_returns_json_rpc_error() {
        let response = request(
            &config(false),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "does/not/exist" }),
        )
        .await;
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn read_only_rejects_write_tool_calls() {
        let client = reqwest::Client::new();
        let response = handle_request(
            &client,
            &config(true),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "doonce_run_workflow", "arguments": { "reference": "x" } }
            }),
        )
        .await
        .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("read-only")
        );
    }
}
