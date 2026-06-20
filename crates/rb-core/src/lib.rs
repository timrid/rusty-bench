//! RustyBench core: the [`Session`] that owns connected devices, the
//! [`DriverRegistry`] of active drivers, and the runtime glue that fills each
//! device's stores.
//!
//! # Layers
//! - [`DriverRegistry`] scans and connects devices via the active
//!   [`DriverFactory`](rb_transport::DriverFactory)s.
//! - [`Session`] maps each [`DeviceId`](rb_device::DeviceId) to a [`DeviceHandle`]
//!   (device + per-channel stores + [`AcquisitionState`]).
//! - [`runtime::run_acquisition`] drives a handle from command/tick streams; the
//!   `native` and `web` features add the matching spawners.
//!
//! The default build is runtime-free (only `futures`) and compiles to
//! `wasm32-unknown-unknown` with no features.

#![forbid(unsafe_code)]

mod error;
mod handle;
mod registry;
pub mod runtime;
mod session;

pub use error::SessionError;
pub use handle::{AcquisitionCommand, AcquisitionState, DeviceHandle};
pub use registry::{DriverRegistry, ScanResult};
pub use runtime::run_acquisition;
pub use session::Session;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn end_to_end_scan_connect_acquire() {
        // Discover the demo device, register it in a session and acquire.
        let registry = DriverRegistry::with_default_factories();
        let results = block_on(registry.scan_all()).unwrap();
        let demo = results
            .iter()
            .find(|r| r.driver == "demo")
            .expect("demo candidate present");
        let device = block_on(registry.connect("demo", &demo.candidate)).unwrap();

        let mut session = Session::new();
        let id = session.add_device(device);

        let handle = session.device_mut(&id).unwrap();
        block_on(handle.apply(AcquisitionCommand::Start)).unwrap();
        handle.pump(256);

        let handle = session.device(&id).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Running);
        assert_eq!(handle.sample_count(), 256);
        assert_eq!(handle.analog_traces()[0].len(), 256);
        assert_eq!(handle.digital_trace().unwrap().len(), 256);
    }
}
