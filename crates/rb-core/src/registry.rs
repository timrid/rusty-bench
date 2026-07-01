//! The [`DriverRegistry`]: the active set of [`DriverFactory`]s and scan/connect.

use rb_device::Device;
use rb_transport::{DeviceCandidate, DriverFactory};

use crate::error::SessionError;

/// How we learned about a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceOrigin {
    /// Found by a driver scan.
    Discovered,
    /// Manually added by the user.
    Manual,
}

/// A device we know how to reach, regardless of how we learned about it.
///
/// Wraps a [`DeviceCandidate`] with the driver name and provenance.
/// This is the unit the UI displays in the device dropdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownDevice {
    /// The driver to use for connecting.
    pub driver: String,
    /// The candidate (identity + opaque address).
    pub candidate: DeviceCandidate,
    /// How this entry entered the system.
    pub origin: DeviceOrigin,
}

// Compatibility alias — the old name still works.
pub type ScanResult = KnownDevice;

/// The set of drivers active in this build, ready to scan and connect.
///
/// The default set comes from [`rb_drivers::factories`], which collects one
/// factory per Cargo-enabled driver. Front-ends scan across all drivers and then
/// connect a chosen candidate by driver name.
pub struct DriverRegistry {
    factories: std::rc::Rc<Vec<Box<dyn DriverFactory>>>,
}

impl Clone for DriverRegistry {
    fn clone(&self) -> Self {
        Self {
            factories: self.factories.clone(),
        }
    }
}

impl DriverRegistry {
    /// Builds a registry from an explicit list of factories.
    #[must_use]
    pub fn new(factories: Vec<Box<dyn DriverFactory>>) -> Self {
        Self {
            factories: std::rc::Rc::new(factories),
        }
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
    pub async fn scan_all(&self) -> Result<Vec<KnownDevice>, SessionError> {
        let mut results = Vec::new();
        for factory in self.factories.iter() {
            for candidate in factory.scan().await? {
                results.push(KnownDevice {
                    driver: factory.name().to_string(),
                    candidate,
                    origin: DeviceOrigin::Discovered,
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

    #[tokio::test(flavor = "current_thread")]
    async fn scan_all_then_connect_yields_a_demo_device() {
        let registry = DriverRegistry::with_default_factories();
        let results = registry.scan_all().await.unwrap();
        let demo = results
            .iter()
            .find(|r| r.driver == "demo")
            .expect("demo candidate present");

        let device = registry.connect("demo", &demo.candidate).await.unwrap();
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
