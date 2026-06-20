//! Typed transport errors.

/// A failure on a [`Transport`](crate::Transport).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The transport has been closed and can no longer be used.
    #[error("transport is closed")]
    Closed,

    /// The transport is not connected to a peer.
    #[error("transport is not connected")]
    NotConnected,

    /// An operation exceeded its deadline.
    #[error("transport operation timed out")]
    Timeout,

    /// The transport does not support the requested operation.
    #[error("operation not supported by this transport")]
    Unsupported,

    /// A lower-level I/O failure. The string keeps this crate independent of any
    /// concrete platform I/O error type.
    #[error("transport I/O error: {0}")]
    Io(String),
}

/// Convenience alias for fallible transport operations.
pub type TransportResult<T> = Result<T, TransportError>;
