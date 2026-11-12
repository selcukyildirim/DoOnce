use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("security error: {0}")]
    Security(#[from] doonce_security::SecurityError),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid data: {0}")]
    Invalid(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = error
            && db.is_unique_violation()
        {
            return StoreError::Conflict(db.message().to_string());
        }
        StoreError::Sql(error)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;
