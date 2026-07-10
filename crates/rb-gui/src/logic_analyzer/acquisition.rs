//! Per-tab acquisition state: configuration, traces, and push_chunk helper.
//!
//! [`AcquisitionConfig`] is the **source of truth** for what channels to
//! acquire and at what rate. It is built from device capabilities on connect
//! and drives trace creation — not the other way around.

use rb_device::Device;
use rb_model::{AnalogChannel, AnalogChunkData, AnalogTrace, DigitalChannel, DigitalChunkData, DigitalTrace, SampleChunk, Timebase};

// ── Acquisition configuration ─────────────────────────────────────────────────

/// User-facing configuration for the next acquisition run.
///
/// Built once from device capabilities when a device is connected.
/// Persists across Stop → Start cycles.  Channel metadata is stable;
/// enabled flags and sample rate can be edited in the channel-config panel.
#[derive(Clone, Debug, PartialEq)]
pub struct AcquisitionConfig {
    /// Desired sample rate in Hz.
    pub sample_rate_hz: f64,
    /// The list of sample rates the device supports, in hertz.
    /// Empty when the driver does not advertise a fixed list (free input).
    pub supported_sample_rates: Vec<f64>,
    /// Analog channel descriptors (device capabilities, stable).
    pub analog_channels: Vec<AnalogChannel>,
    /// Per-analog-channel enable flags. A disabled channel is still present in
    /// traces but [`push_chunk`] skips it.
    pub analog_enabled: Vec<bool>,
    /// Digital channel descriptors (device capabilities, stable).
    pub digital_channels: Vec<DigitalChannel>,
    /// Per-digital-channel enable flags.
    pub digital_enabled: Vec<bool>,
}

/// Default sample rate when the device reports 0 or no rate (200 kHz).
pub const DEFAULT_SAMPLE_RATE_HZ: f64 = 200_000.0;

impl AcquisitionConfig {
    /// Builds a default config from a connected device's capabilities.
    /// All channels are enabled; the sample rate is the device's native rate,
    /// falling back to [`DEFAULT_SAMPLE_RATE_HZ`].
    pub fn from_device(device: &dyn Device) -> Self {
        let analog_channels = device
            .as_oscilloscope()
            .map(|s| s.channels().to_vec())
            .unwrap_or_default();
        let digital_channels = device
            .as_logic_analyzer()
            .map(|la| la.channels().to_vec())
            .unwrap_or_default();

        let supported_sample_rates = device
            .as_oscilloscope()
            .map(|s| s.supported_sample_rates().to_vec())
            .or_else(|| {
                device
                    .as_logic_analyzer()
                    .map(|la| la.supported_sample_rates().to_vec())
            })
            .unwrap_or_default();

        let sample_rate_hz = supported_sample_rates
            .first()
            .copied()
            .or_else(|| {
                let raw = device
                    .as_oscilloscope()
                    .map(|s| s.sample_rate_hz())
                    .or_else(|| device.as_logic_analyzer().map(|la| la.sample_rate_hz()))
                    .unwrap_or(0.0);
                if raw > 0.0 { Some(raw) } else { None }
            })
            .unwrap_or(DEFAULT_SAMPLE_RATE_HZ);

        Self {
            sample_rate_hz,
            supported_sample_rates,
            analog_enabled: vec![true; analog_channels.len()],
            digital_enabled: vec![true; digital_channels.len()],
            analog_channels,
            digital_channels,
        }
    }

    /// Creates analog and digital traces from this config.
    pub fn build_traces(&self) -> (Vec<AnalogTrace>, Option<DigitalTrace>) {
        let timebase = Timebase::new(clamp_rate(self.sample_rate_hz), 0.0);
        let analog: Vec<AnalogTrace> = self
            .analog_channels
            .iter()
            .map(|ch| AnalogTrace::new(ch.clone(), timebase))
            .collect();
        let digital = if !self.digital_channels.is_empty() {
            Some(DigitalTrace::new(self.digital_channels.clone(), timebase))
        } else {
            None
        };
        (analog, digital)
    }
}

impl Default for AcquisitionConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            supported_sample_rates: Vec::new(),
            analog_channels: Vec::new(),
            analog_enabled: Vec::new(),
            digital_channels: Vec::new(),
            digital_enabled: Vec::new(),
        }
    }
}

fn clamp_rate(hz: f64) -> f64 {
    if hz.is_finite() && hz > 0.0 { hz } else { 1.0 }
}

// ── push_chunk helper ─────────────────────────────────────────────────────────

/// Push a [`SampleChunk`] into analog and digital traces, respecting enable flags.
pub fn push_chunk(
    chunk: &SampleChunk,
    analog: &mut [AnalogTrace],
    digital: &mut Option<DigitalTrace>,
    config: &AcquisitionConfig,
) -> usize {
    let mut added = 0;

    // ── Analog dispatch ──────────────────────────────────────────────────
    if let Some(ref a) = chunk.analog() {
        match a {
            AnalogChunkData::I32(channels) => {
                for (index, trace) in analog.iter_mut().enumerate() {
                    if !config.analog_enabled.get(index).copied().unwrap_or(true) {
                        continue;
                    }
                    if let Some(samples) = channels.get(index) {
                        trace.push_raw(samples);
                        added += samples.len();
                    }
                }
            }
            AnalogChunkData::I16(channels) => {
                for (index, trace) in analog.iter_mut().enumerate() {
                    if !config.analog_enabled.get(index).copied().unwrap_or(true) {
                        continue;
                    }
                    if let Some(samples) = channels.get(index) {
                        trace.push_raw_i16(samples);
                        added += samples.len();
                    }
                }
            }
            AnalogChunkData::I8(channels) => {
                for (index, trace) in analog.iter_mut().enumerate() {
                    if !config.analog_enabled.get(index).copied().unwrap_or(true) {
                        continue;
                    }
                    if let Some(samples) = channels.get(index) {
                        trace.push_raw_i8(samples);
                        added += samples.len();
                    }
                }
            }
        }
    }

    // ── Digital dispatch ─────────────────────────────────────────────────
    if let Some(dt) = digital {
        if config.digital_enabled.iter().any(|e| *e) {
            if let Some(ref data) = chunk.digital() {
                match data {
                    DigitalChunkData::Raw8(bytes) => {
                        dt.push_raw_8bit(bytes);
                        added += bytes.len();
                    }
                    DigitalChunkData::Words(words) if !words.is_empty() => {
                        dt.push_words(words);
                        added += words.len();
                    }
                    _ => {}
                }
            }
        }
    }
    added
}
