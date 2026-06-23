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

    #[tokio::test(flavor = "current_thread")]
    async fn end_to_end_scan_connect_acquire() {
        // Discover the demo device, register it in a session and acquire.
        let registry = DriverRegistry::with_default_factories();
        let results = registry.scan_all().await.unwrap();
        let demo = results
            .iter()
            .find(|r| r.driver == "demo")
            .expect("demo candidate present");
        let device = registry.connect("demo", &demo.candidate).await.unwrap();

        let mut session = Session::new();
        let id = session.add_device(device);

        let handle = session.device_mut(&id).unwrap();
        let (read_loop, mut data_rx) = handle.start_streaming().await.unwrap();

        let local_set = tokio::task::LocalSet::new();
        local_set.spawn_local(read_loop);

        let chunk = local_set.run_until(data_rx.next()).await.unwrap();
        handle.ingest_chunk(&chunk);

        // Stop streaming so the read-loop future exits.
        handle.apply(AcquisitionCommand::Stop).await.unwrap();
        drop(data_rx);
        // Run remaining tasks until the read-loop exits or we time out.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            local_set.run_until(futures::future::pending::<()>()),
        )
        .await;

        let handle = session.device(&id).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Stopped);
        assert!(handle.sample_count() > 0, "should have streamed samples");
        assert_eq!(handle.analog_traces()[0].len(), handle.sample_count());
        assert_eq!(handle.digital_trace().unwrap().len(), handle.sample_count());
    }
}
