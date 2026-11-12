use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("AI provider is not configured: {0}")]
    NotConfigured(String),
    #[error("AI request failed: {0}")]
    Request(String),
    #[error("AI provider returned HTTP {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("AI provider returned an invalid response: {0}")]
    Response(String),
    #[error("the configured model does not support images")]
    VisionUnsupported,
}

pub type Result<T> = std::result::Result<T, LlmError>;
