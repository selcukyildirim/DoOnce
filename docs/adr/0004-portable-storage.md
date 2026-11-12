# ADR 0004 — Portable SQLite/Postgres persistence

- Status: Accepted
- Date: 2026-11-06

## Context

Local and air-gapped deployments need zero-dependency startup. Enterprise SaaS
and HA deployments need Postgres, row-level security and native types. We want
one repository layer during the foundation phase without maintaining two SQL
dialects for every query.

## Decision

- Use SQLx's `Any` driver with dialect-specific migrations.
- Store IDs, timestamps and JSON as `TEXT` in both engines for the foundation.
- Use `BIGINT`/`INTEGER` for counters and booleans (`0/1`).
- Repositories use `?` placeholders; `Database::sql` rewrites them to `$n` for
  Postgres.
- Every query is tenant-scoped explicitly; no ambient tenant context.

## Consequences

- The same repository code works on SQLite and Postgres from day one.
- Native Postgres features (UUID, JSONB, TIMESTAMPTZ, RLS, partial indexes,
  `jsonb_path_ops`) are not used yet.
- A future ADR will introduce native Postgres types and RLS. That migration
  must preserve the wire format and be performed online with a backfill job.
- Performance is adequate for the foundation; high-volume artifact/audit tables
  will be partitioned in the native-Postgres phase.

## Alternatives considered

- `sqlx::Any` with native types: rejected because type encoding differs across
  drivers.
- Separate repositories per engine: rejected because it doubles the security
  review surface.
- Postgres-only: rejected because it breaks local/air-gapped quickstart.
