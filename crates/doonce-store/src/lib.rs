//! Portable persistence layer.
//!
//! SQLite is the default for local and single-node deployments; Postgres is
//! supported through the same SQLx `Any` pool. Migrations are dialect-specific
//! but repositories speak one placeholder dialect (`?`), which is rewritten to
//! `$n` for Postgres. Timestamps and JSON are stored as text so the exact same
//! query works on both engines.

#![forbid(unsafe_code)]

pub mod cron;
pub mod error;
pub mod metrics;
pub mod repo;

use std::time::Duration;

use sqlx::AnyPool;
use sqlx::any::{AnyPoolOptions, install_default_drivers};
use sqlx::migrate::Migrator;

pub use error::{Result, StoreError};

static SQLITE_MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");
static POSTGRES_MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub fn from_url(url: &str) -> Self {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Dialect::Postgres
        } else {
            Dialect::Sqlite
        }
    }
}

#[derive(Clone)]
pub struct Database {
    pool: AnyPool,
    dialect: Dialect,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self> {
        install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(10)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .idle_timeout(Duration::from_secs(300))
            .connect(url)
            .await?;
        Ok(Database {
            pool,
            dialect: Dialect::from_url(url),
        })
    }

    /// Opens an in-memory SQLite database for tests. A single connection keeps
    /// the in-memory database alive for the lifetime of the pool.
    pub async fn open_in_memory() -> Result<Self> {
        install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let db = Database {
            pool,
            dialect: Dialect::Sqlite,
        };
        db.migrate().await?;
        Ok(db)
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        match self.dialect {
            Dialect::Sqlite => SQLITE_MIGRATOR.run(&self.pool).await?,
            Dialect::Postgres => POSTGRES_MIGRATOR.run(&self.pool).await?,
        }
        Ok(())
    }

    /// Converts the single `?` placeholder dialect used by repositories into
    /// the target database's placeholder syntax.
    pub fn sql(&self, query: &str) -> String {
        rewrite_placeholders(query, self.dialect)
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}

fn rewrite_placeholders(query: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::Sqlite => query.to_string(),
        Dialect::Postgres => {
            let mut out = String::with_capacity(query.len() + 16);
            let mut index = 1u32;
            for c in query.chars() {
                if c == '?' {
                    out.push('$');
                    out.push_str(&index.to_string());
                    index += 1;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_migrations_and_ping() {
        let db = Database::open_in_memory().await.unwrap();
        db.ping().await.unwrap();
        assert_eq!(db.dialect(), Dialect::Sqlite);
    }

    #[test]
    fn placeholder_rewrite() {
        assert_eq!(
            rewrite_placeholders("SELECT * FROM t WHERE a = ? AND b = ?", Dialect::Postgres),
            "SELECT * FROM t WHERE a = $1 AND b = $2"
        );
        assert_eq!(
            rewrite_placeholders("SELECT * FROM t WHERE a = ?", Dialect::Sqlite),
            "SELECT * FROM t WHERE a = ?"
        );
    }
}
