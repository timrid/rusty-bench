//! Per-device acquisition state: traces, configuration, and the spawned future handle.
//!
//! [`DeviceAcquisition`] bundles a background acquisition future with local
//! display stores. Data flows through `data_rx` into per-channel traces.

use futures::channel::mpsc;
use rb_core::{AcquisitionCommand, AcquisitionState};
use rb_model::{AnalogChannel, AnalogTrace, DigitalChannel, DigitalTrace, SampleChunk, Timebase};

// ── Acquisition configuration ─────────────────────────────────────────────────

/// User-facing configuration for the next acquisition run.
///
/// Persists across Stop → Start cycles so that channel selection and sample
/// rate survive a re-run.  Built once at [`spawn_acquisition`] time from the
/// device's channel list (all channels enabled, device's native sample rate).
#[derive(Clone, Debug)]
pub struct AcquisitionConfig {
    /// Desired sample rate in Hz.  Sent via [`SetSampleRate`](AcquisitionCommand::SetSampleRate)
    /// before every [`Start`](AcquisitionCommand::Start).
    pub sample_rate_hz: f64,
    /// Per-channel enable flags, in device channel order.  A disabled channel
    /// is still present in [`DeviceAcquisition::analog`] (so labels render) but
    /// [`drain`](DeviceAcquisition::drain) skips pushing samples into it.
    pub analog_enabled: Vec<bool>,
    /// Per-digital-channel enable flags, in device channel order.
    pub digital_enabled: Vec<bool>,
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
        // Read channel metadata from current traces BEFORE replacing them.
        let analog_channels: Vec<AnalogChannel> =
            self.analog.iter().map(|t| t.channel().clone()).collect();
        let digital_channels: Option<Vec<DigitalChannel>> =
            self.digital.as_ref().map(|t| t.channels().to_vec());
        let rate = self.config.sample_rate_hz;
        let timebase = Timebase::new(rate, 0.0);
        self.analog = analog_channels
            .iter()
            .map(|ch| AnalogTrace::new(ch.clone(), timebase))
            .collect();
        self.digital = digital_channels
            .as_ref()
            .map(|chs| DigitalTrace::new(chs.clone(), timebase));
        self.sample_count = 0;
    }
}
