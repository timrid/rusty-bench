//! Per-device acquisition state: traces, configuration, and the spawned future handle.
//!
//! [`DeviceAcquisition`] bundles a background acquisition future with local
//! display stores. Data flows through `data_rx` into per-channel traces.
//!
//! [`AcquisitionConfig`] is the **source of truth** for what channels to
//! acquire and at what rate. It is built from device capabilities on connect
//! and drives trace creation — not the other way around.

use futures::channel::mpsc;
use rb_core::{AcquisitionCommand, AcquisitionState, DeviceHandle};
use rb_device::Device;
use rb_model::{AnalogChannel, AnalogTrace, DigitalChannel, DigitalTrace, SampleChunk, Timebase};

// ── Acquisition configuration ─────────────────────────────────────────────────

/// User-facing configuration for the next acquisition run.
///
/// Built once from device capabilities when a device is connected.
/// Persists across Stop → Start cycles.  Channel metadata is stable;
/// enabled flags and sample rate can be edited in the channel-config panel.
#[derive(Clone, Debug)]
pub struct AcquisitionConfig {
    /// Desired sample rate in Hz.  Sent via [`SetSampleRate`](AcquisitionCommand::SetSampleRate)
    /// before every [`Start`](AcquisitionCommand::Start).
    pub sample_rate_hz: f64,
    /// Analog channel descriptors (device capabilities, stable).
    pub analog_channels: Vec<AnalogChannel>,
    /// Per-analog-channel enable flags. A disabled channel is still present in
    /// traces but [`DeviceAcquisition::drain`] skips it.
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

        // Prefer analog rate, fall back to digital, then the default.
        // Some drivers (e.g. fx2lafw) report 0.0 before the first arm();
        // use the configured default in that case.
        let raw_rate = device
            .as_oscilloscope()
            .map(|s| s.sample_rate_hz())
            .or_else(|| device.as_logic_analyzer().map(|la| la.sample_rate_hz()))
            .unwrap_or(0.0);
        let sample_rate_hz = if raw_rate > 0.0 {
            raw_rate
        } else {
            DEFAULT_SAMPLE_RATE_HZ
        };

        Self {
            sample_rate_hz,
            analog_enabled: vec![true; analog_channels.len()],
            digital_enabled: vec![true; digital_channels.len()],
            analog_channels,
            digital_channels,
        }
    }

    /// Creates analog and digital traces from this config.
    /// Traces include ALL channels (even disabled ones) so that chunk ingestion
    /// indices match the device's channel order.
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

    /// Rebuilds a [`DeviceHandle`]'s internal traces to match this config
    /// (sample rate and channel layout).
    pub fn apply_to_handle(&self, handle: &mut DeviceHandle) {
        let (analog, digital) = self.build_traces();
        handle.set_traces(analog, digital);
    }
}

impl Default for AcquisitionConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            analog_channels: Vec::new(),
            analog_enabled: Vec::new(),
            digital_channels: Vec::new(),
            digital_enabled: Vec::new(),
        }
    }
}

/// Clamps a sample rate to a strictly positive value so the [`Timebase`]
/// invariant holds.
fn clamp_rate(hz: f64) -> f64 {
    if hz.is_finite() && hz > 0.0 { hz } else { 1.0 }
}

// ── Device acquisition ────────────────────────────────────────────────────────

/// Bundles a background acquisition future with the local display stores.
///
/// The acquisition future runs continuously on the platform's native spawner
/// (tokio `LocalSet` for native, `wasm-bindgen-futures` for web). Data flows
/// back through `data_rx` into the local stores.
pub struct DeviceAcquisition {
    pub analog: Vec<AnalogTrace>,
    pub digital: Option<DigitalTrace>,
    pub state: AcquisitionState,
    pub sample_count: usize,
    /// Sends [`AcquisitionCommand`]s to the spawned future.
    pub cmd_tx: mpsc::UnboundedSender<AcquisitionCommand>,
    /// Receives [`SampleChunk`]s from the spawned future.
    pub data_rx: mpsc::UnboundedReceiver<SampleChunk>,
    /// User-editable acquisition configuration.
    pub config: AcquisitionConfig,
}

impl DeviceAcquisition {
    pub fn drain(&mut self) {
        let chunks: Vec<SampleChunk> =
            std::iter::from_fn(|| self.data_rx.try_recv().ok()).collect();
        for chunk in &chunks {
            for (index, trace) in self.analog.iter_mut().enumerate() {
                if !self.config.analog_enabled.get(index).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(samples) = chunk.analog_channel(index) {
                    trace.push_raw(samples);
                }
            }
            if let Some(ref mut digital) = self.digital {
                if self.config.digital_enabled.iter().any(|e| *e) && !chunk.logic().is_empty() {
                    digital.push_words(chunk.logic());
                }
            }
            self.sample_count += chunk.sample_count();
        }
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }
    pub fn analog_traces(&self) -> &[AnalogTrace] {
        &self.analog
    }
    pub fn digital_trace(&self) -> Option<&DigitalTrace> {
        self.digital.as_ref()
    }
    pub fn state(&self) -> &AcquisitionState {
        &self.state
    }

    pub fn send_command(&self, cmd: AcquisitionCommand) {
        let _ = self.cmd_tx.unbounded_send(cmd);
    }

    /// Resets all traces to empty, preserving channel configuration.
    /// Called on re-run so old data is discarded before a fresh acquisition.
    pub fn reset_traces(&mut self) {
        let timebase = Timebase::new(clamp_rate(self.config.sample_rate_hz), 0.0);
        self.analog = self
            .config
            .analog_channels
            .iter()
            .map(|ch| AnalogTrace::new(ch.clone(), timebase))
            .collect();
        self.digital = if !self.config.digital_channels.is_empty() {
            Some(DigitalTrace::new(self.config.digital_channels.clone(), timebase))
        } else {
            None
        };
        self.sample_count = 0;
    }
}
