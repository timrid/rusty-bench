//! RustyBench core: the [`Session`] that owns connected devices, the
//! [`DriverRegistry`] of active drivers, and the runtime glue for acquisition.
//!
//! # Layers
//! - [`DriverRegistry`] scans and connects devices via the active
//!   [`DriverFactory`](rb_transport::DriverFactory)s.
//! - [`Session`] maps each [`DeviceId`](rb_device::DeviceId) to a [`DeviceHandle`]
//!   (an exclusive access token for a connected device).
//! - [`runtime::run_acquisition`] polls a pre-armed device's read-loop and
//!   data receiver concurrently; the `native` and `web` features add spawners.
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
pub use handle::DeviceHandle;
pub use registry::{DeviceOrigin, DriverRegistry, KnownDevice, ScanResult};
pub use runtime::run_acquisition;
pub use session::Session;
