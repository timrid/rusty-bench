//! Typed device errors.
//!
//! Per the project error policy, library crates surface typed [`thiserror`]
//! enums; only the binaries reach for `anyhow`. A failing device is expected to
//! be isolated by the runtime (it enters an error/disconnected state) rather
//! than taking down the whole session.

/// Something went wrong while talking to or controlling a device.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceError {
    /// The device is not currently connected/open.
    #[error("device is not connected")]
    NotConnected,

    /// The requested operation is not supported by this device.
    #[error("operation not supported: {0}")]
    Unsupported(String),

    /// A parameter was outside the device's accepted range or format.
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),

    /// The device reported a protocol- or command-level failure.
    #[error("device protocol error: {0}")]
    Protocol(String),

    /// The underlying transport failed. The string keeps `rb-device` free of a
    /// dependency on any concrete transport error type.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// Convenience alias for fallible device operations.
pub type DeviceResult<T> = Result<T, DeviceError>;
