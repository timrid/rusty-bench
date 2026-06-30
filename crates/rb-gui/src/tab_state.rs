//! Per-tab state: one GUI tab.
//!
//! Each [`TabState`] references at most one device (via [`TabSource::Device`])
//! or an imported capture (via [`TabSource::File`]). The actual device handle
//! lives in [`DeviceManager`](crate::device_manager::DeviceManager) and is
//! shared across tabs — only one tab can acquire data at a time.

use std::path::PathBuf;

use rb_device::DeviceId;

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

    // ── Content access ───────────────────────────────────────────────────

    /// Borrows the [`LogicAnalyzerContent`] immutably.
    ///
    /// # Panics
    /// Panics if this tab's content is not [`TabContent::LogicAnalyzer`].
    /// Only call when the tab is known to be a Logic Analyzer tab.
    pub fn logic_analyzer(&self) -> &LogicAnalyzerContent {
        match &self.content {
            Some(TabContent::LogicAnalyzer(la)) => la,
            _ => panic!("expected LogicAnalyzer content in tab {}", self.id.0),
        }
    }

    /// Borrows the [`LogicAnalyzerContent`] mutably.
    ///
    /// # Panics
    /// Panics if this tab's content is not [`TabContent::LogicAnalyzer`].
    /// Only call when the tab is known to be a Logic Analyzer tab.
    pub fn logic_analyzer_mut(&mut self) -> &mut LogicAnalyzerContent {
        match &mut self.content {
            Some(TabContent::LogicAnalyzer(la)) => la,
            _ => panic!("expected LogicAnalyzer content in tab {}", self.id.0),
        }
    }
}
