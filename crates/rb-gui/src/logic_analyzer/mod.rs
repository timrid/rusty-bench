//! Logic Analyzer tab content: data types, acquisition, waveform state,
//! UI components, and acquisition orchestration.

pub mod acquisition;
pub mod components;
pub mod control;
pub mod decoder;
pub mod waveform_state;

use futures::channel::{mpsc, oneshot};
use rb_model::{AnalogTrace, DigitalTrace, SampleChunk};

use acquisition::AcquisitionConfig;
use decoder::DecoderConfig;
use waveform_state::WaveformState;

/// Where a tab's acquisition is in its lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquisitionState {
    /// No acquisition in progress.
    Idle,
    /// Armed and streaming samples.
    Running,
    /// Acquisition was stopped cleanly.
    Stopped,
    /// Acquisition ended in error.
    Error(String),
}

impl Default for AcquisitionState {
    fn default() -> Self {
        Self::Idle
    }
}

/// All state specific to a Logic Analyzer tab.
pub struct LogicAnalyzerContent {
    /// Acquisition configuration built from device capabilities on connect.
    /// Drives channel selection, sample rate, and trace creation.
    pub acquisition_config: AcquisitionConfig,
    /// Analog traces, one per channel. Owned by this tab.
    pub analog: Vec<AnalogTrace>,
    /// Digital trace, if the device has digital channels. Owned by this tab.
    pub digital: Option<DigitalTrace>,
    /// Current acquisition lifecycle state.
    pub acq_state: AcquisitionState,
    /// Total samples acquired in this run.
    pub sample_count: usize,
    /// Sender to stop the streaming source (drop to stop).
    /// `None` when not acquiring.
    pub data_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
    /// One-shot sender to signal the acquisition task to call stop_streaming.
    pub stop_tx: Option<oneshot::Sender<()>>,
    /// Per-tab waveform pan/zoom, row layout, and marker state.
    pub waveform_state: WaveformState,
    /// Protocol decoder configuration and annotations.
    pub decoder_config: DecoderConfig,
    /// Bumped every time this content is replaced (device switch, etc.).
    pub content_version: u64,
}

impl LogicAnalyzerContent {
    /// Returns true if this tab is currently acquiring.
    pub fn is_running(&self) -> bool {
        matches!(self.acq_state, AcquisitionState::Running)
    }

    /// Whether this content holds acquired sample data.
    pub fn has_data(&self) -> bool {
        self.sample_count > 0
    }

    /// Push a chunk into the tab's traces.
    pub fn push_chunk(&mut self, chunk: &SampleChunk) {
        acquisition::push_chunk(
            chunk,
            &mut self.analog,
            &mut self.digital,
            &self.acquisition_config,
        );
        self.sample_count += chunk.sample_count();
    }

    /// Discard all acquired samples, preserving channel configuration.
    pub fn reset_traces(&mut self) {
        let (analog, digital) = self.acquisition_config.build_traces();
        self.analog = analog;
        self.digital = digital;
        self.sample_count = 0;
    }
}

impl Default for LogicAnalyzerContent {
    fn default() -> Self {
        Self {
            acquisition_config: AcquisitionConfig::default(),
            analog: Vec::new(),
            digital: None,
            acq_state: AcquisitionState::default(),
            sample_count: 0,
            data_tx: None,
            stop_tx: None,
            waveform_state: WaveformState::default(),
            decoder_config: DecoderConfig::default(),
            content_version: 0,
        }
    }
}

// ── Content factory ──────────────────────────────────────────────────────────

/// Creates a [`TabContent::LogicAnalyzer`] with default settings.
pub fn default_content() -> crate::tab_content::TabContent {
    crate::tab_content::TabContent::LogicAnalyzer(LogicAnalyzerContent::default())
}

/// Creates a [`LogicAnalyzerContent`] from a connected device's capabilities.
/// Increments `content_version` so the UI can detect the replacement.
pub fn init_content(device: &dyn rb_device::Device) -> LogicAnalyzerContent {
    static NEXT_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LogicAnalyzerContent {
        acquisition_config: AcquisitionConfig::from_device(device),
        content_version: NEXT_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ..LogicAnalyzerContent::default()
    }
}
