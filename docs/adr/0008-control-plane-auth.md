# ADR 0008 — Control plane, API-key auth and console shell

- Status: Accepted
- Date: 2026-11-06

## Context

Phases 1–3 produced the domain, browser and AI runtimes. Enterprise customers
need a stable control plane: authenticated REST APIs, tenant isolation, audit
events and a UI. Full OIDC/SAML/SCIM/MFA is a larger identity program and must
not block the API/UI foundation.

## Decision

1. **Axum control plane** under `/api/v1`, hosted by `doonce-server`.
2. **API-key auth** (`x-api-key` or `Authorization: Bearer`) with Argon2
   verification. Keys are looked up by non-secret prefix; plaintext is shown
   once on creation.
3. **Deny-by-default RBAC** (`owner`, `admin`, `operator`, `viewer`, `auditor`)
   and explicit `tenant_id` on every repository call.
4. **RFC 9457 Problem Details** for errors; request IDs via `tower-http`.
5. **OpenAPI 3.1** served at `/api/v1/openapi.json`.
6. **Write-only credentials.** The API encrypts secrets with the master key and
   never returns ciphertext/plaintext.
7. **Audit events** for mutations. Phase 4 appends best-effort; transactional
   audit coupling is a later hardening item.
8. **React console** served from the same origin. The API key is stored in
   `localStorage` for this shell and sent as `x-api-key`.
9. **Dev bypass is explicit and dangerous:** `DOONCE_DEV_NO_AUTH=1` /
   `--dev-no-auth` acts as the default tenant owner. It must be rejected in
   production deployments by policy and startup checks in later phases.
10. **Execution is deferred.** The API queues workflow runs; the worker/queue
    executes them in Phase 7. CLI execution remains available meanwhile.

## Consequences

- The control plane is usable today for workflow/task management, credentials,
  schedules, audit and API-key rotation.
- OIDC/SAML/SCIM/MFA will replace API-key-only login for humans while keeping
  service-account API keys.
- localStorage is acceptable for the shell but will move to secure, HttpOnly
  session cookies (or short-lived tokens) with SSO.
- Audit atomicity and worker execution are explicit follow-ups, not implicit
  assumptions.
