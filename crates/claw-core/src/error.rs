use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClawError {
    #[error("unauthorized user")]
    Unauthorized,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}
