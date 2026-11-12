# ADR 0013 — Worker queue, scheduler, approvals and webhooks

- Status: Accepted
- Date: 2026-11-06

## Context

The API could queue runs but nothing executed them. Enterprise automation also
needs cron schedules, human approval for risky steps, cancellation, retries and
outbound notifications, without tying execution to the API process.

## Decision

1. **Separate worker process** (`doonce worker`) claims queued workflow runs and
   tasks from the shared database. The API remains stateless.
2. **Database leases.** Claims atomically move `queued` → `running` and set
   `locked_at/locked_by/attempts`. A heartbeat refreshes the lease. Stale
   `running` jobs are requeued by the worker.
3. **Cancellation** is DB-driven: a watcher polls the run/task status and
   cancels the engine/agent cancellation token. The final status never
   overwrites an explicit `cancelled`.
4. **Scheduler** evaluates due cron schedules every 10s, creates workflow runs
   with `trigger = "schedule"` and advances `next_run_at` using the configured
   timezone (`chrono-tz`).
5. **Approvals** are persisted in `approvals`. A risky step pauses the job,
   sets the run/task to `awaiting_approval`, and polls for a decision.
   Approve/deny via API/UI; timeout denies the step. `--auto-approve` is an
   explicit trusted-automation escape hatch.
6. **Webhooks** are signed with HMAC-SHA256
   (`x-doonce-signature: sha256=...`), retried up to three times with backoff,
   and recorded in `webhook_deliveries`. Redirects are disabled; private
   destinations require `DOONCE_WEBHOOK_ALLOW_PRIVATE=true`.
7. **Step/action history** is written via a channel-backed observer so the
   synchronous observer trait never blocks the runtime.
8. **Browser isolation** uses one Chrome profile/download directory per job.
   Cancellation and leases prevent two workers from running the same job.
9. **Fail closed:** missing Chrome, missing AI provider or missing credentials
   mark the job failed with an explicit reason rather than silently skipping.

## Consequences

- API and worker can scale independently; SQLite remains single-node friendly
  and Postgres is the scale-out path.
- Approvals require an operator/API client; automation can opt into
  auto-approval only deliberately.
- Webhook receivers authenticate with the signing secret; secret rotation and
  replay protection are future hardening items.
- Artifact/screenshot collection, live view and metrics are covered by ADR 0014.
