use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use doonce_store::StoreError;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
#[error("{title}: {detail}")]
pub struct ApiError {
    pub status: StatusCode,
    pub title: &'static str,
    pub detail: String,
}

impl ApiError {
    pub fn unauthorized(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::UNAUTHORIZED,
            title: "Unauthorized",
            detail: detail.into(),
        }
    }

    pub fn forbidden(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::FORBIDDEN,
            title: "Forbidden",
            detail: detail.into(),
        }
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::NOT_FOUND,
            title: "Not Found",
            detail: detail.into(),
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::BAD_REQUEST,
            title: "Bad Request",
            detail: detail.into(),
        }
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::CONFLICT,
            title: "Conflict",
            detail: detail.into(),
        }
    }

    pub fn service_unavailable(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            title: "Service Unavailable",
            detail: detail.into(),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            title: "Internal Server Error",
            detail: detail.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "type": "about:blank",
            "title": self.title,
            "status": self.status.as_u16(),
            "detail": self.detail,
        });
        (
            self.status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound(detail) => ApiError::not_found(detail),
            StoreError::Conflict(detail) => ApiError::conflict(detail),
            StoreError::Invalid(detail) => ApiError::bad_request(detail),
            StoreError::Security(error) => {
                tracing::error!(error = %error, "security error while serving an API request");
                ApiError::internal("a security operation failed")
            }
            StoreError::Json(error) => {
                tracing::error!(error = %error, "json error while serving an API request");
                ApiError::internal("stored data is malformed")
            }
            StoreError::Sql(error) => {
                tracing::error!(error = %error, "database error while serving an API request");
                ApiError::internal("database operation failed")
            }
            StoreError::Migrate(error) => {
                tracing::error!(error = %error, "migration error");
                ApiError::internal("database migration failed")
            }
        }
    }
}

impl From<doonce_security::SecurityError> for ApiError {
    fn from(error: doonce_security::SecurityError) -> Self {
        tracing::error!(error = %error, "security error while serving an API request");
        ApiError::internal("a security operation failed")
    }
}
