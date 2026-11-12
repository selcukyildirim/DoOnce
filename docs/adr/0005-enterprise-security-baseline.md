# ADR 0005 — Enterprise security baseline

- Status: Accepted
- Date: 2026-11-06

## Context

The product targets SOC 2 Type II, ISO 27001, GDPR, KVKK, HIPAA and PCI-DSS.
Security cannot be retrofitted after the API, browser worker and credential
vault exist. We need binding defaults for every contributor.

## Decision

The following are mandatory from the first commit:

1. **Deny by default.** Every API/worker action checks an explicit permission
   and tenant scope.
2. **Tenant isolation is explicit.** No ambient tenant state; every repository
   call takes `tenant_id`. Cross-tenant tests are required.
3. **Secrets are encrypted at rest.** AES-256-GCM envelope encryption with a
   KMS/HSM-backed master key in production. Local key files are for development.
4. **Secrets never leave the process unredacted.** Logs, events, prompts,
   errors and API responses pass through a redactor.
5. **API keys are hashed.** Argon2id, prefix lookup, rotate/revoke, show once.
6. **Audit is tamper-evident.** Append-only, hash-chained records with SIEM
   export in the server phase.
7. **Outbound network is guarded.** Workflow HTTP blocks reject private,
   loopback, link-local and metadata endpoints; runtime DNS re-checks are
   required in the worker.
8. **Input is validated strictly.** Workflow size/step/string limits, schema
   checks, URL allowlists and secret-placement rules.
9. **Supply chain is verifiable.** `cargo-deny`, `cargo-audit`, pinned
   lockfile, SBOM, signed artifacts and SLSA provenance targets.
10. **Secure SDLC.** ADR/threat model per feature, security review for auth,
    crypto, tenancy, network, browser, LLM and file paths.
11. **No AGPL code.** See ADR 0002.
12. **Production defaults are safe.** Telemetry off, TLS 1.3, mTLS internally,
    SQLite rejected in `DOONCE_ENV=production`.

## Consequences

- Some shortcuts are intentionally unavailable.
- Security review and tests add lead time; this is accepted for the target
  market.
- Any exception requires a written ADR with a compensating control and an
  expiry date.
