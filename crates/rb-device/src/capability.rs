//! Per-class capability traits.
//!
//! Each trait corresponds to one [`DeviceClass`](crate::DeviceClass) and
//! describes that class's **control-plane**: the coarse, infrequent commands a
//! frontend issues to configure or steer the instrument (set a rate, arm,
//! enable an output, ...).
//!
//! These methods are `async` (via [`async_trait`]) because they typically round
//! -trip to hardware over a transport. They are deliberately *not* the path for
//! bulk sample data: acquired samples flow into the device's sample store and
//! are read back per-frame, never returned through these calls.
//!
//! `async_trait` is used in `?Send` mode: futures produced on the web (e.g. over
//! WebUSB) are not `Send`, so requiring `Send` here would make the traits
//! unusable in the browser. Device control runs on the owning device's task, so
//! a non-`Send` future is not a problem.

use async_trait::async_trait;
use rb_model::{AnalogChannel, DigitalChannel};

use crate::error::DeviceResult;

/// Captures digital/logic channels.
#[async_trait(?Send)]
pub trait LogicAnalyzer {
    /// The logic channels this device exposes.
    fn channels(&self) -> &[DigitalChannel];
    /// The configured acquisition sample rate, in hertz.
    fn sample_rate_hz(&self) -> f64;
    /// The list of sample rates this device supports, in hertz.
    /// A front-end may use this to populate a drop-down; [`set_sample_rate_hz`]
    /// should validate against this list.
    fn supported_sample_rates(&self) -> &[f64];
    /// Requests a new acquisition sample rate, in hertz.
    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Arms the device so it begins streaming samples into its store.
    async fn arm(&mut self) -> DeviceResult<()>;
    /// Stops an in-progress acquisition.
    async fn stop(&mut self) -> DeviceResult<()>;
}

/// Captures analog waveforms.
#[async_trait(?Send)]
pub trait Oscilloscope {
    /// The analog channels this device exposes.
    fn channels(&self) -> &[AnalogChannel];
    /// The configured acquisition sample rate, in hertz.
    fn sample_rate_hz(&self) -> f64;
    /// The list of sample rates this device supports, in hertz.
    fn supported_sample_rates(&self) -> &[f64];
    /// Requests a new acquisition sample rate, in hertz.
    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Arms the device so it begins streaming samples into its store.
    async fn arm(&mut self) -> DeviceResult<()>;
    /// Stops an in-progress acquisition.
    async fn stop(&mut self) -> DeviceResult<()>;
}

/// Single-value measurement instrument (volts, amps, ohms, ...).
#[async_trait(?Send)]
pub trait Multimeter {
    /// Physical unit of the present reading, if known (e.g. `"V"`).
    fn unit(&self) -> Option<&str>;
    /// Triggers and returns a single measurement in physical units.
    async fn measure(&mut self) -> DeviceResult<f64>;
}

/// Programmable DC power supply output.
#[async_trait(?Send)]
pub trait PowerSupply {
    /// Whether the output is currently enabled.
    fn is_output_enabled(&self) -> bool;
    /// Enables or disables the output.
    async fn set_output_enabled(&mut self, on: bool) -> DeviceResult<()>;
    /// Sets the target output voltage, in volts.
    async fn set_voltage(&mut self, volts: f64) -> DeviceResult<()>;
    /// Sets the current limit, in amperes.
    async fn set_current_limit(&mut self, amps: f64) -> DeviceResult<()>;
}

/// Signal/function generator output.
#[async_trait(?Send)]
pub trait WaveformGenerator {
    /// Whether the output is currently enabled.
    fn is_output_enabled(&self) -> bool;
    /// Enables or disables the output.
    async fn set_output_enabled(&mut self, on: bool) -> DeviceResult<()>;
    /// Sets the output frequency, in hertz.
    async fn set_frequency_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Sets the output amplitude, in volts.
    async fn set_amplitude_v(&mut self, volts: f64) -> DeviceResult<()>;
}

/// Software-defined radio receiver front-end.
#[async_trait(?Send)]
pub trait SdrReceiver {
    /// The configured sample rate, in hertz.
    fn sample_rate_hz(&self) -> f64;
    /// Sets the tuner center frequency, in hertz.
    async fn set_center_frequency_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Sets the receive sample rate, in hertz.
    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Arms the receiver so it begins streaming IQ samples into its store.
    async fn arm(&mut self) -> DeviceResult<()>;
    /// Stops an in-progress acquisition.
    async fn stop(&mut self) -> DeviceResult<()>;
}

/// Frequency-domain analyzer.
#[async_trait(?Send)]
pub trait SpectrumAnalyzer {
    /// Sets the center frequency of the sweep, in hertz.
    async fn set_center_frequency_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Sets the frequency span of the sweep, in hertz.
    async fn set_span_hz(&mut self, hz: f64) -> DeviceResult<()>;
    /// Arms the analyzer so it begins streaming sweeps into its store.
    async fn arm(&mut self) -> DeviceResult<()>;
    /// Stops an in-progress sweep.
    async fn stop(&mut self) -> DeviceResult<()>;
}

/// Regulation mode of an [`ElectronicLoad`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadMode {
    /// Constant current.
    ConstantCurrent,
    /// Constant voltage.
    ConstantVoltage,
    /// Constant resistance.
    ConstantResistance,
    /// Constant power.
    ConstantPower,
}

/// Programmable electronic load.
#[async_trait(?Send)]
pub trait ElectronicLoad {
    /// The active regulation mode.
    fn mode(&self) -> LoadMode;
    /// Selects the regulation mode.
    async fn set_mode(&mut self, mode: LoadMode) -> DeviceResult<()>;
    /// Sets the regulation setpoint in the unit implied by the active [`LoadMode`]
    /// (amperes, volts, ohms, or watts).
    async fn set_setpoint(&mut self, value: f64) -> DeviceResult<()>;
    /// Enables or disables the load input.
    async fn set_input_enabled(&mut self, on: bool) -> DeviceResult<()>;
}
