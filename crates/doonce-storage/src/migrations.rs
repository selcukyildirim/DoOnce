//! Ordered schema migrations tracked with SQLite's `user_version`. Never
//! edit a released migration; append a new one instead.

use rusqlite::Connection;

pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE workflows (
        id              INTEGER PRIMARY KEY,
        slug            TEXT NOT NULL UNIQUE,
        name            TEXT NOT NULL,
        schema_version  TEXT NOT NULL,
        definition      TEXT NOT NULL,
        current_version INTEGER NOT NULL,
        created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );

    CREATE TABLE workflow_revisions (
        id          INTEGER PRIMARY KEY,
        workflow_id INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
        version     INTEGER NOT NULL,
        definition  TEXT NOT NULL,
        reason      TEXT NOT NULL DEFAULT '',
        created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        UNIQUE (workflow_id, version)
    );

    CREATE TABLE executions (
        id                INTEGER PRIMARY KEY,
        workflow_id       INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
        workflow_revision INTEGER NOT NULL,
        source            TEXT NOT NULL,
        started_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        finished_at       TEXT,
        status            TEXT NOT NULL,
        error             TEXT
    );
    CREATE INDEX executions_by_workflow ON executions(workflow_id, id DESC);

    CREATE TABLE execution_steps (
        id           INTEGER PRIMARY KEY,
        execution_id INTEGER NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
        step_id      TEXT NOT NULL,
        status       TEXT NOT NULL,
        error        TEXT,
        duration_ms  INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX execution_steps_by_execution ON execution_steps(execution_id);

    CREATE TABLE repair_attempts (
        id           INTEGER PRIMARY KEY,
        execution_id INTEGER NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
        step_id      TEXT NOT NULL,
        provider     TEXT NOT NULL,
        proposal     TEXT NOT NULL,
        result       TEXT NOT NULL,
        created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
];

pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
