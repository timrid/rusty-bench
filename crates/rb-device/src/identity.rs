//! Device identity and class taxonomy.
//!
//! A [`Device`](crate::Device) is identified at runtime by a stable
//! [`DeviceId`] and described by [`DeviceInfo`]. The abilities it exposes are
//! categorised by [`DeviceClass`]; a single device may belong to several
//! classes (a *multi-class device*).

/// A category of instrumentation ability.
///
/// Each variant maps one-to-one to a capability trait in
/// [`crate::capability`]. The set of classes a device belongs to is derived
/// from the capability traits it implements (see [`Device::classes`]).
///
/// [`Device::classes`]: crate::Device::classes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum DeviceClass {
    /// Captures digital/logic channels.
    LogicAnalyzer,
    /// Single-value measurement instrument (volts, amps, ohms, ...).
    Multimeter,
    /// Captures analog waveforms.
    Oscilloscope,
    /// Programmable DC power supply.
    PowerSupply,
    /// Signal/function generator.
    WaveformGenerator,
    /// Software-defined radio receiver.
    SdrReceiver,
    /// Frequency-domain analyzer.
    SpectrumAnalyzer,
    /// Programmable electronic load.
    ElectronicLoad,
}

/// Stable identifier for a connected device within a [`Session`].
///
/// The string is driver-defined but must be stable for the lifetime of the
/// connection so the session can map it back to its handle.
///
/// [`Session`]: # "owned by rb-core"
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(String);

impl DeviceId {
    /// Wraps a driver-supplied identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Human-readable identity of a device, independent of its capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    /// Manufacturer / vendor name (e.g. `"sigrok"`).
    pub vendor: String,
    /// Model or product name (e.g. `"fx2lafw"`).
    pub model: String,
    /// Serial number, if the device reports one.
    pub serial: Option<String>,
}

impl DeviceInfo {
    /// Creates a [`DeviceInfo`] with no serial number.
    #[must_use]
    pub fn new(vendor: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            vendor: vendor.into(),
            model: model.into(),
            serial: None,
        }
    }

    /// Sets the serial number.
    #[must_use]
    pub fn with_serial(mut self, serial: impl Into<String>) -> Self {
        self.serial = Some(serial.into());
        self
    }
}
