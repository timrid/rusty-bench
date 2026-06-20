//! The [`Session`]: lifecycle owner and registry of connected devices.

use std::collections::HashMap;

use rb_device::{Device, DeviceId};

use crate::handle::DeviceHandle;

/// Owns every connected device in a running RustyBench instance.
///
/// The session maps each [`DeviceId`] to its [`DeviceHandle`] (the device plus
/// its stores and acquisition state). CLI and GUI front-ends talk to the session
/// to list devices, drive acquisition and read samples. Devices are independent:
/// there is no cross-device bus, and a fault on one never disturbs another.
#[derive(Default)]
pub struct Session {
    devices: HashMap<DeviceId, DeviceHandle>,
}

impl Session {
    /// Creates an empty session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a connected device, wrapping it in a [`DeviceHandle`], and
    /// returns its identifier. An existing device with the same id is replaced.
    pub fn add_device(&mut self, device: Box<dyn Device>) -> DeviceId {
        let handle = DeviceHandle::new(device);
        let id = handle.id().clone();
        self.devices.insert(id.clone(), handle);
        id
    }

    /// Removes and returns a device's handle, if present.
    pub fn remove(&mut self, id: &DeviceId) -> Option<DeviceHandle> {
        self.devices.remove(id)
    }

    /// Read-only access to a device's handle.
    pub fn device(&self, id: &DeviceId) -> Option<&DeviceHandle> {
        self.devices.get(id)
    }

    /// Mutable access to a device's handle (to drive acquisition).
    pub fn device_mut(&mut self, id: &DeviceId) -> Option<&mut DeviceHandle> {
        self.devices.get_mut(id)
    }

    /// The identifiers of every registered device.
    #[must_use]
    pub fn device_ids(&self) -> Vec<DeviceId> {
        self.devices.keys().cloned().collect()
    }

    /// Number of registered devices.
    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether the session has no devices.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Pumps every running device once, returning the total samples appended.
    pub fn pump_all(&mut self, max_samples: usize) -> usize {
        self.devices
            .values_mut()
            .map(|handle| handle.pump(max_samples))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use rb_drivers::demo::{DemoConfig, DemoDevice};

    fn add_demo(session: &mut Session, id: &str) -> DeviceId {
        let device = DemoDevice::new(DeviceId::new(id), DemoConfig::default());
        session.add_device(Box::new(device))
    }

    #[test]
    fn add_and_remove_devices() {
        let mut session = Session::new();
        assert!(session.is_empty());
        let id = add_demo(&mut session, "demo:0");
        assert_eq!(session.len(), 1);
        assert!(session.device(&id).is_some());
        assert_eq!(session.device_ids(), vec![id.clone()]);
        assert!(session.remove(&id).is_some());
        assert!(session.is_empty());
    }

    #[test]
    fn pump_all_only_advances_running_devices() {
        let mut session = Session::new();
        let running = add_demo(&mut session, "demo:0");
        let _idle = add_demo(&mut session, "demo:1");

        block_on(session.device_mut(&running).unwrap().start()).unwrap();
        let appended = session.pump_all(32);
        assert_eq!(appended, 32);
        assert_eq!(session.device(&running).unwrap().sample_count(), 32);
    }
}
