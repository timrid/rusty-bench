//! Waveform state: ViewModel layer for the waveform display.
//!
//! Contains the pan/zoom viewport, row layout, and marker management.
//! These are pure data structures — no Dioxus components here.

pub mod marker;
pub mod row_layout;
pub mod viewport;

use marker::MarkerSet;
use row_layout::RowLayout;
use viewport::Viewport;

/// Bundles the three waveform-state sub-structs into a single unit.
///
/// Owned by [`LogicAnalyzerContent`](crate::logic_analyzer::LogicAnalyzerContent)
/// and passed as a `Signal<WaveformState>` into the UI components.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveformState {
    pub viewport: Viewport,
    pub row_layout: RowLayout,
    pub marker_set: MarkerSet,
}

impl WaveformState {
    /// Convenience: clamp view and rebuild rows from enabled channel counts.
    ///
    /// `analog_count` and `digital_ch_count` should be the counts of
    /// *acquisition-enabled* channels, not total device channels.
    pub fn clamp_and_rebuild(
        &mut self,
        sample_count: usize,
        is_running: bool,
        analog_count: usize,
        digital_ch_count: usize,
    ) {
        self.viewport.clamp_view(sample_count, is_running);
        self.row_layout.rebuild_rows(analog_count, digital_ch_count);
    }
}
