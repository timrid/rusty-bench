//! Per-tab state: one GUI tab.
//!
//! Each [`TabState`] references at most one device (via [`TabSource::Device`])
//! or an imported capture (via [`TabSource::File`]). The actual device handle
//! lives in [`DeviceManager`](crate::device_manager::DeviceManager) and is
//! shared across tabs — only one tab can acquire data at a time.

use std::path::PathBuf;

use rb_device::DeviceId;

use crate::logic_analyzer::acquisition::{AcquisitionConfig, DeviceAcquisition};
use crate::logic_analyzer::view::WaveformView;
use crate::tab_content::{LogicAnalyzerContent, TabContent};

// ── Tab identifier ────────────────────────────────────────────────────────────

/// Opaque identifier for a GUI tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub(crate) u64);

// ── Tab source ────────────────────────────────────────────────────────────────

/// Where a tab's data comes from.
#[derive(Debug, Clone)]
pub enum TabSource {
    /// No data source yet (empty tab).
    None,
    /// Live device, identified by its [`DeviceId`].
    Device(DeviceId),
    /// Loaded from a file (e.g. an imported Capture).
    File(PathBuf),
}

// ── Tab state ─────────────────────────────────────────────────────────────────

/// All state owned by one GUI tab.
pub struct TabState {
    pub id: TabId,
    /// Display name shown in the tab header (device label or "Untitled").
    pub label: String,
    /// Where this tab's data comes from: a live device, an imported file, or none.
    pub source: TabSource,
    /// Device-class-specific content. Defaults to [`TabContent::LogicAnalyzer`].
    pub content: Option<TabContent>,
}

impl TabState {
    pub fn new(id: TabId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            source: TabSource::None,
            content: Some(TabContent::default()),
        }
    }

    // ── Derived accessors ─────────────────────────────────────────────────

    /// The assigned [`DeviceId`], if this tab is bound to a live device.
    pub fn assigned_device_id(&self) -> Option<&DeviceId> {
        match &self.source {
            TabSource::Device(id) => Some(id),
            _ => None,
        }
    }

    /// Sets the device assignment (or clears it).
    pub fn set_assigned_device_id(&mut self, id: Option<DeviceId>) {
        self.source = match id {
            Some(did) => TabSource::Device(did),
            None => TabSource::None,
        };
    }

    /// Returns true if this tab's content is currently running.
    pub fn is_running(&self) -> bool {
        self.content.as_ref().is_some_and(|c| c.is_running())
    }

    // ── Logic Analyzer convenience accessors ──────────────────────────────
    //
    // These expect the tab content to be `LogicAnalyzer` (the default).
    // They exist to minimise churn while only one content variant exists.
    // Once more variants are added, callers should match on `self.content`.

    /// Borrows the [`LogicAnalyzerContent`] immutably.
    fn logic_analyzer(&self) -> &LogicAnalyzerContent {
        match &self.content {
            Some(TabContent::LogicAnalyzer(la)) => la,
            _ => panic!("expected LogicAnalyzer content in tab {}", self.id.0),
        }
    }

    /// Borrows the [`LogicAnalyzerContent`] mutably.
    fn logic_analyzer_mut(&mut self) -> &mut LogicAnalyzerContent {
        match &mut self.content {
            Some(TabContent::LogicAnalyzer(la)) => la,
            _ => panic!("expected LogicAnalyzer content in tab {}", self.id.0),
        }
    }

    // Convenience delegates — these avoid `.logic_analyzer().field` noise.

    pub fn acquisition_config(&self) -> &AcquisitionConfig {
        &self.logic_analyzer().acquisition_config
    }

    pub fn acquisition_config_mut(&mut self) -> &mut AcquisitionConfig {
        &mut self.logic_analyzer_mut().acquisition_config
    }

    pub fn acquisition(&self) -> Option<&DeviceAcquisition> {
        self.logic_analyzer().acquisition.as_ref()
    }

    pub fn acquisition_mut(&mut self) -> Option<&mut DeviceAcquisition> {
        self.logic_analyzer_mut().acquisition.as_mut()
    }

    pub fn set_acquisition(&mut self, acq: Option<DeviceAcquisition>) {
        self.logic_analyzer_mut().acquisition = acq;
    }

    pub fn view(&self) -> &WaveformView {
        &self.logic_analyzer().view
    }

    pub fn view_mut(&mut self) -> &mut WaveformView {
        &mut self.logic_analyzer_mut().view
    }
}
