# ADR 0003 — Hybrid control plane / data plane

- Status: Accepted
- Date: 2026-11-06

## Context

Enterprise customers split into two groups: those who accept a multi-tenant
SaaS control plane, and regulated customers who require their browser data,
screenshots, credentials and artifacts to remain inside their VPC or an
air-gapped environment. We need one product that serves both without forking
the codebase.

## Decision

Separate the platform into two logical planes:

- **Control plane (SaaS):** UI, identity/SSO/SCIM, RBAC, tenant metadata,
  licensing, scheduling metadata, billing/metering and audit ingestion.
- **Data plane (customer VPC / on-prem / SaaS worker):** API/worker/browser
  processes, credential vault, artifacts, local audit log and customer data.

The data plane connects to the control plane **outbound only** using mTLS and
short-lived credentials. There are no inbound ports. Air-gapped deployments
run both planes locally with an offline license and signed update bundles.

## Consequences

- All services must tolerate split connectivity and reconcile state.
- Job dispatch uses an outbound pull/lease model, not inbound RPC.
- Tenant data residency is a first-class routing concern.
- The connector (`doonce-connector`) becomes a security-critical component and
  needs its own threat model and update channel.
- Local development defaults to a single node running both planes.
