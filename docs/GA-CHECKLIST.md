# GA readiness checklist

This is an engineering checklist, not a certification claim. Phase 11
(penetration test, DR drills, live audit evidence automation) was explicitly
skipped and remains an external/operational gate before GA.

## Product and security

- [x] Tenant isolation and deny-by-default RBAC tests.
- [x] API-key Argon2 hashing, OIDC PKCE, sessions/CSRF, TOTP MFA, SCIM.
- [x] Secret redaction, AES-256-GCM credentials, audit hash chain.
- [x] SSRF/DNS guards, sandboxed JS, bounded loops and timeouts.
- [x] Artifact path confinement and 50 MB download cap.
- [x] Connector outbound-only leases, fail-closed deletion and region matching.
- [x] Production guardrails: Postgres required, KMS/secret-manager master key,
  HTTPS public URL and secure cookies.
- [ ] External penetration test and remediation sign-off.
- [ ] SAML 2.0 / WebAuthn and connector mTLS/request signing.

## Operations

- [x] Docker, Compose, Helm, systemd, air-gap bundle and backup runbook.
- [x] Prometheus metrics, health/readiness endpoints and audit export.
- [ ] DR restore drill with measured RTO/RPO.
- [ ] On-call rotation, paging and incident tabletop exercise.
- [ ] Capacity/load test at the published SLO.

## Compliance

- [x] Control matrix, threat model, ADRs, SECURITY.md and evidence exporter.
- [x] SBOM, dependency/license policy and release provenance/signing.
- [ ] SOC 2 Type II observation window and auditor report.
- [ ] ISO 27001 certification audit.
- [ ] HIPAA BAAs and PCI scope validation for each customer deployment.
- [ ] DPA/subprocessor register, ROPA/DPIA and retention approvals.

## Release

- [x] CI: fmt, clippy, 102+ tests, cargo-deny, web, SDK, Helm/Compose checks.
- [x] Tagged release workflow: linux/macOS/Windows binaries, checksums, SBOM,
  GHCR image, provenance and cosign keyless signature.
- [x] CHANGELOG and semantic versioning policy.
- [ ] Release candidate soak in staging with real IdP and customer-like data.
- [ ] Rollback/upgrade rehearsal across two releases.
