//! Tenant-scoped operational metrics for dashboards and Prometheus.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;

use crate::{Database, Result};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub workflows: i64,
    pub tasks: i64,
    pub runs: i64,
    pub runs_completed: i64,
    pub runs_failed: i64,
    pub runs_cancelled: i64,
    pub runs_running: i64,
    pub runs_queued: i64,
    pub tasks_completed: i64,
    pub tasks_failed: i64,
    pub tasks_cancelled: i64,
    pub tasks_running: i64,
    pub tasks_queued: i64,
    pub pending_approvals: i64,
    pub webhooks: i64,
    pub webhook_deliveries: i64,
    pub artifacts: i64,
    pub avg_run_duration_ms: Option<f64>,
    pub avg_task_duration_ms: Option<f64>,
    pub success_rate: f64,
}

pub async fn snapshot(db: &Database, tenant_id: &str) -> Result<Metrics> {
    let mut metrics = Metrics {
        workflows: count(db, tenant_id, "workflows").await?,
        tasks: count(db, tenant_id, "tasks").await?,
        runs: count(db, tenant_id, "workflow_runs").await?,
        webhooks: count(db, tenant_id, "webhooks").await?,
        webhook_deliveries: count(db, tenant_id, "webhook_deliveries").await?,
        artifacts: count(db, tenant_id, "artifacts").await?,
        pending_approvals: count_where(db, tenant_id, "approvals", "status = 'pending'").await?,
        ..Metrics::default()
    };

    let rows = sqlx::query(&db.sql(
        "SELECT status, COUNT(*) AS n FROM workflow_runs WHERE tenant_id = ? GROUP BY status",
    ))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    for row in rows {
        let status: String = row.try_get("status")?;
        let n: i64 = row.try_get("n")?;
        match status.as_str() {
            "completed" => metrics.runs_completed = n,
            "failed" => metrics.runs_failed = n,
            "cancelled" => metrics.runs_cancelled = n,
            "running" => metrics.runs_running = n,
            "queued" => metrics.runs_queued = n,
            _ => {}
        }
    }

    let rows = sqlx::query(
        &db.sql("SELECT status, COUNT(*) AS n FROM tasks WHERE tenant_id = ? GROUP BY status"),
    )
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    for row in rows {
        let status: String = row.try_get("status")?;
        let n: i64 = row.try_get("n")?;
        match status.as_str() {
            "completed" => metrics.tasks_completed = n,
            "failed" => metrics.tasks_failed = n,
            "cancelled" => metrics.tasks_cancelled = n,
            "running" => metrics.tasks_running = n,
            "queued" => metrics.tasks_queued = n,
            _ => {}
        }
    }

    metrics.avg_run_duration_ms = average_duration(db, tenant_id, "workflow_runs").await?;
    metrics.avg_task_duration_ms = average_duration(db, tenant_id, "tasks").await?;
    let terminal = metrics.runs_completed + metrics.runs_failed;
    metrics.success_rate = if terminal > 0 {
        metrics.runs_completed as f64 / terminal as f64 * 100.0
    } else {
        0.0
    };
    Ok(metrics)
}

async fn count(db: &Database, tenant_id: &str, table: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql(&format!(
        "SELECT COUNT(*) AS n FROM {table} WHERE tenant_id = ?"
    )))
    .bind(tenant_id)
    .fetch_one(db.pool())
    .await?;
    Ok(row.try_get("n")?)
}

async fn count_where(db: &Database, tenant_id: &str, table: &str, clause: &str) -> Result<i64> {
    let row = sqlx::query(&db.sql(&format!(
        "SELECT COUNT(*) AS n FROM {table} WHERE tenant_id = ? AND {clause}"
    )))
    .bind(tenant_id)
    .fetch_one(db.pool())
    .await?;
    Ok(row.try_get("n")?)
}

async fn average_duration(db: &Database, tenant_id: &str, table: &str) -> Result<Option<f64>> {
    let rows = sqlx::query(&db.sql(&format!(
        "SELECT started_at, finished_at FROM {table}
         WHERE tenant_id = ? AND started_at IS NOT NULL AND finished_at IS NOT NULL
         ORDER BY id DESC LIMIT 100"
    )))
    .bind(tenant_id)
    .fetch_all(db.pool())
    .await?;
    let mut total = 0i64;
    let mut count = 0i64;
    for row in rows {
        let started: String = row.try_get("started_at")?;
        let finished: String = row.try_get("finished_at")?;
        if let (Ok(start), Ok(end)) = (
            DateTime::parse_from_rfc3339(&started),
            DateTime::parse_from_rfc3339(&finished),
        ) {
            let duration = end.with_timezone(&Utc) - start.with_timezone(&Utc);
            total += duration.num_milliseconds().max(0);
            count += 1;
        }
    }
    Ok((count > 0).then(|| total as f64 / count as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn computes_counts_and_success_rate() {
        let db = Database::open_in_memory().await.unwrap();
        let tenant = crate::repo::create_tenant(&db, "Test", "test")
            .await
            .unwrap();
        let metrics = snapshot(&db, &tenant.id).await.unwrap();
        assert_eq!(metrics.workflows, 0);
        assert_eq!(metrics.success_rate, 0.0);
    }
}
