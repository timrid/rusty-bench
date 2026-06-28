//! Channel metadata and analog scaling.
//!
//! Channels describe *what* a stream of samples means; the actual samples live
//! in the stores ([`crate::AnalogStore`], [`crate::DigitalStore`]). Analog
//! samples are stored as raw integers and converted to physical units on demand
//! via an [`AnalogFormat`].

/// Stable identifier for a channel within a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(pub u32);

/// Linear mapping from a raw integer sample to a physical value.
///
/// `physical = raw * scale + offset`. Keeping the raw integer at the base level
/// preserves the device's native ADC resolution and memory footprint; the
/// floating-point conversion is applied only where a physical value is needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalogFormat {
    /// Physical units per raw count.
    pub scale: f64,
    /// Physical value corresponding to a raw count of `0`.
    pub offset: f64,
}

impl AnalogFormat {
    /// Creates a format with the given `scale` and `offset`.
    ///
    /// # Panics
    /// Panics if `scale` or `offset` is not finite.
    #[must_use]
    pub fn new(scale: f64, offset: f64) -> Self {
        assert!(
            scale.is_finite() && offset.is_finite(),
            "analog scale/offset must be finite"
        );
        Self { scale, offset }
    }

    /// The identity mapping (`physical == raw`).
    #[must_use]
    pub fn identity() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
        }
    }

    /// Converts a raw sample to its physical value.
    #[must_use]
    pub fn to_physical(&self, raw: i32) -> f64 {
        raw as f64 * self.scale + self.offset
    }

    /// Converts a (possibly fractional) raw average to physical.
    #[must_use]
    pub fn to_physical_f64(&self, raw: f64) -> f64 {
        raw * self.scale + self.offset
    }
}

impl Default for AnalogFormat {
    fn default() -> Self {
        Self::identity()
    }
}

/// An analog input channel: a continuously-valued signal stored as raw integers.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogChannel {
    /// Stable identifier within the capture.
    pub id: ChannelId,
    /// Human-readable name (e.g. `"CH1"`).
    pub name: String,
    /// Raw-to-physical conversion.
    pub format: AnalogFormat,
    /// Physical unit symbol for display (e.g. `"V"`), if known.
    pub unit: Option<String>,
}

impl AnalogChannel {
    /// Creates an analog channel.
    #[must_use]
    pub fn new(id: ChannelId, name: impl Into<String>, format: AnalogFormat) -> Self {
        Self {
            id,
            name: name.into(),
            format,
            unit: None,
        }
    }

    /// Sets the physical unit symbol.
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// A single digital (logic) channel, identified by its bit position within a
/// packed [`crate::LogicWord`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigitalChannel {
    /// Stable identifier within the capture.
    pub id: ChannelId,
    /// Human-readable name (e.g. `"D0"`).
    pub name: String,
    /// Bit position (`0..64`) within each logic word.
    pub bit: u8,
}

impl DigitalChannel {
    /// Creates a digital channel bound to `bit`.
    ///
    /// # Panics
    /// Panics if `bit >= 64`.
    #[must_use]
    pub fn new(id: ChannelId, name: impl Into<String>, bit: u8) -> Self {
        assert!(bit < 64, "digital channel bit must be < 64");
        Self {
            id,
            name: name.into(),
            bit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_physical_applies_scale_and_offset() {
        let fmt = AnalogFormat::new(0.5, -1.0);
        assert!((fmt.to_physical(0) - (-1.0)).abs() < 1e-12);
        assert!((fmt.to_physical(10) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn identity_is_passthrough() {
        let fmt = AnalogFormat::identity();
        assert!((fmt.to_physical(42) - 42.0).abs() < 1e-12);
    }

    #[test]
    fn analog_channel_builder() {
        let ch = AnalogChannel::new(ChannelId(1), "CH1", AnalogFormat::identity()).with_unit("V");
        assert_eq!(ch.name, "CH1");
        assert_eq!(ch.unit.as_deref(), Some("V"));
    }

    #[test]
    #[should_panic(expected = "bit must be < 64")]
    fn digital_channel_bit_bounds() {
        let _ = DigitalChannel::new(ChannelId(0), "D64", 64);
    }
}
