//! The [`DriverRegistry`]: the active set of [`DriverFactory`]s and scan/connect.

use rb_device::Device;
use rb_transport::{DeviceCandidate, DriverFactory};

use crate::error::SessionError;

/// One reachable device found during a scan, tagged with the driver that found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    /// Name of the driver that produced this candidate.
    pub driver: String,
    /// The reachable-but-not-yet-connected device.
    pub candidate: DeviceCandidate,
}

/// The set of drivers active in this build, ready to scan and connect.
///
/// The default set comes from [`rb_drivers::factories`], which collects one
/// factory per Cargo-enabled driver. Front-ends scan across all drivers and then
/// connect a chosen candidate by driver name.
pub struct DriverRegistry {
    factories: Vec<Box<dyn DriverFactory>>,
}

impl DriverRegistry {
    /// Builds a registry from an explicit list of factories.
    #[must_use]
    pub fn new(factories: Vec<Box<dyn DriverFactory>>) -> Self {
        Self { factories }
    }

    /// Builds a registry from the drivers enabled at compile time.
    #[must_use]
    pub fn with_default_factories() -> Self {
        Self::new(rb_drivers::factories())
    }

    /// The registered factories.
    #[must_use]
    pub fn factories(&self) -> &[Box<dyn DriverFactory>] {
        &self.factories
    }

    /// Number of registered drivers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Whether no drivers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Scans every registered driver and returns all discovered candidates.
    ///
    /// # Errors
    /// Returns the first [`SessionError::Driver`] if any driver's scan fails.
    pub async fn scan_all(&self) -> Result<Vec<ScanResult>, SessionError> {
        let mut results = Vec::new();
        for factory in &self.factories {
            for candidate in factory.scan().await? {
                results.push(ScanResult {
                    driver: factory.name().to_string(),
                    candidate,
                });
            }
        }
        Ok(results)
    }

    /// Connects a candidate using the named driver.
    ///
    /// # Errors
    /// Returns [`SessionError::UnknownDriver`] if no such driver exists, or a
    /// [`SessionError::Driver`] if the connection fails.
    pub async fn connect(
        &self,
        driver: &str,
        candidate: &DeviceCandidate,
    ) -> Result<Box<dyn Device>, SessionError> {
        let factory = self
            .factories
            .iter()
            .find(|f| f.name() == driver)
            .ok_or_else(|| SessionError::UnknownDriver(driver.to_string()))?;
        Ok(factory.connect(candidate).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn default_registry_includes_the_demo_driver() {
        let registry = DriverRegistry::with_default_factories();
        assert!(!registry.is_empty());
        assert!(registry.factories().iter().any(|f| f.name() == "demo"));
    }

    #[test]
    fn scan_all_then_connect_yields_a_demo_device() {
        let registry = DriverRegistry::with_default_factories();
        let results = block_on(registry.scan_all()).unwrap();
        let demo = results
            .iter()
            .find(|r| r.driver == "demo")
            .expect("demo candidate present");

        let device = block_on(registry.connect("demo", &demo.candidate)).unwrap();
        assert!(device.as_oscilloscope().is_some());
        assert!(device.as_logic_analyzer().is_some());
    }

    #[test]
    fn connect_rejects_unknown_driver() {
        let registry = DriverRegistry::with_default_factories();
        let candidate = DeviceCandidate::new(rb_device::DeviceInfo::new("x", "y"), "addr");
        assert!(matches!(
            block_on(registry.connect("does-not-exist", &candidate)),
            Err(SessionError::UnknownDriver(_))
        ));
    }
}
