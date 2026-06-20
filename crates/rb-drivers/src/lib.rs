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

use rb_transport::DriverFactory;

/// Collects a boxed [`DriverFactory`] for every driver enabled at compile time.
///
/// The set is determined entirely by Cargo features: enabling the `demo` feature
/// adds the synthetic [`demo::DemoFactory`], and so on for future drivers.
#[must_use]
#[allow(clippy::vec_init_then_push)] // push is conditional on enabled drivers
pub fn factories() -> Vec<Box<dyn DriverFactory>> {
    #[allow(unused_mut)]
    let mut factories: Vec<Box<dyn DriverFactory>> = Vec::new();
    #[cfg(feature = "demo")]
    factories.push(Box::new(demo::DemoFactory::new()));
    factories
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
    }
}
