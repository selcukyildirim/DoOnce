# Integrations

DoOnce exposes everything third-party automation needs through the versioned
REST API and the MCP server. Credentials stay in the caller's secret store; the
OpenAPI document is available at `/api/v1/openapi.json`.

## MCP (Claude Desktop, Cursor, other clients)

Start the stdio server in the client configuration. The control plane must be
running (`doonce serve`):

```json
{
  "mcpServers": {
    "doonce": {
      "command": "/usr/local/bin/doonce",
      "args": ["mcp", "--url", "http://127.0.0.1:8080"],
      "env": {
        "DOONCE_API_KEY": "dsk_replace_me"
      }
    }
  }
}
```

Use `--read-only` to hide tools that create, run or cancel work. The server
exposes `doonce_list_workflows`, `doonce_get_workflow`, `doonce_list_runs`,
`doonce_get_run`, `doonce_list_tasks`, `doonce_get_task`,
`doonce_list_artifacts`, `doonce_list_approvals`, `doonce_metrics`,
`doonce_create_workflow`, `doonce_run_workflow`, `doonce_cancel_run`,
`doonce_create_task`, `doonce_cancel_task` and `doonce_resolve_approval`.

Tool output is JSON text. Prompts, credentials, screenshots and secret values
are never written to stdout logs; diagnostics go to stderr.

## n8n

Use the built-in **HTTP Request** node, not a custom node:

1. Import `/api/v1/openapi.json` as a custom API in the HTTP Request node (or
   configure the request manually).
2. Base URL: `https://doonce.example.com/api/v1`.
3. Authentication: header `x-api-key` from an n8n credential.
4. Typical sequence:
   - `POST /workflows/{slug}/runs` with `{ "inputs": {...} }`
   - `GET /runs/{id}` until `status` is `completed`, `failed` or `cancelled`
   - `GET /runs/{id}/artifacts`
   - `GET /artifacts/{artifactId}` for the screenshot bytes

`examples/integrations/n8n-run-workflow.json` is an importable starting point.

## Zapier and Make

Both platforms can call the same endpoints with **Webhooks by Zapier / HTTP** or
**Make HTTP** modules:

- Action: `POST /workflows/{slug}/runs` to queue work.
- Search/action: `GET /runs/{id}` to poll the result.
- Action: `GET /artifacts/{id}` to attach a screenshot.
- Trigger alternative: subscribe a webhook (`POST /webhooks`) for
  `run.completed`, `run.failed` and `task.completed` events. Verify the
  `x-doonce-signature` HMAC before acting.

Create a dedicated tenant-scoped API key per integration, restrict it to the
minimum role, and rotate it on a schedule. Do not place API keys in workflow
definitions or prompt text.
