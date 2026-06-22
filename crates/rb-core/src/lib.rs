//! RustyBench core: the [`Session`] that owns connected devices, the
//! [`DriverRegistry`] of active drivers, and the runtime glue that fills each
//! device's stores.
//!
//! # Layers
//! - [`DriverRegistry`] scans and connects devices via the active
//!   [`DriverFactory`](rb_transport::DriverFactory)s.
//! - [`Session`] maps each [`DeviceId`](rb_device::DeviceId) to a [`DeviceHandle`]
//!   (device + per-channel stores + [`AcquisitionState`]).
//! - [`runtime::run_acquisition`] drives a handle from a command stream,
//!   pumping continuously; the `native` and `web` features add the matching
//!   spawners.
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
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::task::LocalSpawnExt;

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
        let (read_loop, mut data_rx) = block_on(handle.start_streaming()).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(data_rx.next()).unwrap();
        handle.ingest_chunk(&chunk);

        drop(data_rx);
        pool.run_until_stalled();

        let handle = session.device(&id).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Running);
        assert!(handle.sample_count() > 0, "should have streamed samples");
        assert_eq!(handle.analog_traces()[0].len(), handle.sample_count());
        assert_eq!(handle.digital_trace().unwrap().len(), handle.sample_count());
    }
}
