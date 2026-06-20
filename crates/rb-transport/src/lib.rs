//! Transport abstraction for RustyBench.
//!
//! - [`Transport`] — the byte/packet-oriented link a driver speaks over,
//!   described by [`TransportCapabilities`] / [`TransportKind`].
//! - [`MockTransport`] — the in-memory test backbone: replays queued reads,
//!   captures writes.
//! - [`DriverFactory`] — the scan/connect contract that turns a reachable
//!   [`DeviceCandidate`] into a live [`Device`](rb_device::Device).
//!
//! Concrete platform implementations (USB/Serial/Bluetooth/Ethernet on native,
//! WebUSB/WebSerial/WebBluetooth on web) are added behind Cargo features in
//! later milestones. The default build is pure and wasm-safe, so the mock and
//! the trait surface compile to `wasm32-unknown-unknown` with no features.

#![forbid(unsafe_code)]

mod error;
mod factory;
mod mock;
mod transport;

pub use error::{TransportError, TransportResult};
pub use factory::{DeviceCandidate, DriverError, DriverFactory, DriverResult};
pub use mock::MockTransport;
pub use transport::{Transport, TransportCapabilities, TransportKind};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::executor::block_on;
    use rb_device::{Device, DeviceClass, DeviceId, DeviceInfo};

    /// A device whose state is read back over a [`Transport`], used to show a
    /// driver talking to hardware purely through [`MockTransport`].
    struct EchoDevice {
        id: DeviceId,
        info: DeviceInfo,
        transport: MockTransport,
    }

    #[async_trait(?Send)]
    impl Device for EchoDevice {
        fn id(&self) -> &DeviceId {
            &self.id
        }
        fn info(&self) -> &DeviceInfo {
            &self.info
        }
        async fn open(&mut self) -> rb_device::DeviceResult<()> {
            // A real driver would handshake here; we just send an identify
            // command and confirm the canned reply.
            self.transport
                .write(b"*IDN?\n")
                .await
                .map_err(|e| rb_device::DeviceError::Transport(e.to_string()))?;
            let mut buf = [0u8; 16];
            let n = self
                .transport
                .read(&mut buf)
                .await
                .map_err(|e| rb_device::DeviceError::Transport(e.to_string()))?;
            if &buf[..n] == b"OK" {
                Ok(())
            } else {
                Err(rb_device::DeviceError::Protocol("unexpected reply".into()))
            }
        }
    }

    struct EchoFactory {
        classes: Vec<DeviceClass>,
    }

    #[async_trait(?Send)]
    impl DriverFactory for EchoFactory {
        fn name(&self) -> &str {
            "echo"
        }
        fn supported_classes(&self) -> &[DeviceClass] {
            &self.classes
        }
        async fn scan(&self) -> DriverResult<Vec<DeviceCandidate>> {
            Ok(vec![DeviceCandidate::new(
                DeviceInfo::new("RustyBench", "Echo"),
                "mock://0",
            )])
        }
        async fn connect(&self, candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>> {
            if candidate.address != "mock://0" {
                return Err(DriverError::NotFound);
            }
            Ok(Box::new(EchoDevice {
                id: DeviceId::new(&candidate.address),
                info: candidate.info.clone(),
                transport: MockTransport::new().with_read_data(b"OK"),
            }))
        }
    }

    #[test]
    fn scan_then_connect_yields_a_live_device() {
        let factory = EchoFactory {
            classes: vec![DeviceClass::Multimeter],
        };
        let candidates = block_on(factory.scan()).unwrap();
        assert_eq!(candidates.len(), 1);

        let mut device = block_on(factory.connect(&candidates[0])).unwrap();
        block_on(device.open()).unwrap();
        assert_eq!(device.id().as_str(), "mock://0");
    }

    #[test]
    fn connect_rejects_unknown_candidate() {
        let factory = EchoFactory { classes: vec![] };
        let bogus = DeviceCandidate::new(DeviceInfo::new("x", "y"), "mock://999");
        assert!(matches!(
            block_on(factory.connect(&bogus)),
            Err(DriverError::NotFound)
        ));
    }

    #[test]
    fn driver_factory_is_object_safe() {
        let _f: Box<dyn DriverFactory> = Box::new(EchoFactory { classes: vec![] });
    }
}
