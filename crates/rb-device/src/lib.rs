//! Device and capability abstractions for RustyBench.
//!
//! This crate defines the *seams* that let every higher layer stay testable
//! without hardware:
//!
//! - [`Device`] — the base trait: identity, connection lifecycle, and **typed
//!   capability accessors** (`as_oscilloscope()` → `Option<&dyn Oscilloscope>`).
//! - [`capability`] — one control-plane trait per [`DeviceClass`]
//!   ([`LogicAnalyzer`], [`Oscilloscope`], [`Multimeter`], ...). A
//!   *multi-class device* implements several.
//! - [`DeviceError`] — the typed error surface shared by the above.
//!
//! Like `rb-model`, it is free of I/O and async-runtime dependencies and
//! compiles unchanged to `wasm32-unknown-unknown` with no feature flags. The
//! capability traits are `async` via `async-trait` in `?Send` mode so they work
//! over non-`Send` web futures (e.g. WebUSB).

#![forbid(unsafe_code)]

mod acquisition;
mod capability;
mod device;
mod error;
mod identity;

pub use acquisition::AcquisitionSource;
pub use capability::{
    ElectronicLoad, LoadMode, LogicAnalyzer, Multimeter, Oscilloscope, PowerSupply, SdrReceiver,
    SpectrumAnalyzer, WaveformGenerator,
};
pub use device::Device;
pub use error::{DeviceError, DeviceResult};
pub use identity::{DeviceClass, DeviceId, DeviceInfo};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::executor::block_on;
    use rb_model::{AnalogChannel, AnalogFormat, ChannelId};

    /// A minimal fake oscilloscope used to exercise the capability seam.
    struct FakeScope {
        id: DeviceId,
        info: DeviceInfo,
        channels: Vec<AnalogChannel>,
        sample_rate_hz: f64,
        opened: bool,
    }

    impl FakeScope {
        fn new() -> Self {
            Self {
                id: DeviceId::new("fake-scope-0"),
                info: DeviceInfo::new("RustyBench", "FakeScope"),
                channels: vec![AnalogChannel::new(
                    ChannelId(0),
                    "CH1",
                    AnalogFormat::identity(),
                )],
                sample_rate_hz: 1_000.0,
                opened: false,
            }
        }
    }

    #[async_trait(?Send)]
    impl Oscilloscope for FakeScope {
        fn channels(&self) -> &[AnalogChannel] {
            &self.channels
        }
        fn sample_rate_hz(&self) -> f64 {
            self.sample_rate_hz
        }
        async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()> {
            if hz <= 0.0 {
                return Err(DeviceError::InvalidParameter(
                    "sample rate must be > 0".into(),
                ));
            }
            self.sample_rate_hz = hz;
            Ok(())
        }
        async fn arm(&mut self) -> DeviceResult<()> {
            Ok(())
        }
        async fn stop(&mut self) -> DeviceResult<()> {
            Ok(())
        }
    }

    #[async_trait(?Send)]
    impl Device for FakeScope {
        fn id(&self) -> &DeviceId {
            &self.id
        }
        fn info(&self) -> &DeviceInfo {
            &self.info
        }
        async fn open(&mut self) -> DeviceResult<()> {
            self.opened = true;
            Ok(())
        }
        fn as_oscilloscope(&self) -> Option<&dyn Oscilloscope> {
            Some(self)
        }
        fn as_oscilloscope_mut(&mut self) -> Option<&mut dyn Oscilloscope> {
            Some(self)
        }
    }

    #[test]
    fn classes_are_derived_from_accessors() {
        let dev = FakeScope::new();
        assert_eq!(dev.classes(), vec![DeviceClass::Oscilloscope]);
    }

    #[test]
    fn present_capability_is_accessible_absent_one_is_none() {
        let dev = FakeScope::new();
        assert!(dev.as_oscilloscope().is_some());
        assert!(dev.as_multimeter().is_none());
        assert!(dev.as_logic_analyzer().is_none());
    }

    #[test]
    fn typed_accessor_exposes_control_plane() {
        let mut dev = FakeScope::new();
        // Read through the immutable accessor.
        assert_eq!(dev.as_oscilloscope().unwrap().channels().len(), 1);
        // Drive the async control-plane through the mutable accessor.
        let scope = dev.as_oscilloscope_mut().unwrap();
        block_on(scope.set_sample_rate_hz(2_000.0)).unwrap();
        assert_eq!(dev.as_oscilloscope().unwrap().sample_rate_hz(), 2_000.0);
    }

    #[test]
    fn invalid_parameter_is_rejected() {
        let mut dev = FakeScope::new();
        let scope = dev.as_oscilloscope_mut().unwrap();
        let err = block_on(scope.set_sample_rate_hz(0.0)).unwrap_err();
        assert!(matches!(err, DeviceError::InvalidParameter(_)));
    }

    #[test]
    fn device_is_object_safe_and_lifecycle_runs() {
        let mut dev: Box<dyn Device> = Box::new(FakeScope::new());
        block_on(dev.open()).unwrap();
        assert_eq!(dev.id().as_str(), "fake-scope-0");
        assert_eq!(dev.info().model, "FakeScope");
        block_on(dev.close()).unwrap();
    }
}
