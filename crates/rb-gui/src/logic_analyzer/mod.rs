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
    /// Bumped every time this content is replaced (device switch, etc.).
    /// Used by the UI to detect that signals need reloading from tab state.
    pub content_version: u64,
}

impl LogicAnalyzerContent {
    /// Returns true if this tab is currently acquiring.
    pub fn is_running(&self) -> bool {
        matches!(
            self.acquisition.as_ref().map(|a| a.state()),
            Some(AcquisitionState::Running)
        )
    }

    /// Whether this content holds acquired sample data (either in an active
    /// acquisition or in the returned [`DeviceHandle`]).
    pub fn has_data(&self, device_handle: Option<&rb_core::DeviceHandle>) -> bool {
        if let Some(acq) = &self.acquisition {
            if acq.sample_count() > 0 {
                return true;
            }
        }
        device_handle.is_some_and(|h| h.sample_count() > 0)
    }
}

impl Default for LogicAnalyzerContent {
    fn default() -> Self {
        Self {
            acquisition_config: AcquisitionConfig::default(),
            acquisition: None,
            view: WaveformView::default(),
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
    // Use a thread-local counter so each call gets a unique version.
    static NEXT_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LogicAnalyzerContent {
        acquisition_config: AcquisitionConfig::from_device(device),
        content_version: NEXT_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ..LogicAnalyzerContent::default()
    }
}
