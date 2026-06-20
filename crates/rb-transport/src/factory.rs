//! Driver discovery and connection: the [`DriverFactory`] contract.
//!
//! A driver factory knows how to find and connect one family of devices. The
//! two steps are deliberately distinct (see the ubiquitous language):
//!
//! - **Scan** asks the factory which devices are reachable. The *candidate
//!   source* is platform-supplied — native actively enumerates the bus, while
//!   the web returns only devices the user has already authorised via a browser
//!   picker. The identification logic (VID/PID, `*IDN?`, ...) is shared.
//! - **Connect** binds the factory to one already-identified [`DeviceCandidate`],
//!   producing a live [`Device`].

use async_trait::async_trait;

use rb_device::{Device, DeviceClass, DeviceError, DeviceInfo};

use crate::error::TransportError;

/// A reachable-but-not-yet-connected device found by [`DriverFactory::scan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCandidate {
    /// Best-effort identity gathered during the scan.
    pub info: DeviceInfo,
    /// Opaque, driver-specific address used by [`DriverFactory::connect`] to
    /// reach this exact device (e.g. a USB bus/address, a serial port path, or a
    /// URL). Treated as a token by everything above the driver.
    pub address: String,
}

impl DeviceCandidate {
    /// Creates a candidate from its identity and connection address.
    #[must_use]
    pub fn new(info: DeviceInfo, address: impl Into<String>) -> Self {
        Self {
            info,
            address: address.into(),
        }
    }
}

/// A failure during [`scan`](DriverFactory::scan) or
/// [`connect`](DriverFactory::connect).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DriverError {
    /// The underlying transport failed (e.g. the bus could not be enumerated).
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// The device was reachable but rejected the connection or misbehaved.
    #[error(transparent)]
    Device(#[from] DeviceError),

    /// No device matched the supplied candidate (e.g. it was unplugged).
    #[error("no device matched the candidate")]
    NotFound,
}

/// Convenience alias for fallible driver operations.
pub type DriverResult<T> = Result<T, DriverError>;

/// Discovers and connects one family of devices.
///
/// Registered factories form the explicit, statically-linked driver registry
/// (assembled in `rb-core` behind `#[cfg(feature)]`). A factory whose required
/// transport does not exist on the current platform is simply not registered
/// there, so it never appears in a scan.
#[async_trait(?Send)]
pub trait DriverFactory {
    /// Short, stable name of this driver (e.g. `"fx2lafw"`).
    fn name(&self) -> &str;

    /// The device classes devices from this driver can expose.
    fn supported_classes(&self) -> &[DeviceClass];

    /// Enumerates reachable candidate devices.
    ///
    /// The candidate source is platform-supplied (active enumeration on native,
    /// user-authorised devices on web).
    async fn scan(&self) -> DriverResult<Vec<DeviceCandidate>>;

    /// Connects to one already-identified candidate, producing a live device.
    async fn connect(&self, candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>>;
}
