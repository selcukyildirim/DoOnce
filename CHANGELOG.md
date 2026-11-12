# Changelog

All notable changes are documented here. The project follows semantic
versioning after GA.

## [Unreleased]

### Added

- Rust full-stack control plane, worker, browser/CDP engine, block engine,
  AI task agent, credentials, schedules, approvals and HMAC webhooks.
- OIDC SSO, SCIM, TOTP MFA, RBAC, tenant isolation, audit hash chain.
- Screenshot artifacts, local live view, Prometheus metrics and web console.
- Python/TypeScript SDKs, MCP stdio server and n8n/Zapier/Make recipes.
- Outbound-only hybrid connector, air-gapped mode, region-pinned residency.
- Docker, Compose, Helm, systemd and air-gap bundle packaging.
- Release CI with cross-platform binaries, SBOM, provenance and cosign signing.
- Compliance evidence export script and GA/compliance runbooks.

### Known gaps

- Phase 11 (external penetration test, DR drills, audit evidence automation in
  a live environment) was explicitly skipped and remains an external gate.
- SAML 2.0, WebAuthn/passkeys, connector mTLS/request signing, signed update
  bundles, offline licensing and remote artifact byte relay are not implemented.
- SOC 2 / ISO 27001 / HIPAA / PCI certification requires an external audit and
  organizational evidence; this repository only provides technical controls.
