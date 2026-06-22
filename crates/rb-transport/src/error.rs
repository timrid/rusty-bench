//! Typed transport errors.

/// A failure on an [`UsbTransport`](crate::UsbTransport).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The transport has been closed and can no longer be used.
    #[error("transport is closed")]
    Closed,

    /// An operation exceeded its deadline.
    #[error("transport operation timed out")]
    Timeout,

    /// A lower-level I/O failure. The string keeps this crate independent of any
    /// concrete platform I/O error type.
    #[error("transport I/O error: {0}")]
    Io(String),
}

/// Convenience alias for fallible transport operations.
pub type TransportResult<T> = Result<T, TransportError>;
