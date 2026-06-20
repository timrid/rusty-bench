//! The base [`Device`] trait: identity, lifecycle, and typed capability access.

use async_trait::async_trait;

use crate::capability::{
    ElectronicLoad, LogicAnalyzer, Multimeter, Oscilloscope, PowerSupply, SdrReceiver,
    SpectrumAnalyzer, WaveformGenerator,
};
use crate::error::DeviceResult;
use crate::identity::{DeviceClass, DeviceId, DeviceInfo};

/// A connected piece of instrumentation.
///
/// A device owns its identity ([`id`](Device::id), [`info`](Device::info)) and
/// its connection lifecycle ([`open`](Device::open) / [`close`](Device::close)),
/// and exposes its abilities through **typed capability accessors**
/// (`as_oscilloscope`, `as_multimeter`, ...). Frontends discover what a device
/// can do by probing these accessors rather than by downcasting through
/// [`Any`](core::any::Any) — the set of `Some` results drives which views are
/// built.
///
/// A *multi-class device* simply implements several capability traits and
/// returns `Some` from the matching accessors.
///
/// Each capability has an immutable (`as_*`) and a mutable (`as_*_mut`)
/// accessor; the control-plane methods take `&mut self`, so callers issuing
/// commands use the `_mut` form.
#[async_trait(?Send)]
pub trait Device {
    /// Stable identifier for this device within the session.
    fn id(&self) -> &DeviceId;

    /// Human-readable identity (vendor / model / serial).
    fn info(&self) -> &DeviceInfo;

    /// The device classes this device belongs to.
    ///
    /// Derived by default from the capability accessors, so implementors only
    /// need to wire up the accessors themselves.
    fn classes(&self) -> Vec<DeviceClass> {
        let mut classes = Vec::new();
        if self.as_logic_analyzer().is_some() {
            classes.push(DeviceClass::LogicAnalyzer);
        }
        if self.as_multimeter().is_some() {
            classes.push(DeviceClass::Multimeter);
        }
        if self.as_oscilloscope().is_some() {
            classes.push(DeviceClass::Oscilloscope);
        }
        if self.as_power_supply().is_some() {
            classes.push(DeviceClass::PowerSupply);
        }
        if self.as_waveform_generator().is_some() {
            classes.push(DeviceClass::WaveformGenerator);
        }
        if self.as_sdr_receiver().is_some() {
            classes.push(DeviceClass::SdrReceiver);
        }
        if self.as_spectrum_analyzer().is_some() {
            classes.push(DeviceClass::SpectrumAnalyzer);
        }
        if self.as_electronic_load().is_some() {
            classes.push(DeviceClass::ElectronicLoad);
        }
        classes
    }

    /// Opens the connection and prepares the device for use.
    async fn open(&mut self) -> DeviceResult<()> {
        Ok(())
    }

    /// Closes the connection and releases device resources.
    async fn close(&mut self) -> DeviceResult<()> {
        Ok(())
    }

    /// Logic-analyzer capability, if supported.
    fn as_logic_analyzer(&self) -> Option<&dyn LogicAnalyzer> {
        None
    }
    /// Mutable logic-analyzer capability, if supported.
    fn as_logic_analyzer_mut(&mut self) -> Option<&mut dyn LogicAnalyzer> {
        None
    }

    /// Multimeter capability, if supported.
    fn as_multimeter(&self) -> Option<&dyn Multimeter> {
        None
    }
    /// Mutable multimeter capability, if supported.
    fn as_multimeter_mut(&mut self) -> Option<&mut dyn Multimeter> {
        None
    }

    /// Oscilloscope capability, if supported.
    fn as_oscilloscope(&self) -> Option<&dyn Oscilloscope> {
        None
    }
    /// Mutable oscilloscope capability, if supported.
    fn as_oscilloscope_mut(&mut self) -> Option<&mut dyn Oscilloscope> {
        None
    }

    /// Power-supply capability, if supported.
    fn as_power_supply(&self) -> Option<&dyn PowerSupply> {
        None
    }
    /// Mutable power-supply capability, if supported.
    fn as_power_supply_mut(&mut self) -> Option<&mut dyn PowerSupply> {
        None
    }

    /// Waveform-generator capability, if supported.
    fn as_waveform_generator(&self) -> Option<&dyn WaveformGenerator> {
        None
    }
    /// Mutable waveform-generator capability, if supported.
    fn as_waveform_generator_mut(&mut self) -> Option<&mut dyn WaveformGenerator> {
        None
    }

    /// SDR-receiver capability, if supported.
    fn as_sdr_receiver(&self) -> Option<&dyn SdrReceiver> {
        None
    }
    /// Mutable SDR-receiver capability, if supported.
    fn as_sdr_receiver_mut(&mut self) -> Option<&mut dyn SdrReceiver> {
        None
    }

    /// Spectrum-analyzer capability, if supported.
    fn as_spectrum_analyzer(&self) -> Option<&dyn SpectrumAnalyzer> {
        None
    }
    /// Mutable spectrum-analyzer capability, if supported.
    fn as_spectrum_analyzer_mut(&mut self) -> Option<&mut dyn SpectrumAnalyzer> {
        None
    }

    /// Electronic-load capability, if supported.
    fn as_electronic_load(&self) -> Option<&dyn ElectronicLoad> {
        None
    }
    /// Mutable electronic-load capability, if supported.
    fn as_electronic_load_mut(&mut self) -> Option<&mut dyn ElectronicLoad> {
        None
    }
}
