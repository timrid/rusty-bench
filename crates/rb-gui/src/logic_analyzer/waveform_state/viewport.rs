//! Viewport: pan/zoom state for the waveform display.
//!
//! Manages the visible sample window (view_start / view_samples),
//! auto-scroll behaviour, and cursor state.

use std::ops::Range;

/// Pan/zoom and cursor state for one waveform display.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    /// Index of the first visible sample.
    pub view_start: usize,
    /// Number of samples in the visible window (controls zoom level).
    pub view_samples: usize,
    /// When `true`, the view tracks the newest data while the device is running.
    pub auto_scroll: bool,
    /// Mouse hover state for the cursor line.
    pub cursor: Option<CursorState>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            view_start: 0,
            view_samples: 1_000,
            auto_scroll: true,
            cursor: None,
        }
    }
}

// ── Cursor state ──────────────────────────────────────────────────────────────

/// Mouse hover state for the Cursor Line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CursorState {
    /// Sample position under the cursor (None when mouse is outside Signal Area).
    pub sample_pos: Option<u64>,
    /// Pixel X within the Signal Area (for drawing the vertical line).
    pub px_x: Option<f64>,
    /// Whether Shift is held (enables edge snapping).
    pub shift_held: bool,
}

// ── Viewport methods ──────────────────────────────────────────────────────────

impl Viewport {
    /// Clamp the view window to valid bounds and advance if auto-scrolling.
    /// Returns the visible sample range `[start, end)`.
    pub fn clamp_view(&mut self, sample_count: usize, is_running: bool) -> Range<usize> {
        if sample_count == 0 {
            self.view_start = 0;
            return 0..0;
        }
        self.view_samples = self.view_samples.clamp(1, sample_count.max(1));
        if self.auto_scroll && is_running {
            self.view_start = sample_count.saturating_sub(self.view_samples);
        }
        self.view_start = self
            .view_start
            .min(sample_count.saturating_sub(self.view_samples));
        let view_end = (self.view_start + self.view_samples).min(sample_count);
        self.view_start..view_end
    }

    /// Update pan: delta in pixels (positive = drag right, which pans left into
    /// older samples). `canvas_width` is the width of the drawing area in pixels.
    pub fn pan(&mut self, delta_px: f32, canvas_width: f32, sample_count: usize) {
        if sample_count == 0 {
            return;
        }
        let spx = self.view_samples as f32 / canvas_width.max(1.0);
        let delta = (delta_px * spx) as isize;
        if delta == 0 {
            return;
        }
        let max_start = sample_count.saturating_sub(self.view_samples) as isize;
        self.view_start = (self.view_start as isize - delta).clamp(0, max_start) as usize;
        self.auto_scroll = false;
    }

    /// Zoom: `factor < 1.0` = zoom in (fewer visible samples),
    /// `factor > 1.0` = zoom out.
    pub fn zoom(&mut self, factor: f64, sample_count: usize) {
        if sample_count == 0 {
            return;
        }
        let center = self.view_start + self.view_samples / 2;
        let new_samples =
            ((self.view_samples as f64 * factor) as usize).clamp(16, sample_count);
        self.view_samples = new_samples;
        self.view_start = center
            .saturating_sub(new_samples / 2)
            .min(sample_count.saturating_sub(new_samples));
        // Zoom does NOT disable auto-scroll — only manual pan/drag does.
    }

    // ── Cursor management ─────────────────────────────────────────────────

    /// Update cursor position from a pixel X in the Signal Area.
    pub fn update_cursor(
        &mut self,
        px_x: f64,
        signal_width: f64,
        range_start: u64,
        range_end: u64,
    ) {
        let range_len = (range_end - range_start).max(1) as f64;
        let frac = (px_x / signal_width.max(1.0)).clamp(0.0, 1.0);
        let sample_pos = range_start + (frac * range_len) as u64;
        self.cursor = Some(CursorState {
            sample_pos: Some(sample_pos),
            px_x: Some(px_x),
            shift_held: false,
        });
    }

    /// Clear cursor (mouse left the Signal Area).
    pub fn clear_cursor(&mut self) {
        self.cursor = None;
    }

    /// Set the Shift key state for edge snapping.
    pub fn set_cursor_shift(&mut self, held: bool) {
        if let Some(ref mut c) = self.cursor {
            c.shift_held = held;
        }
    }
}
