//! Local SQLite storage: workflows with full revision history, executions
//! and their step results. Only workflow definitions and step outcomes are
//! stored; parameter values are never written to disk.

mod migrations;

use std::path::{Path, PathBuf};
use std::time::Duration;

use doonce_core::{Workflow, parse_workflow, slugify};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not create data directory {0}: {1}")]
    DataDir(PathBuf, std::io::Error),
    #[error("no automation named \"{0}\"")]
    NotFound(String),
    #[error("revision {1} of \"{0}\" does not exist")]
    NoRevision(String, i64),
    #[error("stored workflow \"{0}\" is invalid: {1}")]
    Corrupt(String, String),
    #[error("this database was created by a newer DoOnce (schema {0}); please upgrade")]
    TooNew(i64),
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecord {
    pub id: i64,
    pub slug: String,
    pub version: i64,
    pub workflow: Workflow,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub version: i64,
    pub step_count: usize,
    pub parameter_count: usize,
    pub updated_at: String,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRecord {
    pub version: i64,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecord {
    pub id: i64,
    pub workflow_id: i64,
    pub workflow_revision: i64,
    pub source: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub step_id: String,
    pub status: String,
    pub error: Option<String>,
    pub duration_ms: i64,
}

pub struct Storage {
    conn: Connection,
}

/// `$DOONCE_HOME`, or the platform data directory (e.g.
/// `~/Library/Application Support/DoOnce` on macOS).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("DOONCE_HOME") {
        return PathBuf::from(home);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("DoOnce")
}

impl Storage {
    pub fn open_default() -> Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::DataDir(dir.clone(), e))?;
        Self::open(&dir.join("doonce.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // The CLI and the browser extension's host may use the database at
        // the same time.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > migrations::MIGRATIONS.len() as i64 {
            return Err(StorageError::TooNew(version));
        }
        migrations::migrate(&mut conn)?;
        Ok(Storage { conn })
    }

    /// Stores a new workflow. The slug is derived from its name and made
    /// unique by appending a number.
    pub fn create_workflow(&mut self, workflow: &Workflow, reason: &str) -> Result<WorkflowRecord> {
        let base = slugify(&workflow.name);
        let tx = self.conn.transaction()?;
        let mut slug = base.clone();
        let mut n = 2;
        while tx.query_row("SELECT 1 FROM workflows WHERE slug = ?1", [&slug], |_| Ok(())).optional()?.is_some() {
            slug = format!("{base}-{n}");
            n += 1;
        }
        let definition = workflow.to_pretty_json();
        tx.execute(
            "INSERT INTO workflows (slug, name, schema_version, definition, current_version) VALUES (?1, ?2, ?3, ?4, 1)",
            params![slug, workflow.name, workflow.schema_version, definition],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO workflow_revisions (workflow_id, version, definition, reason) VALUES (?1, 1, ?2, ?3)",
            params![id, definition, reason],
        )?;
        tx.commit()?;
        self.get_workflow(&id.to_string())
    }

    /// Saves a new revision of an existing workflow and makes it current.
    pub fn update_workflow(&mut self, reference: &str, workflow: &Workflow, reason: &str) -> Result<WorkflowRecord> {
        let id = self.resolve_id(reference)?;
        let definition = workflow.to_pretty_json();
        let tx = self.conn.transaction()?;
        let version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM workflow_revisions WHERE workflow_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO workflow_revisions (workflow_id, version, definition, reason) VALUES (?1, ?2, ?3, ?4)",
            params![id, version, definition, reason],
        )?;
        tx.execute(
            "UPDATE workflows SET name = ?2, schema_version = ?3, definition = ?4, current_version = ?5,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![id, workflow.name, workflow.schema_version, definition, version],
        )?;
        tx.commit()?;
        self.get_workflow(&id.to_string())
    }

    /// Makes an older revision current again by copying it as a new revision.
    pub fn rollback(&mut self, reference: &str, version: i64) -> Result<WorkflowRecord> {
        let id = self.resolve_id(reference)?;
        let definition: Option<String> = self
            .conn
            .query_row(
                "SELECT definition FROM workflow_revisions WHERE workflow_id = ?1 AND version = ?2",
                params![id, version],
                |r| r.get(0),
            )
            .optional()?;
        let definition = definition.ok_or_else(|| StorageError::NoRevision(reference.into(), version))?;
        let workflow = parse_stored(reference, &definition)?;
        self.update_workflow(&id.to_string(), &workflow, &format!("Rolled back to version {version}"))
    }

    pub fn revisions(&self, reference: &str) -> Result<Vec<RevisionRecord>> {
        let id = self.resolve_id(reference)?;
        let mut stmt = self.conn.prepare(
            "SELECT version, reason, created_at FROM workflow_revisions WHERE workflow_id = ?1 ORDER BY version DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(RevisionRecord { version: r.get(0)?, reason: r.get(1)?, created_at: r.get(2)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Looks a workflow up by numeric id or slug.
    pub fn get_workflow(&self, reference: &str) -> Result<WorkflowRecord> {
        let id = self.resolve_id(reference)?;
        let (slug, version, definition, created_at, updated_at): (String, i64, String, String, String) =
            self.conn.query_row(
                "SELECT slug, current_version, definition, created_at, updated_at FROM workflows WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        let workflow = parse_stored(&slug, &definition)?;
        Ok(WorkflowRecord { id, slug, version, workflow, created_at, updated_at })
    }

    pub fn list_workflows(&self) -> Result<Vec<WorkflowSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT w.id, w.slug, w.name, w.current_version, w.definition, w.updated_at,
                    e.started_at, e.status
             FROM workflows w
             LEFT JOIN executions e ON e.id = (SELECT MAX(id) FROM executions WHERE workflow_id = w.id)
             ORDER BY COALESCE(e.started_at, w.updated_at) DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let definition: String = r.get(4)?;
            let (step_count, parameter_count) = serde_json::from_str::<serde_json::Value>(&definition)
                .map(|v| {
                    let count = |k: &str| v.get(k).and_then(|s| s.as_array()).map_or(0, Vec::len);
                    (count("steps"), count("parameters"))
                })
                .unwrap_or_default();
            Ok(WorkflowSummary {
                id: r.get(0)?,
                slug: r.get(1)?,
                name: r.get(2)?,
                version: r.get(3)?,
                step_count,
                parameter_count,
                updated_at: r.get(5)?,
                last_run_at: r.get(6)?,
                last_status: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_workflow(&mut self, reference: &str) -> Result<()> {
        let id = self.resolve_id(reference)?;
        self.conn.execute("DELETE FROM workflows WHERE id = ?1", [id])?;
        Ok(())
    }

    fn resolve_id(&self, reference: &str) -> Result<i64> {
        let by_id = reference.parse::<i64>().ok();
        self.conn
            .query_row(
                "SELECT id FROM workflows WHERE slug = ?1 OR id = ?2 ORDER BY slug = ?1 DESC LIMIT 1",
                params![reference, by_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(reference.into()))
    }

    // ---- executions -------------------------------------------------------

    pub fn start_execution(&mut self, workflow_id: i64, revision: i64, source: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO executions (workflow_id, workflow_revision, source, status) VALUES (?1, ?2, ?3, 'running')",
            params![workflow_id, revision, source],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records the outcome of one step. Errors must already be free of
    /// sensitive values (the engine guarantees this).
    pub fn record_step(
        &mut self,
        execution_id: i64,
        step_id: &str,
        status: &str,
        error: Option<&str>,
        duration_ms: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO execution_steps (execution_id, step_id, status, error, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![execution_id, step_id, status, error, duration_ms],
        )?;
        Ok(())
    }

    pub fn finish_execution(&mut self, execution_id: i64, status: &str, error: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET status = ?2, error = ?3, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![execution_id, status, error],
        )?;
        Ok(())
    }

    /// Marks executions left `running` by a crashed process as interrupted.
    pub fn mark_interrupted(&mut self, older_than_minutes: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE executions SET status = 'cancelled', error = 'interrupted',
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE status = 'running' AND started_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [format!("-{older_than_minutes} minutes")],
        )?)
    }

    pub fn history(&self, reference: &str, limit: usize) -> Result<Vec<ExecutionRecord>> {
        let id = self.resolve_id(reference)?;
        let mut stmt = self.conn.prepare(
            "SELECT id, workflow_id, workflow_revision, source, started_at, finished_at, status, error
             FROM executions WHERE workflow_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![id, limit as i64], |r| {
            Ok(ExecutionRecord {
                id: r.get(0)?,
                workflow_id: r.get(1)?,
                workflow_revision: r.get(2)?,
                source: r.get(3)?,
                started_at: r.get(4)?,
                finished_at: r.get(5)?,
                status: r.get(6)?,
                error: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn execution_steps(&self, execution_id: i64) -> Result<Vec<StepRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT step_id, status, error, duration_ms FROM execution_steps WHERE execution_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([execution_id], |r| {
            Ok(StepRecord { step_id: r.get(0)?, status: r.get(1)?, error: r.get(2)?, duration_ms: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- settings ---------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn parse_stored(name: &str, definition: &str) -> Result<Workflow> {
    parse_workflow(definition).map_err(|issues| {
        StorageError::Corrupt(name.into(), issues.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use doonce_core::{Action, SCHEMA_VERSION, Step};

    fn workflow(name: &str, url: &str) -> Workflow {
        Workflow {
            schema_version: SCHEMA_VERSION.into(),
            name: name.into(),
            description: String::new(),
            parameters: vec![],
            steps: vec![Step {
                id: "step-1".into(),
                action: Action::Navigate { url: url.into() },
                description: None,
                timeout_ms: None,
                retry: None,
                requires_approval: None,
            }],
        }
    }

    #[test]
    fn workflows_revisions_and_rollback() {
        let mut s = Storage::open_in_memory().unwrap();
        let a = s.create_workflow(&workflow("Daily Report", "https://a.test/"), "Recorded").unwrap();
        assert_eq!(a.slug, "daily-report");
        assert_eq!(a.version, 1);
        let b = s.create_workflow(&workflow("Daily Report", "https://b.test/"), "Imported").unwrap();
        assert_eq!(b.slug, "daily-report-2");

        let updated = s.update_workflow("daily-report", &workflow("Daily Report", "https://c.test/"), "Edited").unwrap();
        assert_eq!(updated.version, 2);
        let rolled = s.rollback("daily-report", 1).unwrap();
        assert_eq!(rolled.version, 3);
        assert_eq!(rolled.workflow.steps[0].action, Action::Navigate { url: "https://a.test/".into() });
        let revs = s.revisions(&a.id.to_string()).unwrap();
        assert_eq!(revs.iter().map(|r| r.version).collect::<Vec<_>>(), vec![3, 2, 1]);
        assert!(matches!(s.rollback("daily-report", 99), Err(StorageError::NoRevision(..))));

        assert_eq!(s.list_workflows().unwrap().len(), 2);
        s.delete_workflow("daily-report-2").unwrap();
        assert!(matches!(s.get_workflow("daily-report-2"), Err(StorageError::NotFound(_))));
    }

    #[test]
    fn executions_and_history() {
        let mut s = Storage::open_in_memory().unwrap();
        let w = s.create_workflow(&workflow("Run me", "https://a.test/"), "Recorded").unwrap();
        let e = s.start_execution(w.id, w.version, "cli").unwrap();
        s.record_step(e, "step-1", "completed", None, 12).unwrap();
        s.finish_execution(e, "completed", None).unwrap();
        let summary = &s.list_workflows().unwrap()[0];
        assert_eq!(summary.last_status.as_deref(), Some("completed"));
        assert_eq!(summary.step_count, 1);
        let history = s.history("run-me", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].finished_at.is_some());
        assert_eq!(s.execution_steps(e).unwrap()[0].duration_ms, 12);
    }

    #[test]
    fn migrations_are_idempotent_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let mut s = Storage::open(&path).unwrap();
            s.set_setting("ai.provider", "claude").unwrap();
        }
        let s = Storage::open(&path).unwrap();
        assert_eq!(s.get_setting("ai.provider").unwrap().as_deref(), Some("claude"));
    }
}
