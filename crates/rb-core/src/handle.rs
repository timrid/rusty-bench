//! [`DeviceHandle`]: an exclusive access token for a connected [`Device`].

use rb_device::{Device, DeviceId};

/// An exclusive access token for a connected [`Device`].
///
/// Only one consumer (Tab, CLI session) can hold the handle at a time.
/// All device access — acquisition, configuration, firmware updates —
/// requires the handle. The handle carries no sample data and no
/// acquisition state; those are managed by the consumer.
///
/// The handle is obtained from the [`DeviceManager`] via `take_handle()`
/// and must be returned via `return_handle()` when done.
pub struct DeviceHandle {
    id: DeviceId,
    device: Box<dyn Device>,
}

impl DeviceHandle {
    /// Wraps a connected device.
    #[must_use]
    pub fn new(device: Box<dyn Device>) -> Self {
        let id = device.id().clone();
        Self { id, device }
    }

    /// This device's stable identifier.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Read-only access to the wrapped device (identity, capabilities).
    #[must_use]
    pub fn device(&self) -> &dyn Device {
        self.device.as_ref()
    }

    /// Mutable access to the wrapped device (for capability methods like `arm()`).
    #[must_use]
    pub fn device_mut(&mut self) -> &mut dyn Device {
        self.device.as_mut()
    }
}
