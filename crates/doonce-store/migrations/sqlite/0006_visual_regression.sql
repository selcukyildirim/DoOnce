CREATE TABLE IF NOT EXISTS run_diffs (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    run_id          TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    baseline_run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    pixel_diff_pct  REAL NOT NULL DEFAULT 0,
    changed_bytes   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS run_diffs_by_run ON run_diffs(run_id);
CREATE INDEX IF NOT EXISTS run_diffs_by_baseline ON run_diffs(baseline_run_id);
