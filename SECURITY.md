# Security Policy

DoOnce is intended for regulated enterprise environments. We take reports
seriously and will acknowledge every valid report.

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.**

Email: `security@doonce.example` (replace with the production security contact
before GA). Include:

- affected version or commit,
- a minimal reproduction or proof of concept,
- impact assessment (confidentiality, integrity, availability),
- any suggested mitigation.

If you need encrypted communication, request our PGP key in the first email.

## Response targets

| Severity | Acknowledge | Triage | Fix target |
|---|---|---|---|
| Critical | 24 hours | 48 hours | 7 days |
| High | 2 business days | 5 business days | 30 days |
| Medium | 5 business days | 10 business days | 90 days |
| Low | 10 business days | Next release | Best effort |

These targets apply to supported releases. We may require a coordinated
disclosure window for complex issues.

## Supported versions

Until GA, only the latest commit on the default branch is supported. After GA,
the current major and the previous major receive security fixes according to
the published support policy.

## Safe harbor

We will not pursue legal action for good-faith research that:

- stays within your own accounts and data,
- avoids privacy violations, data destruction and service disruption,
- reports the issue privately and gives us reasonable time to fix it.

## Security baseline in this repository

- `cargo-audit`, `cargo-deny`, `clippy -D warnings` and formatting checks run
  in CI.
- Secrets must never be committed. Use `DOONCE_MASTER_KEY` from a KMS/HSM or
  secret manager.
- The local `master.key` file is a development/single-node fallback and is
  created with `0600` permissions.
- API keys are stored as Argon2 hashes; the plaintext token is shown once.
- Audit logs are hash-chained and append-only at the application layer.
- Workflow HTTP blocks reject private, loopback, link-local and cloud-metadata
  destinations at validation time; runtime DNS re-checks are required in the
  worker phase.

## Dependency advisories

`deny.toml` records a scoped exception for **RUSTSEC-2023-0071** (`rsa 0.9.10`,
pulled in by `openidconnect`). It is used for public-key ID-token signature
verification, not RSA decryption. Compensating controls: HTTPS-only discovery,
trusted issuer configuration, no redirect following, short token/session
lifetimes, and tracking an upstream replacement.

`paste` (RUSTSEC-2024-0436) is an unmaintained build-time proc-macro pulled in by
Boa, the JavaScript sandbox. It is not runtime-reachable and has no known
vulnerability; track Boa releases and remove the exception when upstream
replaces it.

SMTP credentials are read from `DOONCE_SMTP_URL` and must be supplied through the
environment/secret manager. They are never persisted in workflows or logs.

Credential-manager sessions/tokens (`BW_SESSION`, `OP_SERVICE_ACCOUNT_TOKEN`,
`DOONCE_CREDENTIAL_HTTP_TOKEN`) are read from the environment or secret manager.
DoOnce stores only external item references; CLI output is never echoed in errors.

Worker approvals are tenant-scoped and audited. Webhooks are signed with the
per-webhook secret (`x-doonce-signature`) and private destinations are blocked
unless `DOONCE_WEBHOOK_ALLOW_PRIVATE=true`.

Artifacts are tenant-scoped and require `ArtifactRead`. Downloads resolve paths
under `DOONCE_ARTIFACT_DIR` (default: data dir `artifacts/`), reject path escape
and cap responses at 50 MB. Artifact bytes, prompts, credentials and URLs are
not exported by the metrics endpoints.

`doonce browser stream` is a local operator aid: it binds `127.0.0.1` only and
reuses the browser session. It is not a production multi-tenant streaming
service; remote screencast/takeover is future work.

The MCP server (`doonce mcp`) forwards tool calls with a scoped API key, writes
diagnostics to stderr and supports `--read-only` to hide mutating tools. SDKs
store no credentials and raise problem details without logging secrets.

Hybrid connectors authenticate with a one-time connector token (Argon2 hash at
rest), poll outbound only and lease tenant/region-scoped jobs. If the connector
is deleted or disabled, queued work fails closed rather than running locally.
Screenshots, profiles and credentials stay in the data plane; only artifact
metadata returns to the control plane. Use TLS for the control-plane URL and
rotate connector tokens by deleting/recreating the connector.

`DOONCE_DEPLOYMENT_MODE=airgapped` refuses telemetry and dev-no-auth, and the
worker requires a loopback LLM endpoint. Browser egress must also be restricted
by the network/sandbox layer; the deployment flag is not an egress firewall.

`DOONCE_ENV=production` requires Postgres, `DOONCE_MASTER_KEY` from a
KMS/secret manager, an HTTPS `DOONCE_PUBLIC_URL`, secure cookies and no
dev-no-auth. These checks run at API startup before serving traffic.

## Out of scope (current phase)

The repository now contains the browser worker, REST API, web console, CLI,
SDKs, MCP server, hybrid connector and deployment packaging. SAML 2.0,
WebAuthn/passkeys, connector mTLS/request signing, signed update bundles,
offline license enforcement and remote artifact byte relay are still pending.
Reports about components that have not landed are accepted once they are
released, but the security architecture and review requirements already apply.
