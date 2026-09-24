use thiserror::Error;

/// Every failure the protocol core can report. Messages are user-facing and
/// never contain key material.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BandError {
    #[error("{0}")]
    Protocol(String),
    /// The band rejected our stored owner identity (kinesis `BandIdentityMismatchError`).
    #[error("{0}")]
    IdentityMismatch(String),
}

pub type Result<T> = std::result::Result<T, BandError>;

pub(crate) fn perr(message: impl Into<String>) -> BandError {
    BandError::Protocol(message.into())
}
