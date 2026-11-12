ALTER TABLE workflow_runs ADD COLUMN locked_at TEXT;
ALTER TABLE workflow_runs ADD COLUMN locked_by TEXT;
ALTER TABLE workflow_runs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN locked_at TEXT;
ALTER TABLE tasks ADD COLUMN locked_by TEXT;
ALTER TABLE tasks ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS approvals (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    run_id      TEXT REFERENCES workflow_runs(id) ON DELETE CASCADE,
    task_id     TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    step_id     TEXT NOT NULL,
    summary     TEXT NOT NULL,
    reason      TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    created_at  TEXT NOT NULL,
    resolved_at TEXT,
    resolved_by TEXT
);
CREATE INDEX IF NOT EXISTS approvals_by_run ON approvals(run_id);
CREATE INDEX IF NOT EXISTS approvals_by_task ON approvals(task_id);
CREATE INDEX IF NOT EXISTS approvals_pending ON approvals(status, created_at);
