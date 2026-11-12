-- DoOnce enterprise schema (Postgres baseline).
-- Portable TEXT/BIGINT columns keep repository code identical to SQLite.
-- Production deployments may add native UUID/JSONB/TIMESTAMPTZ columns later.

CREATE TABLE IF NOT EXISTS tenants (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    role         TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (tenant_id, email)
);
CREATE INDEX IF NOT EXISTS users_by_tenant ON users(tenant_id);

CREATE TABLE IF NOT EXISTS api_keys (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id      TEXT REFERENCES users(id) ON DELETE SET NULL,
    name         TEXT NOT NULL,
    prefix       TEXT NOT NULL UNIQUE,
    key_hash     TEXT NOT NULL,
    scopes       TEXT NOT NULL DEFAULT '[]',
    last_used_at TEXT,
    expires_at   TEXT,
    revoked_at   TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS api_keys_by_tenant ON api_keys(tenant_id);

CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    slug            TEXT NOT NULL,
    name            TEXT NOT NULL,
    current_version BIGINT NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (tenant_id, slug)
);
CREATE INDEX IF NOT EXISTS workflows_by_tenant ON workflows(tenant_id);

CREATE TABLE IF NOT EXISTS workflow_versions (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version     BIGINT NOT NULL,
    definition  TEXT NOT NULL,
    reason      TEXT NOT NULL DEFAULT '',
    created_by  TEXT,
    created_at  TEXT NOT NULL,
    UNIQUE (workflow_id, version)
);
CREATE INDEX IF NOT EXISTS workflow_versions_by_workflow ON workflow_versions(workflow_id, version DESC);

CREATE TABLE IF NOT EXISTS tasks (
    id                 TEXT PRIMARY KEY,
    tenant_id          TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    created_by         TEXT,
    prompt             TEXT NOT NULL,
    url                TEXT,
    navigation_goal    TEXT,
    data_schema        TEXT,
    error_codes        TEXT NOT NULL DEFAULT '[]',
    status             TEXT NOT NULL DEFAULT 'queued',
    output             TEXT,
    failure_reason     TEXT,
    model              TEXT,
    max_steps          BIGINT NOT NULL DEFAULT 30,
    browser_session_id TEXT,
    created_at         TEXT NOT NULL,
    started_at         TEXT,
    finished_at        TEXT
);
CREATE INDEX IF NOT EXISTS tasks_by_tenant ON tasks(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS tasks_by_status ON tasks(status);

CREATE TABLE IF NOT EXISTS task_actions (
    id                     TEXT PRIMARY KEY,
    tenant_id              TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    task_id                TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    seq                    BIGINT NOT NULL,
    action                 TEXT NOT NULL,
    reasoning              TEXT,
    screenshot_artifact_id TEXT,
    status                 TEXT NOT NULL DEFAULT 'recorded',
    created_at             TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS task_actions_by_task ON task_actions(task_id, seq);

CREATE TABLE IF NOT EXISTS workflow_runs (
    id               TEXT PRIMARY KEY,
    tenant_id        TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    workflow_id      TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    workflow_version BIGINT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'queued',
    inputs           TEXT NOT NULL DEFAULT '{}',
    outputs          TEXT,
    error            TEXT,
    trigger          TEXT NOT NULL DEFAULT 'manual',
    parent_run_id    TEXT,
    created_by       TEXT,
    created_at       TEXT NOT NULL,
    started_at       TEXT,
    finished_at      TEXT
);
CREATE INDEX IF NOT EXISTS workflow_runs_by_workflow ON workflow_runs(workflow_id, created_at DESC);
CREATE INDEX IF NOT EXISTS workflow_runs_by_tenant ON workflow_runs(tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS workflow_block_runs (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    run_id      TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    block_id    TEXT NOT NULL,
    seq         BIGINT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    input       TEXT,
    output      TEXT,
    error       TEXT,
    attempt     BIGINT NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS workflow_block_runs_by_run ON workflow_block_runs(run_id, seq);

CREATE TABLE IF NOT EXISTS artifacts (
    id           TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    run_id       TEXT REFERENCES workflow_runs(id) ON DELETE CASCADE,
    task_id      TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    block_run_id TEXT REFERENCES workflow_block_runs(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    filename     TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    size         BIGINT NOT NULL DEFAULT 0,
    storage_key  TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS artifacts_by_run ON artifacts(run_id);
CREATE INDEX IF NOT EXISTS artifacts_by_task ON artifacts(task_id);
CREATE INDEX IF NOT EXISTS artifacts_by_tenant ON artifacts(tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS credentials (
    id                    TEXT PRIMARY KEY,
    tenant_id             TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    kind                  TEXT NOT NULL,
    username              TEXT,
    secret_encrypted      TEXT NOT NULL,
    totp_secret_encrypted TEXT,
    metadata              TEXT NOT NULL DEFAULT '{}',
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS credentials_by_tenant ON credentials(tenant_id);

CREATE TABLE IF NOT EXISTS browser_sessions (
    id            TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'opening',
    user_data_dir TEXT,
    cdp_url       TEXT,
    proxy_url     TEXT,
    created_at    TEXT NOT NULL,
    closed_at     TEXT
);
CREATE INDEX IF NOT EXISTS browser_sessions_by_tenant ON browser_sessions(tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS schedules (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    cron        TEXT NOT NULL,
    timezone    TEXT NOT NULL DEFAULT 'UTC',
    enabled     INTEGER NOT NULL DEFAULT 1,
    next_run_at TEXT,
    last_run_at TEXT,
    last_status TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS schedules_by_next_run ON schedules(enabled, next_run_at);

CREATE TABLE IF NOT EXISTS webhooks (
    id               TEXT PRIMARY KEY,
    tenant_id        TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    url              TEXT NOT NULL,
    secret_encrypted TEXT NOT NULL,
    events           TEXT NOT NULL DEFAULT '[]',
    enabled          INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhooks_by_tenant ON webhooks(tenant_id);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id            TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    webhook_id    TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event         TEXT NOT NULL,
    payload       TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending',
    attempts      BIGINT NOT NULL DEFAULT 0,
    response_code BIGINT,
    last_error    TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhook_deliveries_by_webhook ON webhook_deliveries(webhook_id, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_configs (
    id                TEXT PRIMARY KEY,
    tenant_id         TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    provider          TEXT NOT NULL,
    model             TEXT NOT NULL,
    base_url          TEXT,
    api_key_encrypted TEXT,
    capabilities      TEXT NOT NULL DEFAULT '{}',
    is_default        INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS llm_configs_by_tenant ON llm_configs(tenant_id);

CREATE TABLE IF NOT EXISTS settings (
    tenant_id  TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, key)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id         TEXT PRIMARY KEY,
    tenant_id  TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    actor_id   TEXT,
    action     TEXT NOT NULL,
    target     TEXT NOT NULL,
    metadata   TEXT NOT NULL DEFAULT '{}',
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS audit_logs_by_tenant ON audit_logs(tenant_id, id);

CREATE TABLE IF NOT EXISTS run_cache (
    tenant_id  TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    cache_key  TEXT NOT NULL,
    value      TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, cache_key)
);
