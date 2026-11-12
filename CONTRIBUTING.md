# Contributing to DoOnce

DoOnce is an enterprise product. Contributions must preserve security,
auditability and backward compatibility.

## Before you start

- Read `docs/adr/` and `docs/security/threat-model.md`.
- For a new feature, open an ADR/RFC describing the threat model, data flow,
  API changes and test plan.
- Never include real credentials, customer data or production URLs in issues,
  tests or commits.

## Development workflow

1. Fork or branch from the default branch.
2. Use conventional commit messages (`feat:`, `fix:`, `docs:`, `security:`,
   `refactor:`, `test:`, `chore:`).
3. Keep changes focused. Large refactors need a prior RFC.
4. Add tests for every behavior change. Security fixes require a regression
   test.
5. Run the full gate locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
```

6. Open a pull request with:
   - problem statement and threat considerations,
   - migration/rollback plan if the schema or API changes,
   - test evidence,
   - documentation updates.

## Code standards

### Rust

- `#![forbid(unsafe_code)]` unless an ADR justifies an exception.
- No `unwrap`/`expect` in request, worker or CLI paths; return typed errors.
- All I/O has timeouts. Async code must not block.
- Secrets use `zeroize`/`secrecy`; never log them.
- Every public API has docs and a test.

### Database

- Append migrations; never edit a released migration.
- Every tenant-owned table has `tenant_id` and an index.
- Queries must be tenant-scoped. Cross-tenant tests are mandatory.
- Migrations must work on SQLite and Postgres until the native-Postgres
  migration in ADR 0004.

### API

- OpenAPI 3.1 is the contract.
- Errors use RFC 9457 Problem Details.
- New endpoints require auth, authorization and audit events.
- Breaking changes require a version bump and a deprecation plan.

### Frontend

- WCAG 2.2 AA, keyboard navigation and screen-reader labels.
- CSP, Trusted Types, no secrets in client bundles.
- Dependency pinning and license checks.

## Review requirements

- At least one maintainer approval.
- Security review for: auth, crypto, tenancy, secrets, network egress,
  browser isolation, LLM prompts and file handling.
- CI must be green. Do not merge with failing security checks.

## Definition of Done

- Threat model and ADR updated.
- Unit, integration and (where relevant) e2e tests.
- Authorization and tenant-isolation tests.
- Metrics/alerts/SLOs for new runtime paths.
- Runbook and rollback notes.
- SBOM/provenance impact reviewed.
- Documentation updated.

## License and provenance

DoOnce is Apache-2.0. By contributing you agree that your contribution is
licensed under Apache-2.0. Do not copy code from AGPL-licensed projects; see
`docs/adr/0002-clean-room-licensing.md`.
