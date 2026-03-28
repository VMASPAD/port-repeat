use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Frame too large: {0} bytes")]
    FrameTooLarge(u32),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Authentication failed")]
    AuthFailed,
}

pub type Result<T> = std::result::Result<T, ProtoError>;
