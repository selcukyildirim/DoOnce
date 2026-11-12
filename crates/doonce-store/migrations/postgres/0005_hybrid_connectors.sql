-- Deployment region and outbound-only hybrid data-plane connectors.

ALTER TABLE tenants ADD COLUMN region TEXT NOT NULL DEFAULT 'global';

CREATE TABLE IF NOT EXISTS connectors (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    prefix      TEXT NOT NULL UNIQUE,
    token_hash  TEXT NOT NULL,
    region      TEXT NOT NULL DEFAULT 'global',
    status      TEXT NOT NULL DEFAULT 'active',
    version     TEXT,
    last_seen_at TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS connectors_by_tenant ON connectors(tenant_id, created_at);

CREATE TABLE IF NOT EXISTS connector_jobs (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    connector_id TEXT NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    run_id      TEXT REFERENCES workflow_runs(id) ON DELETE CASCADE,
    task_id     TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    payload     TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'queued',
    attempts    BIGINT NOT NULL DEFAULT 0,
    locked_at   TEXT,
    locked_until TEXT,
    result      TEXT,
    error       TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS connector_jobs_run ON connector_jobs(run_id) WHERE run_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS connector_jobs_task ON connector_jobs(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS connector_jobs_claim ON connector_jobs(connector_id, status, created_at);
