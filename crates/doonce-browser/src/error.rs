use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser is disconnected")]
    Disconnected,
    #[error("browser timed out: {0}")]
    Timeout(String),
    #[error("browser error: {0}")]
    Browser(String),
    #[error("transient browser error: {0}")]
    Transient(String),
    #[error("CDP protocol error: {0}")]
    Protocol(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("invalid CDP endpoint: {0}")]
    Endpoint(String),
}

impl BrowserError {
    pub fn is_disconnected(&self) -> bool {
        matches!(self, BrowserError::Disconnected)
    }
}

pub type Result<T> = std::result::Result<T, BrowserError>;
