use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Connection closed")]
    ConnectionClosed,
}

pub type Result<T> = std::result::Result<T, ProtoError>;
