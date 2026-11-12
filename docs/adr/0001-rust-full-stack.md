# ADR 0001 — Rust full-stack

- Status: Accepted
- Date: 2026-11-06

## Context

The previous DoOnce codebase combined a Rust replay engine with a Chrome
extension. The enterprise target requires a browser worker, API server,
scheduler, credential vault, artifact pipeline, SDKs and a web console. We
must pick a primary implementation language and runtime.

## Decision

Build the platform as a Rust workspace:

- `axum` for the REST/WebSocket control plane,
- SQLx (SQLite/Postgres) for persistence,
- CDP (`chromiumoxide`/raw CDP) for browser control,
- a TypeScript/React web console and thin Python/TS SDKs,
- one binary family (`doonce`) for serve, worker, CLI and diagnostics.

## Rationale

- Memory safety and strong concurrency for long-running browser/worker
  services.
- Single static binary simplifies on-prem, air-gapped and container
  deployments.
- Existing Apache-2.0 knowledge (locator scoring, recording compiler, CDP page
  agent) can be reimplemented in the same language.
- Predictable performance and resource usage per browser worker.

## Consequences

- Vision/LLM and browser ecosystems are less mature than Python; we accept
  more integration work and must build adapters carefully.
- Python/TypeScript SDKs remain separate packages and need generated types.
- A clean-room discipline is mandatory because the primary reference is AGPL.
- We must invest early in property/fuzz/load testing to compensate for the
  smaller Rust browser-automation ecosystem.
