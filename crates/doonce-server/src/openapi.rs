use serde_json::{Value, json};

pub fn spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "DoOnce Enterprise API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Control-plane API for deterministic and AI browser automation."
        },
        "servers": [ { "url": "/api/v1" } ],
        "components": {
            "securitySchemes": {
                "ApiKey": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "x-api-key"
                },
                "Bearer": {
                    "type": "http",
                    "scheme": "bearer"
                }
            },
            "schemas": {
                "Problem": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string" },
                        "title": { "type": "string" },
                        "status": { "type": "integer" },
                        "detail": { "type": "string" }
                    }
                }
            }
        },
        "security": [ { "ApiKey": [] }, { "Bearer": [] } ],
        "paths": {
            "/health": { "get": { "summary": "Liveness", "security": [], "responses": { "200": { "description": "OK" } } } },
            "/version": { "get": { "summary": "Build version", "security": [], "responses": { "200": { "description": "OK" } } } },
            "/auth/me": { "get": { "summary": "Current principal", "responses": { "200": { "description": "OK" }, "401": { "description": "Unauthorized" } } } },
            "/workflows": {
                "get": { "summary": "List workflows", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create a workflow", "responses": { "201": { "description": "Created" }, "400": { "description": "Invalid workflow" } } }
            },
            "/workflows/{reference}": {
                "get": { "summary": "Get a workflow and its current definition", "responses": { "200": { "description": "OK" }, "404": { "description": "Not found" } } },
                "put": { "summary": "Create a new workflow version", "responses": { "200": { "description": "OK" } } },
                "delete": { "summary": "Delete a workflow and its history", "responses": { "200": { "description": "OK" } } }
            },
            "/workflows/{reference}/versions": { "get": { "summary": "List workflow versions", "responses": { "200": { "description": "OK" } } } },
            "/workflows/{reference}/runs": {
                "get": { "summary": "List runs for a workflow", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Queue a workflow run", "responses": { "202": { "description": "Queued" } } }
            },
            "/runs": { "get": { "summary": "List workflow runs", "responses": { "200": { "description": "OK" } } } },
            "/runs/{id}": { "get": { "summary": "Get a run and its block timeline", "responses": { "200": { "description": "OK" } } } },
            "/runs/{id}/cancel": { "post": { "summary": "Cancel a queued or running run", "responses": { "200": { "description": "OK" } } } },
            "/runs/{id}/artifacts": { "get": { "summary": "List run artifacts (screenshots, downloads)", "responses": { "200": { "description": "OK" } } } },
            "/tasks/{id}/artifacts": { "get": { "summary": "List task artifacts", "responses": { "200": { "description": "OK" } } } },
            "/artifacts/{id}": { "get": { "summary": "Download an artifact", "responses": { "200": { "description": "OK" }, "404": { "description": "Not found" } } } },
            "/connectors": { "get": { "summary": "List data-plane connectors", "responses": { "200": { "description": "OK" } } }, "post": { "summary": "Create a connector and return its token once", "responses": { "201": { "description": "Created" } } } },
            "/connectors/{id}": { "delete": { "summary": "Delete a connector", "responses": { "200": { "description": "OK" } } } },
            "/connector/heartbeat": { "post": { "summary": "Connector heartbeat (connector token)", "responses": { "200": { "description": "OK" } } } },
            "/connector/jobs/claim": { "post": { "summary": "Claim an outbound connector job", "responses": { "200": { "description": "OK" } } } },
            "/connector/jobs/{id}/heartbeat": { "post": { "summary": "Extend a connector job lease", "responses": { "200": { "description": "OK" } } } },
            "/connector/jobs/{id}/complete": { "post": { "summary": "Report connector job completion", "responses": { "200": { "description": "OK" } } } },
            "/metrics": { "get": { "summary": "Tenant metrics for dashboards", "responses": { "200": { "description": "OK" } } } },
            "/metrics/prometheus": { "get": { "summary": "Prometheus text metrics", "responses": { "200": { "description": "OK" } } } },
            "/tasks": {
                "get": { "summary": "List natural-language tasks", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create a task", "responses": { "201": { "description": "Created" } } }
            },
            "/tasks/{id}": { "get": { "summary": "Get a task", "responses": { "200": { "description": "OK" } } } },
            "/tasks/{id}/cancel": { "post": { "summary": "Cancel a task", "responses": { "200": { "description": "OK" } } } },
            "/credentials": {
                "get": { "summary": "List credential metadata (never secrets)", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create an encrypted credential", "responses": { "201": { "description": "Created" } } }
            },
            "/credentials/{id}": { "delete": { "summary": "Delete a credential", "responses": { "200": { "description": "OK" } } } },
            "/schedules": {
                "get": { "summary": "List schedules", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create a schedule", "responses": { "201": { "description": "Created" } } }
            },
            "/schedules/{id}": { "delete": { "summary": "Delete a schedule", "responses": { "200": { "description": "OK" } } } },
            "/schedules/{id}/enabled": { "patch": { "summary": "Enable or disable a schedule", "responses": { "200": { "description": "OK" } } } },
            "/settings": {
                "get": { "summary": "List settings", "responses": { "200": { "description": "OK" } } },
                "put": { "summary": "Set a non-secret setting", "responses": { "200": { "description": "OK" } } }
            },
            "/audit": { "get": { "summary": "List audit events", "responses": { "200": { "description": "OK" } } } },
            "/approvals": { "get": { "summary": "List approvals (default: pending)", "parameters": [ { "name": "status", "in": "query", "schema": { "type": "string" } }, { "name": "runId", "in": "query", "schema": { "type": "string" } }, { "name": "taskId", "in": "query", "schema": { "type": "string" } } ], "responses": { "200": { "description": "OK" } } } },
            "/approvals/{id}": { "post": { "summary": "Approve or deny a pending step", "responses": { "200": { "description": "OK" }, "409": { "description": "Not pending" } } } },
            "/webhooks": {
                "get": { "summary": "List webhooks", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create a webhook; secret is returned once", "responses": { "201": { "description": "Created" } } }
            },
            "/webhooks/{id}": { "delete": { "summary": "Delete a webhook", "responses": { "200": { "description": "OK" } } } },
            "/webhooks/{id}/deliveries": { "get": { "summary": "List webhook delivery attempts", "responses": { "200": { "description": "OK" } } } },
            "/api-keys": {
                "get": { "summary": "List API keys (never hashes)", "responses": { "200": { "description": "OK" } } },
                "post": { "summary": "Create an API key (token shown once)", "responses": { "201": { "description": "Created" } } }
            },
            "/api-keys/{id}": { "delete": { "summary": "Revoke an API key", "responses": { "200": { "description": "OK" } } } }
        }
    })
}
