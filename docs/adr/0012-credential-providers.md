# ADR 0012 — External credential providers

- Status: Accepted
- Date: 2026-11-06

## Context

Enterprise customers already store credentials in Bitwarden, 1Password,
LastPass or an internal credential service. Copying passwords into DoOnce is
unacceptable. The vault must support references to those systems while keeping
the local encrypted vault as the default.

## Decision

1. **Credentials carry a `provider` and an optional `external_ref`.** The local
   vault is `provider = "local"`. External providers store only the reference
   (item id/path/URL segment), never the password.
2. **CLI adapters run without a shell.** Bitwarden (`bw get item <ref> --raw`),
   1Password (`op item get <ref> --format json`) and LastPass
   (`lpass show --json <ref>`) are invoked with `tokio::process::Command` and a
   20-second timeout. The reference is passed as a separate argv item.
3. **Secrets never reach logs or errors.** Adapter errors report only the
   command and exit code; stdout/stderr are not echoed.
4. **Sessions/tokens come from the environment or secret manager**
   (`BW_SESSION`, `OP_SERVICE_ACCOUNT_TOKEN`, `DOONCE_CREDENTIAL_HTTP_TOKEN`).
   They are never persisted in workflows or the database.
5. **Custom HTTP service** uses `GET {base}/{ref}` with optional bearer auth.
   Redirects are disabled. Unless
   `DOONCE_CREDENTIAL_HTTP_ALLOW_PRIVATE=true`, the URL must be public http(s).
6. **TOTP** is extracted from `otpauth://` URIs or a raw base32 secret and a
   fresh code is generated per run.
7. **The API and CLI never return secrets.** `CredentialView` exposes provider
   and reference only. `doonce credential test` prints a masked secret.
8. **Unknown providers fail closed.** Local credentials still require an
   encrypted secret; external credentials require `externalRef`.

## Consequences

- Customers can keep their existing password manager as the source of truth.
- The local vault remains the zero-dependency default for air-gapped installs.
- CLI adapters require the corresponding CLI to be installed and authenticated
  on the worker host. HTTP service is the alternative for containerized
  deployments.
- Provider-specific parsing is covered by unit tests; live provider calls are
  environment-dependent and belong in integration/staging tests.
- Phase 7 worker runs will use the same `CredentialResolver`, so credential
  handling does not fork between CLI and server execution.
