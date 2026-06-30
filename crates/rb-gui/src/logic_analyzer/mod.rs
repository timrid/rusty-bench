//! Logic Analyzer tab content: data types, acquisition, waveform view,
//! and UI components.

pub mod acquisition;
pub mod components;
pub mod view;

use rb_core::AcquisitionState;

use acquisition::{AcquisitionConfig, DeviceAcquisition};
use view::WaveformView;

/// All state specific to a Logic Analyzer tab.
pub struct LogicAnalyzerContent {
    /// Acquisition configuration built from device capabilities on connect.
    /// Drives channel selection, sample rate, and trace creation.
    pub acquisition_config: AcquisitionConfig,
    /// Active acquisition, if running or stopped.
    pub acquisition: Option<DeviceAcquisition>,
    /// Per-tab waveform pan/zoom/marker state.
    pub view: WaveformView,
}

impl LogicAnalyzerContent {
    /// Returns true if this tab is currently acquiring.
    pub fn is_running(&self) -> bool {
        matches!(
            self.acquisition.as_ref().map(|a| a.state()),
            Some(AcquisitionState::Running)
        )
    }
}

impl Default for LogicAnalyzerContent {
    fn default() -> Self {
        Self {
            acquisition_config: AcquisitionConfig::default(),
            acquisition: None,
            view: WaveformView::default(),
        }
    }
}
