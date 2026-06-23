//! Device drivers for RustyBench.
//!
//! One module per driver, each gated behind a Cargo feature. The
//! [`factories`] function is the explicit central registry: it collects one
//! [`DriverFactory`](rb_transport::DriverFactory) per active driver via
//! `#[cfg(feature)]`, so a build only ever pulls in the drivers it enabled. This
//! keeps web builds lean and the active driver set deterministic (no global
//! registration magic).

#![forbid(unsafe_code)]

#[cfg(feature = "demo")]
pub mod demo;
#[cfg(feature = "fx2lafw")]
pub mod fx2lafw;

use rb_transport::DriverFactory;

/// Collects a boxed [`DriverFactory`] for every driver enabled at compile time.
///
/// The set is determined entirely by Cargo features: enabling the `demo` feature
/// adds the synthetic [`demo::DemoFactory`], enabling `fx2lafw` adds the
/// [`fx2lafw::Fx2lafwFactory`], and so on for future drivers.
#[must_use]
#[allow(clippy::vec_init_then_push)] // push is conditional on enabled drivers
pub fn factories() -> Vec<Box<dyn DriverFactory>> {
    #[allow(unused_mut)]
    let mut factories: Vec<Box<dyn DriverFactory>> = Vec::new();
    #[cfg(feature = "demo")]
    factories.push(Box::new(demo::DemoFactory::new()));
    #[cfg(feature = "fx2lafw")]
    factories.push(Box::new(fx2lafw::Fx2lafwFactory::new()));
    factories
}

/// Collects all known USB VID/PID pairs from every driver enabled at compile
/// time.  Useful for building WebUSB `requestDevice()` filters so the browser
/// can show a permission dialog for any supported device.
///
/// Synthetic drivers (like `demo`) contribute nothing; only drivers backed by
/// real USB hardware add entries.
#[must_use]
pub fn known_usb_vid_pids() -> Vec<(u16, u16)> {
    let mut v: Vec<(u16, u16)> = Vec::new();
    #[cfg(feature = "demo")]
    v.extend(demo::known_vid_pids());
    #[cfg(feature = "fx2lafw")]
    v.extend(fx2lafw::known_vid_pids());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_enabled_drivers() {
        let factories = factories();
        #[cfg(feature = "demo")]
        assert!(factories.iter().any(|f| f.name() == "demo"));
        #[cfg(not(feature = "demo"))]
        assert!(factories.is_empty());
        #[cfg(feature = "fx2lafw")]
        assert!(factories.iter().any(|f| f.name() == "fx2lafw"));
    }
}
