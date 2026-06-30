//! Logic Analyzer tab content: data types, acquisition, waveform view,
//! UI components, and acquisition orchestration.

pub mod acquisition;
pub mod components;
pub mod control;
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

// ── Content factory ──────────────────────────────────────────────────────────

/// Creates a [`TabContent::LogicAnalyzer`] with default settings.
pub fn default_content() -> crate::tab_content::TabContent {
    crate::tab_content::TabContent::LogicAnalyzer(LogicAnalyzerContent::default())
}

/// Creates a [`LogicAnalyzerContent`] from a connected device's capabilities.
pub fn init_content(device: &dyn rb_device::Device) -> LogicAnalyzerContent {
    LogicAnalyzerContent {
        acquisition_config: AcquisitionConfig::from_device(device),
        ..LogicAnalyzerContent::default()
    }
}
