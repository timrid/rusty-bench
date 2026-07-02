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

/// Resolves driver firmware files by name (e.g. `"fx2lafw-cypress-fx2.fw"`).
///
/// The host application (GUI / CLI) implements this trait to provide firmware
/// bytes at runtime via platform-specific asset loading (Dioxus `asset!()`,
/// `std::fs`, etc.).  This trait is always available regardless of which
/// Cargo features are enabled.
#[async_trait::async_trait(?Send)]
pub trait FirmwareLoader {
    /// Returns the firmware bytes for the given file name.
    async fn load_firmware(&self, name: &str) -> Result<Vec<u8>, String>;
}

/// Collects a boxed [`DriverFactory`] for every driver enabled at compile time.
///
/// The set is determined entirely by Cargo features: enabling the `demo` feature
/// adds the synthetic [`demo::DemoFactory`], enabling `fx2lafw` adds the
/// [`fx2lafw::Fx2lafwFactory`], and so on for future drivers.
///
/// Use [`factories_with_firmware`] if you want to provide firmware for
/// bootloader-based drivers like fx2lafw.
#[must_use]
#[allow(clippy::vec_init_then_push)] // push is conditional on enabled drivers
pub fn factories() -> Vec<Box<dyn DriverFactory>> {
    factories_with_firmware(None)
}

/// Like [`factories`], but provides a [`FirmwareLoader`] so that
/// bootloader devices (Cypress FX2 without EEPROM) are automatically flashed
/// during [`scan`](DriverFactory::scan).
#[must_use]
#[allow(clippy::vec_init_then_push)]
pub fn factories_with_firmware(
    firmware_loader: Option<Box<dyn FirmwareLoader>>,
) -> Vec<Box<dyn DriverFactory>> {
    #[allow(unused_mut)]
    let mut factories: Vec<Box<dyn DriverFactory>> = Vec::new();
    #[cfg(feature = "demo")]
    factories.push(Box::new(demo::DemoFactory::new()));
    #[cfg(feature = "fx2lafw")]
    {
        let mut fxlafw = fx2lafw::Fx2lafwFactory::new();
        if let Some(loader) = firmware_loader {
            fxlafw.set_firmware_loader(loader);
        }
        factories.push(Box::new(fxlafw));
    }
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
