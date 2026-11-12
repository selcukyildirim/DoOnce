# ADR 0016 — Hybrid connector, air-gapped mode and residency

- Status: Accepted
- Date: 2026-11-07

## Context

ADR 0003 defined a control plane/data plane split with outbound-only
connectivity, but the code still assumed API and worker shared one database.
Regulated customers also need an explicit air-gapped mode and a region boundary
that is visible in the data model and deployment config.

## Decision

1. **Connector registry.** `connectors` stores tenant-scoped connectors with an
   Argon2-hashed token, region, version and last heartbeat. The plaintext token
   is returned once at creation.
2. **Outbound leases.** When a tenant setting `execution.connector_id` points to
   an active connector, new workflow runs/tasks create a `connector_jobs` row in
   the same database transaction. Local workers skip jobs with an active
   connector lease. Connectors claim jobs with a lease and report completion;
   leases expire so a dead connector can be replaced.
3. **Fail closed.** A tenant bound to a deleted/inactive connector cannot queue
   work. Jobs routed to a connector are never executed by a local worker.
4. **Local data plane execution.** `doonce connector` polls the control plane,
   upserts the workflow into the local API, queues the run/task, waits for the
   local worker, optionally approves local gates, and reports status/output.
   Screenshots, browser profiles and credentials remain in the data plane; only
   artifact metadata is returned.
5. **Deployment mode.** `DOONCE_DEPLOYMENT_MODE` is `saas`, `onprem` or
   `airgapped`. Air-gapped mode refuses telemetry and dev-no-auth; the worker
   rejects non-loopback LLM endpoints. Browser egress policy is a deployment
   control (network policy/sandbox), not a replacement for this flag.
6. **Residency.** Tenants and connectors carry a `region`; connectors must match
   a non-global tenant region. `DOONCE_REGION` pins the installation and artifact
   root. Multi-region routing requires separate installations; one database is
   not a cross-region router.
7. **Packaging.** Docker Compose, Helm, systemd units and an air-gapped bundle
   script ship in `deploy/`. The container uses a Chromium wrapper for sandbox
   incompatibility; production should use a dedicated browser sandbox.

## Consequences

- The control plane never needs an inbound route into the customer VPC.
- Connector tokens are bearer credentials over TLS; mTLS and signed request
  bodies remain hardening items.
- Remote artifact bytes are intentionally not uploaded; operators download
  them from the data plane. A future signed artifact relay can add this without
  changing residency defaults.
- Local approvals require `--auto-approve` for unattended connectors; otherwise
  the connector fails closed and asks an operator to resolve the gate.
