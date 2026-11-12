# DoOnce Threat Model

- Version: 1 (foundation phase)
- Owner: Security Engineering
- Review cadence: every major feature, at least quarterly

## 1. Scope and assets

| Asset | Why it matters |
|---|---|
| Customer credentials, TOTP seeds, API keys, LLM keys | Direct account takeover / data breach |
| Browser sessions, cookies, localStorage, CDP endpoints | Impersonation of customer users |
| Screenshots, HAR, downloads, DOM snapshots | May contain PII, PHI or cardholder data |
| Workflow definitions and AI prompts | Business logic, potential secrets, prompt injection surface |
| Audit logs | Regulatory evidence, tamper target |
| Tenant metadata and billing data | Privacy and contractual impact |
| Build artifacts, SBOM, signing keys | Supply-chain compromise |

## 2. Trust boundaries

1. **Internet → edge/API**: untrusted clients, API keys, SSO.
2. **Control plane → data plane connector**: outbound-only mTLS, leased jobs.
3. **Server → browser worker**: trusted but must not assume page safety.
4. **Browser worker → arbitrary website**: hostile content, prompt injection.
5. **Worker → LLM provider**: sensitive context leaves the trust boundary.
6. **Tenant → tenant**: strict isolation via repository scoping, RLS later.
7. **Build pipeline → release artifacts**: supply-chain boundary.

## 3. STRIDE summary

| Threat | Example | Controls | Status |
|---|---|---|---|
| Spoofing | Stolen API key or session cookie | Argon2 hashed keys, prefix lookup, rotation/revocation, SSO+MFA target, audit `last_used_at` | Partial |
| Tampering | Editing workflow or audit row | Strict validation, revision history, hash-chained audit, WORM/SIEM target | Partial |
| Repudiation | Admin denies running a destructive block | Audit events with actor/IP/tenant, approval records | Partial |
| Information disclosure | Secret in logs, cross-tenant read, artifact leak | Redaction, tenant-scoped repos, encrypted credentials, artifact authorization | Partial |
| Denial of service | Run flood, huge workflow, browser exhaustion | Size/step limits, quotas, rate limits, queue backpressure (worker phase) | Partial |
| Elevation of privilege | Viewer triggers workflow, tenant escape | Deny-by-default RBAC, explicit tenant_id, cross-tenant tests | Partial |

## 4. Browser and AI-specific threats

| Threat | Impact | Required controls |
|---|---|---|
| Prompt injection from page content | Agent performs unintended actions, exfiltrates data | Treat page text as data, tool allowlist, URL allowlist, approval for risky actions, injection regression suite |
| SSRF via HTTP block | Access cloud metadata / internal services | Validation-time private-IP block; runtime DNS resolve + re-check; egress proxy allowlist |
| Credential exfiltration | Account takeover | Secrets only injected into fill/HTTP auth sinks; never in prompts; redaction; per-tenant vault |
| Malicious download | Malware on worker/client | Download sandbox, size/type limits, AV scan hook, content-disposition handling |
| Browser sandbox escape | Host compromise | Container/gVisor isolation, no host mounts, non-root, seccomp/AppArmor |
| Cross-tenant browser reuse | Data leakage via cookies/storage | Ephemeral context per run, no shared profile, per-tenant encryption |
| Artifact overexposure | PII/PHI leak | Tenant-scoped signed URLs, retention policy, access audit, DLP scan |
| LLM data retention | Confidential data at provider | Provider allowlist, DPA, zero-retention mode, PII redaction, local model option |
| Model hallucinated action | Irreversible side effect | Deterministic action schema, risk approval, max steps, cost/time budget |
| Workflow import attack | RCE via code block or secret leak | Sandboxed JS, no fs/network by default, strict validation, approval |
| Audit tampering | Failed compliance evidence | Hash chain, append-only store, SIEM export, restricted DB roles |

## 5. Foundation-phase mitigations implemented

- Workflow v2 validation: unique IDs, control targets, URL allowlists, secret
  placement, SSRF guard for outbound HTTP, size/step limits.
- AES-256-GCM encryption with `zeroize` and redacted `Debug`.
- Argon2 API key hashing; plaintext shown once.
- RBAC permissions; explicit `tenant_id` in every repository call.
- Secret redaction for logs/JSON/headers.
- SHA-256 audit chain with tamper detection.
- AI agent safety: refs-only actions, action allowlist, untrusted page boundary,
  JSON Schema extraction validation, step/time/failure budgets.
- Identity: OIDC authorization code + PKCE, issuer/audience/nonce validation,
  encrypted client secrets, JIT provisioning with verified-email linking.
- Browser sessions: opaque HttpOnly/SameSite=Strict cookies, SHA-256 token hashes,
  CSRF double-submit, server-side revocation and TOTP MFA gating.
- SCIM: scoped API keys (`scim`), tenant-scoped user provisioning and audit.
- Block engine: JSON Logic conditions, bounded loops, HTTP SSRF/DNS re-checks,
  redirects disabled, 2 MB response cap, sandboxed Boa JavaScript with
  loop/recursion/time limits, canonicalized artifact directory for file parsing.
