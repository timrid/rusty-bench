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

    /// Inserts a pre-built [`DeviceHandle`] under the given id. An existing
    /// device with the same id is replaced.
    pub fn add_device_from_handle(&mut self, id: DeviceId, handle: DeviceHandle) -> DeviceId {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::task::LocalSpawnExt;
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
    fn start_streaming_fills_store() {
        let mut session = Session::new();
        let id = add_demo(&mut session, "demo:0");

        let handle = session.device_mut(&id).unwrap();
        let (read_loop, mut data_rx) = block_on(handle.start_streaming()).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(data_rx.next()).unwrap();
        handle.ingest_chunk(&chunk);

        drop(data_rx);
        pool.run_until_stalled();

        let handle = session.device(&id).unwrap();
        assert!(handle.sample_count() > 0);
    }
}
