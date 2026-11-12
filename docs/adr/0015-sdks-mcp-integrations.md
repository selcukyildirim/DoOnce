# ADR 0015 — SDKs, MCP and low-code integrations

- Status: Accepted
- Date: 2026-11-06

## Context

Operators and integrators should not hand-roll HTTP calls, and AI clients need
a safe, discoverable tool surface. Shipping official SDKs, an MCP server and
low-code recipes reduces integration errors without expanding the trusted
compute base.

## Decision

1. **Python SDK** (`sdk/python`) is a synchronous, dependency-free client
   built on `urllib`. It covers workflows, runs, tasks, approvals, artifacts and
   metrics, and surfaces RFC 9457 problem details as `DoOnceError`.
2. **TypeScript SDK** (`sdk/typescript`) targets Node 18+/browsers with the
   global `fetch` and ships declarations. No runtime dependencies.
3. **MCP server** (`doonce mcp`) speaks JSON-RPC 2.0 over stdio, forwards tool
   calls to the REST API, and supports `--read-only` to hide mutating tools.
   Logs go to stderr; stdout carries protocol messages only.
4. **Low-code integrations** use the versioned REST API/OpenAPI document.
   n8n imports the OpenAPI schema into HTTP Request nodes; Zapier/Make use
   webhook/HTTP modules plus signed DoOnce webhooks for completion events.
5. **Security defaults:** one scoped API key per integration, least-privilege
   role, no secrets in workflow definitions, no credentials or artifact bytes
   in MCP text output, and no stdout diagnostics from the MCP transport.

## Consequences

- The API remains the single source of truth; SDKs/MCP are thin clients.
- SDKs can be vendored into restricted environments; no transitive packages.
- Read-only MCP mode is suitable for analysis assistants; mutating mode should
  be granted only to trusted operators.
- Deeper native nodes for n8n/Zapier/Make and an MCP resources/prompts surface
  remain future work.