- Service blocks: AI approvals routed through the engine, credentials decrypted
  only in memory, TOTP codes generated per run, SMTP credentials environment-only,
  attachments restricted to the artifact directory, upload destinations confined
  to the upload root, nested workflow depth capped.
- Credential providers: external references only, shell-free CLI execution with
  timeouts, no stdout/stderr in errors, HTTP credential service redirects
  disabled and private addresses opt-in, fresh TOTP per run.
- Worker: DB leases/heartbeats prevent duplicate execution, stale jobs requeue,
  cancellation is DB-driven, approvals are tenant-scoped and audited, webhooks
  use HMAC-SHA256 with retries and SSRF/private-address guards.
- Artifacts: tenant-scoped rows, `ArtifactRead` authorization, canonicalized
  paths that cannot escape the artifact root, inline download with a 50 MB cap,
  no artifact content in logs or metrics.
- Live view: `doonce browser stream` binds loopback only and reuses the
  authenticated browser session; it is not exposed by the API.
- Metrics: tenant-scoped snapshots of counts/latency/success rate and queue
  depth; no prompts, credentials, URLs or artifact bytes are exported.
- SDKs/MCP: scoped API keys only, MCP stdio logging on stderr, explicit tool
  allowlist, `--read-only` hides mutating tools, no secrets or artifact bytes in
  MCP text output, and no runtime dependencies in the official SDKs.
- Hybrid connector: outbound-only polling, Argon2-hashed connector tokens,
  tenant/region-scoped leases, local workers skip connector-routed jobs, jobs
  fail closed if the connector is inactive, and only artifact metadata returns
  to the control plane.
- Air-gapped/deployment: `DOONCE_DEPLOYMENT_MODE` rejects telemetry and
  dev-no-auth; the worker rejects non-loopback LLM endpoints. Residency is
  pinned with `DOONCE_REGION` and tenant/connector regions.
- Production guardrails: `DOONCE_ENV=production` requires Postgres, an
  environment/KMS master key, HTTPS public URL and secure cookies; SQLite and
  dev-no-auth are rejected at startup.
- `cargo-deny` (AGPL/GPL denied), `cargo-audit`, clippy `-D warnings`.
- Secure local defaults: telemetry off, production SQLite rejection.

## 6. Open risks to close in later phases

1. Postgres RLS and per-tenant KMS keys (native-Postgres migration).
2. Runtime DNS re-check for outbound HTTP and browser navigation.
3. Browser worker isolation (gVisor/Firecracker) and egress policy.
4. SSO/SAML/SCIM/MFA and step-up authentication.
5. SIEM export and immutable audit storage.
6. DLP/PII scanning for artifacts and LLM prompts.
7. Provider DPAs, zero-retention contracts and local-model fallback.
8. Supply chain: SBOM, cosign signatures, SLSA provenance.
9. Penetration test and cross-tenant isolation test suite.
10. DR/backup restore drills and incident-response runbooks.
11. Track the `rsa` RUSTSEC-2023-0071 advisory pulled in by `openidconnect`;
    the vulnerable decryption path is not exposed, but an upstream replacement
    or a post-quantum/hybrid JWT stack should be evaluated.
12. SAML 2.0 and WebAuthn/passkeys are not implemented yet; add them with the
    same session/CSRF/audit model.
13. Track the `paste` RUSTSEC-2024-0436 build-time advisory from Boa; it is not
    runtime-reachable but should be removed when upstream replaces it.
14. Object storage backends (S3/GCS/Azure), artifact retention/legal hold,
    artifact encryption-at-rest and DLP scanning are still pending. Local
    uploads and the local artifact root remain the default until reviewed.
15. Webhook replay protection/idempotency keys and secret rotation are pending.
16. Worker per-job browser containers and network egress policy are Phase 10.
17. CDP screencast/remote browser takeover for the live view is Phase 8b; the
    current stream endpoint is local-only and not a hardened multi-tenant
    service.
18. Native n8n/Zapier/Make nodes, SDK publishing/signing and MCP resource
    subscriptions are future work; current integrations use OpenAPI and HTTP.
19. Connector transport is TLS + bearer token; mTLS, request signing and token
    rotation are pending. Remote artifact byte relay and local-artifact proofs
    are not implemented.
20. Air-gapped browser egress policy, signed update bundles and offline license
    enforcement are deployment/process controls that still need hardening.
21. Multi-region routing is per-installation; a single control plane does not
    yet route tenants between regional databases.

## 7. Security review checklist (per feature)

- New trust boundary or data flow?
- New secret or PII handling?
- New outbound network path?
- New browser action or file path?
- New tenant-scoped table/query?
- Authorization and audit events?
- Abuse/rate-limit cases?
- Prompt injection or model-output validation?
- Tests: negative, cross-tenant, secret leak, SSRF, injection.
- Monitoring/alerting and rollback plan.
