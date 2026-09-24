use thiserror::Error;

/// Every failure the protocol core can report. Messages are user-facing and
/// never contain key material.
///
/// `Protocol` / `IdentityMismatch` returned from `BandSession::feed` end the
/// session. `Busy` means a request was refused and the session is fine.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BandError {
    #[error("{0}")]
    Protocol(String),
    /// The band rejected our stored owner identity (kinesis `BandIdentityMismatchError`).
    #[error("{0}")]
    IdentityMismatch(String),
    /// A request was refused because the session isn't ready or another
    /// change is in flight; the session itself is unaffected.
    #[error("{0}")]
    Busy(String),
}

pub type Result<T> = std::result::Result<T, BandError>;

pub(crate) fn perr(message: impl Into<String>) -> BandError {
    BandError::Protocol(message.into())
}
