//! The error surface shared by [`Session`](crate::Session) operations.

use rb_device::{DeviceError, DeviceId};
use rb_transport::DriverError;

/// Something went wrong while managing devices or acquisition in a
/// [`Session`](crate::Session).
///
/// Per the per-device isolation rule, a failure here ends only the affected
/// device's acquisition; the session and its other devices keep running.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// No device with the given identifier is registered in the session.
    #[error("no device with id {0}")]
    NotFound(DeviceId),

    /// No registered driver matches the requested name.
    #[error("no driver named {0}")]
    UnknownDriver(String),

    /// The device exposes no capability that can stream samples.
    #[error("device exposes no acquirable capability")]
    NotAcquirable,

    /// The device or one of its capabilities failed.
    #[error(transparent)]
    Device(#[from] DeviceError),

    /// A driver scan or connect failed.
    #[error(transparent)]
    Driver(#[from] DriverError),

    /// The acquisition task panicked and was isolated at the task boundary; the
    /// process/tab survived but this device's capture was lost.
    #[error("acquisition task panicked")]
    AcquisitionPanicked,

    /// The acquisition task has already finished, so the command was not
    /// delivered.
    #[error("acquisition task is no longer running")]
    TaskClosed,
}
