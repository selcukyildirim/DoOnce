# ADR 0014 — Artifacts, local live view and observability

- Status: Accepted
- Date: 2026-11-06

## Context

Operators need evidence of what the browser did, a way to watch a run while it
is executing, and queue/SLO numbers for dashboards and alerts. Phase 7 already
persisted run/task state, block history and webhook deliveries, but screenshots
and metrics were not exposed.

## Decision

1. **Final screenshots are first-class artifacts.** After a successful workflow
   run or task, the worker captures a PNG and writes both the file and an
   `artifacts` row (`kind = "screenshot"`, tenant/run/task scoped, relative
   `storage_key`, SHA-256, size).
2. **Local artifact store first.** Files live under `DOONCE_ARTIFACT_DIR`
   (default: data dir `artifacts/`). Object storage (S3/GCS/Azure) is a later
   adapter; the API contract does not expose the storage backend.
3. **Authorization on every read.** Listing and download require
   `ArtifactRead`. Downloads canonicalize the resolved path and reject anything
   that escapes the artifact root; responses are capped at 50 MB.
4. **Local live view is loopback-only.** `doonce browser stream` binds
   `127.0.0.1:<port>`, serves a small HTML page and returns 1 fps PNG frames
   captured from the existing browser session. It is a development/operator
   aid, not a multi-tenant production streaming service.
5. **Metrics are tenant-scoped.** `/metrics` returns JSON and
   `/metrics/prometheus` returns Prometheus text with workflow/task counts by
   status, queue depth, pending approvals, success rate, average run duration
   and artifact count. No prompt, credential or artifact content is exported.
6. **Console surfaces the data.** The React console shows SLO cards and a
   per-run screenshot gallery; blob URLs are revoked when the gallery closes.

## Consequences

- The worker and API share only the database and artifact root; no new daemon.
- Artifact retention, encryption-at-rest policy, DLP scanning, signed URLs and
  object storage remain hardening items.
- The live view is intentionally simple and local; CDP screencast and remote
  browser takeover are Phase 8b.
- Metrics are point-in-time snapshots, not a replacement for a time-series
  backend; Prometheus scrapes the text endpoint directly.
