use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("cryptographic operation failed: {0}")]
    Crypto(String),
    #[error("invalid key material: {0}")]
    Key(String),
    #[error("invalid encoding: {0}")]
    Encoding(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("hashing failed: {0}")]
    Hash(String),
    #[error("value is too large: {0}")]
    TooLarge(String),
}

pub type Result<T> = std::result::Result<T, SecurityError>;
