//! Per-session state: one GUI tab.
//!
//! Each [`SessionState`] references at most one device by [`DeviceId`].
//! The actual device handle lives in [`DeviceManager`](crate::device_manager::DeviceManager)
//! and is shared across sessions — only one session can acquire data at a time.

use rb_core::AcquisitionState;
use rb_device::DeviceId;

use crate::device_acquisition::{AcquisitionConfig, DeviceAcquisition};
use crate::waveform_state::WaveformView;

// ── Session identifier ────────────────────────────────────────────────────────

/// Opaque identifier for a GUI session (one tab = one session = one device).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub(crate) u64);

// ── Session state ─────────────────────────────────────────────────────────────

/// All state owned by one GUI session (one tab).
pub struct SessionState {
    pub id: SessionId,
    /// Display name shown in the tab (device label or "Untitled").
    pub label: String,
    /// The [`DeviceId`] of the device assigned to this session.
    /// `None` until the user picks a device from the dropdown.
    pub assigned_device_id: Option<DeviceId>,
    /// Acquisition configuration built from device capabilities on connect.
    /// Drives channel selection, sample rate, and trace creation.
    pub acquisition_config: AcquisitionConfig,
    /// Active acquisition, if running or stopped.
    pub acquisition: Option<DeviceAcquisition>,
    /// Per-session waveform pan/zoom/marker state.
    pub view: WaveformView,
}

impl SessionState {
    pub fn new(id: SessionId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            assigned_device_id: None,
            acquisition_config: AcquisitionConfig::default(),
            acquisition: None,
            view: WaveformView::default(),
        }
    }

    /// Returns true if this session is currently acquiring.
    pub fn is_running(&self) -> bool {
        matches!(
            self.acquisition.as_ref().map(|a| a.state()),
            Some(AcquisitionState::Running)
        )
    }
}
