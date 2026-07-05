//! Per-device waveform view state: pan/zoom window, row layout, markers, and
//! decoder management.
//!
//! [`WaveformView`] holds the visible sample window (pan/zoom state) for one
//! connected device and manages the protocol-decoder lifecycle. Drawing is
//! handled by the canvas component in [`components::waveform_canvas`].
//!
//! # Pan / zoom
//! - Scroll wheel over the panel: zoom in/out around the view centre.
//! - Drag: pan left/right.
//! - "Follow" checkbox: auto-scrolls to the newest samples while running.

use std::ops::Range;

use rb_core::DeviceHandle;
use rb_model::DigitalTrace;

use crate::logic_analyzer::components::decoder_setup::DecoderConfig;

// ── Row kinds and constants ───────────────────────────────────────────────────

/// The kind of a Row in the waveform canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RowKind {
    /// Displays one analog [`Channel`](rb_model::AnalogTrace).
    #[default]
    Analog,
    /// Displays one digital/logic [`Channel`](rb_model::DigitalChannel).
    Digital,
    /// Displays [`Annotation`]s from a [`Decoder`].
    Decoder,
}

/// Default height of the Signal Area (excluding measurement zones) for each Row kind.
pub const DEFAULT_ANALOG_SIGNAL_H: f64 = 80.0;
pub const DEFAULT_DIGITAL_SIGNAL_H: f64 = 22.0;
pub const DEFAULT_DECODER_SIGNAL_H: f64 = 24.0;

/// Fixed height of a single measurement zone (above and below the Signal Area).
pub const MEASUREMENT_ZONE_H: f64 = 14.0;

/// Height of the divider drag handle between Rows.
pub const DIVIDER_H: f64 = 5.0;

/// Height of the Time Ruler header.
pub const TIME_RULER_H: f64 = 22.0;

/// Height of the Marker Bar.
pub const MARKER_BAR_H: f64 = 20.0;

/// Fixed width of the label area on the left side of each Row.
pub const LABEL_W: f64 = 36.0;

/// Returns the Row height (signal area + 2× measurement zones).
/// The divider between rows is rendered as a separate element.
pub fn row_total_height(signal_h: f64) -> f64 {
    signal_h + 2.0 * MEASUREMENT_ZONE_H
}

// ── Row descriptor ────────────────────────────────────────────────────────────

/// Describes one Row in the waveform canvas layout.
///
/// Rows form a flat, ordered list.  Decoder Rows can be freely interleaved with
/// Analog and Digital Rows.
#[derive(Clone, Debug)]
pub struct RowDescriptor {
    /// Row kind.
    pub kind: RowKind,
    /// Height of the Signal Area in pixels (without measurement zones or divider).
    pub signal_height_px: f64,
    /// For Analog/Digital: index into the device's channel list.
    /// For Decoder: index into the view's decoder list.
    pub channel_index: usize,
    /// Whether this Row is currently visible.
    pub visible: bool,
    /// For Decoder Rows: which decoder this Row references.
    pub decoder_kind: Option<DecoderKind>,
}

impl RowDescriptor {
    pub fn total_height(&self) -> f64 {
        if !self.visible {
            return 0.0;
        }
        match self.kind {
            RowKind::Decoder => self.signal_height_px,
            _ => row_total_height(self.signal_height_px),
        }
    }
}

// ── Marker types ──────────────────────────────────────────────────────────────

/// Unique identifier for a Time Marker.
pub type MarkerId = u32;

/// A user-placed time marker at a specific sample position.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeMarker {
    pub id: MarkerId,
    /// Sample position in the Sample Store.
    pub sample_pos: u64,
    /// Optional user label.
    pub label: Option<String>,
}

/// Unique identifier for a Marker Pair.
pub type PairId = u32;

/// Two linked Time Markers that display Δt and frequency.
#[derive(Clone, Debug)]
pub struct MarkerPair {
    pub id: PairId,
    pub marker_a: MarkerId,
    pub marker_b: MarkerId,
    pub label: Option<String>,
}

// ── Cursor state ──────────────────────────────────────────────────────────────

/// Mouse hover state for the Cursor Line.
#[derive(Clone, Debug, Default)]
pub struct CursorState {
    /// Sample position under the cursor (None when mouse is outside Signal Area).
    pub sample_pos: Option<u64>,
    /// Pixel X within the Signal Area (for drawing the vertical line).
    pub px_x: Option<f64>,
    /// Whether Shift is held (enables edge snapping).
    pub shift_held: bool,
}

// ── Decoder kind selector ─────────────────────────────────────────────────────

/// Which protocol decoder (if any) is attached to this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DecoderKind {
    #[default]
    None,
    Uart,
    I2c,
    Spi,
}

impl DecoderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Uart => "UART",
            Self::I2c => "I²C",
            Self::Spi => "SPI",
        }
    }
}

// ── View state ────────────────────────────────────────────────────────────────

/// Pan/zoom, row layout, markers, and optional decoder state for one device's
/// waveform display.
///
/// Manual Clone impl because `Box<dyn Decoder>` isn't Clone — the decoder
/// is rebuilt on demand after cloning.
pub struct WaveformView {
    /// Index of the first visible sample.
    pub view_start: usize,
    /// Number of samples in the visible window (controls zoom level).
    pub view_samples: usize,
    /// When `true`, the view tracks the newest data while the device is running.
    pub auto_scroll: bool,

    // ── Row layout ────────────────────────────────────────────────────────────
    /// Ordered list of Rows in the waveform canvas.
    pub rows: Vec<RowDescriptor>,
    /// Whether the row list needs to be rebuilt from device channels.
    pub rows_dirty: bool,
    /// Width of the left label panel in pixels (default [`LABEL_W`]).
    pub label_width: f64,

    // ── Markers ───────────────────────────────────────────────────────────────
    pub markers: Vec<TimeMarker>,
    pub marker_pairs: Vec<MarkerPair>,
    next_marker_id: MarkerId,
    next_pair_id: PairId,

    // ── Cursor ────────────────────────────────────────────────────────────────
    pub cursor: Option<CursorState>,

    // ── Vertical scroll ───────────────────────────────────────────────────────
    /// Vertical scroll offset of the row area (below headers) in pixels.
    pub scroll_y: f64,

    // ── Decoder config ────────────────────────────────────────────────────────
    pub decoder: DecoderConfig,
}

// Manual Clone because `Box<dyn Decoder>` isn't Clone.
impl Clone for WaveformView {
    fn clone(&self) -> Self {
        Self {
            view_start: self.view_start,
            view_samples: self.view_samples,
            auto_scroll: self.auto_scroll,
            rows: self.rows.clone(),
            rows_dirty: true,
            label_width: self.label_width,
            markers: self.markers.clone(),
            marker_pairs: self.marker_pairs.clone(),
            next_marker_id: self.next_marker_id,
            next_pair_id: self.next_pair_id,
            cursor: self.cursor.clone(),
            scroll_y: self.scroll_y,
            decoder: self.decoder.clone(),
        }
    }
}

impl Default for WaveformView {
    fn default() -> Self {
        Self {
            view_start: 0,
            view_samples: 1_000,
            auto_scroll: true,
            rows: Vec::new(),
            rows_dirty: true,
            label_width: LABEL_W,
            markers: Vec::new(),
            marker_pairs: Vec::new(),
            next_marker_id: 0,
            next_pair_id: 0,
            cursor: None,
            scroll_y: 0.0,
            decoder: DecoderConfig::default(),
        }
    }
}

impl WaveformView {
    // ── Row management ────────────────────────────────────────────────────────

    /// Rebuild the row list from device channels.  Call when a device connects
    /// or when channel metadata changes.
    pub fn rebuild_rows(&mut self, analog_count: usize, digital_ch_count: usize) {
        if !self.rows_dirty {
            return;
        }
        // Preserve existing row heights where possible.
        let old_heights: Vec<(RowKind, usize, f64)> = self
            .rows
            .iter()
            .filter(|r| r.visible)
            .map(|r| (r.kind, r.channel_index, r.signal_height_px))
            .collect();

        self.rows.clear();
        for i in 0..analog_count {
            let h = old_heights
                .iter()
                .find(|(k, ci, _)| *k == RowKind::Analog && *ci == i)
                .map(|(_, _, h)| *h)
                .unwrap_or(DEFAULT_ANALOG_SIGNAL_H);
            self.rows.push(RowDescriptor {
                kind: RowKind::Analog,
                signal_height_px: h,
                channel_index: i,
                visible: true,
                decoder_kind: None,
            });
        }
        for i in 0..digital_ch_count {
            let h = old_heights
                .iter()
                .find(|(k, ci, _)| *k == RowKind::Digital && *ci == i)
                .map(|(_, _, h)| *h)
                .unwrap_or(DEFAULT_DIGITAL_SIGNAL_H);
            self.rows.push(RowDescriptor {
                kind: RowKind::Digital,
                signal_height_px: h,
                channel_index: i,
                visible: true,
                decoder_kind: None,
            });
        }
        self.rows_dirty = false;
    }

    /// Set the Signal Area height of a Row by its index in the row list.
    pub fn set_row_height(&mut self, row_index: usize, new_height_px: f64) {
        if let Some(row) = self.rows.get_mut(row_index) {
            let min_h = match row.kind {
                RowKind::Analog => 20.0,
                RowKind::Digital => 10.0,
                RowKind::Decoder => 12.0,
            };
            row.signal_height_px = new_height_px.clamp(min_h, 400.0);
        }
    }

    /// Set the width of the left label panel in pixels.
    pub fn set_label_width(&mut self, new_width_px: f64) {
        self.label_width = new_width_px.clamp(20.0, 200.0);
    }

    /// Toggle visibility of a Row.
    pub fn toggle_row_visible(&mut self, row_index: usize) {
        if let Some(row) = self.rows.get_mut(row_index) {
            row.visible = !row.visible;
        }
    }

    /// Move a row from `from` index to `to` index.
    /// `to` is the 0-based position in the array BEFORE removal.
    /// After removal, elements at indices > `from` shift left, so when
    /// `to > from` we insert at `to - 1`.
    /// Marks rows as dirty so the next `rebuild_rows` call re-indexes channels.
    pub fn move_row(&mut self, from: usize, to: usize) {
        if from < self.rows.len() && to <= self.rows.len() && from != to {
            let row = self.rows.remove(from);
            let insert_pos = if to > from { to - 1 } else { to };
            self.rows.insert(insert_pos, row);
            self.rows_dirty = true;
        }
    }

    /// Compute the total height of all visible Rows plus dividers between them.
    pub fn total_rows_height(&self) -> f64 {
        let visible_count = self.rows.iter().filter(|r| r.visible).count();
        if visible_count == 0 {
            return 0.0;
        }
        let row_sum: f64 = self.rows.iter().filter(|r| r.visible).map(|r| r.total_height()).sum();
        row_sum + (visible_count - 1) as f64 * DIVIDER_H
    }

    /// Find the Row index at a given vertical pixel offset (relative to top of
    /// the row area, below Time Ruler + Marker Bar).
    /// Dividers between rows are treated as belonging to the row **below**
    /// (i.e. a click on the divider after row i returns i + 1).
    pub fn row_at_y(&self, y_px: f64) -> Option<usize> {
        let mut offset = 0.0;
        for (i, row) in self.rows.iter().enumerate() {
            if !row.visible {
                continue;
            }
            let h = row.total_height();
            if h <= 0.0 {
                continue;
            }
            // Row body.
            if y_px >= offset && y_px < offset + h {
                return Some(i);
            }
            offset += h;
            // Divider zone (except after the last visible row).
            let has_next_visible = self.rows[i + 1..].iter().any(|r| r.visible);
            if has_next_visible {
                if y_px >= offset && y_px < offset + DIVIDER_H {
                    return Some(i + 1);
                }
                offset += DIVIDER_H;
            }
        }
        None
    }

    /// Get the Y offset of a Row's top edge (relative to top of row area),
    /// accounting for dividers between visible rows.
    pub fn row_y_offset(&self, row_index: usize) -> f64 {
        let mut offset = 0.0;
        for (i, row) in self.rows.iter().enumerate().take(row_index) {
            if !row.visible {
                continue;
            }
            offset += row.total_height();
            // Add divider if there is another visible row after this one.
            let has_next_visible = self.rows[i + 1..].iter().any(|r| r.visible);
            if has_next_visible {
                offset += DIVIDER_H;
            }
        }
        offset
    }

    /// Returns `true` if the Row at `row_index` is visible and has the
    /// measurement zone capability (Analog or Digital, not Decoder).
    pub fn row_has_measurement_zones(&self, row_index: usize) -> bool {
        self.rows
            .get(row_index)
            .map_or(false, |r| matches!(r.kind, RowKind::Analog | RowKind::Digital))
    }

    // ── Marker management ─────────────────────────────────────────────────────

    /// Add a new TimeMarker at the given sample position.
    pub fn add_marker(&mut self, sample_pos: u64) -> MarkerId {
        let id = self.next_marker_id;
        self.next_marker_id += 1;
        self.markers.push(TimeMarker {
            id,
            sample_pos,
            label: None,
        });
        id
    }

    /// Move an existing marker to a new sample position.
    pub fn move_marker(&mut self, id: MarkerId, new_pos: u64) {
        if let Some(m) = self.markers.iter_mut().find(|m| m.id == id) {
            m.sample_pos = new_pos;
        }
    }

    /// Remove a marker (and any pairs that reference it).
    pub fn remove_marker(&mut self, id: MarkerId) {
        self.markers.retain(|m| m.id != id);
        self.marker_pairs
            .retain(|p| p.marker_a != id && p.marker_b != id);
    }

    /// Create a Marker Pair from two existing markers.
    pub fn add_marker_pair(&mut self, marker_a: MarkerId, marker_b: MarkerId) -> Option<PairId> {
        if !self.markers.iter().any(|m| m.id == marker_a)
            || !self.markers.iter().any(|m| m.id == marker_b)
            || marker_a == marker_b
        {
            return None;
        }
        let id = self.next_pair_id;
        self.next_pair_id += 1;
        self.marker_pairs.push(MarkerPair {
            id,
            marker_a,
            marker_b,
            label: None,
        });
        Some(id)
    }

    /// Remove a Marker Pair (does not remove the underlying markers).
    pub fn remove_marker_pair(&mut self, id: PairId) {
        self.marker_pairs.retain(|p| p.id != id);
    }

    /// Find the sample position of a marker by ID.
    pub fn marker_pos(&self, id: MarkerId) -> Option<u64> {
        self.markers.iter().find(|m| m.id == id).map(|m| m.sample_pos)
    }

    // ── Cursor management ─────────────────────────────────────────────────────

    /// Update cursor position from a pixel X in the Signal Area.
    /// `signal_width` is the pixel width of the Signal Area (canvas width minus label).
    /// `range_start` / `range_end` are the current visible sample range.
    pub fn update_cursor(&mut self, px_x: f64, signal_width: f64, range_start: u64, range_end: u64) {
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

    // ── Vertical scroll ───────────────────────────────────────────────────────

    /// Scroll the row area vertically by `delta_px` pixels.
    pub fn scroll_rows(&mut self, delta_px: f64) {
        let total_h = self.total_rows_height();
        let max_scroll = (total_h - 200.0).max(0.0); // min visible area 200px
        self.scroll_y = (self.scroll_y + delta_px).clamp(0.0, max_scroll);
    }

    // ── Decoder (delegates to DecoderConfig) ───────────────────────────────────

    /// Feed new digital samples to the decoder.
    pub fn feed_decoder(&mut self, dt: &DigitalTrace) {
        self.decoder.feed(dt);
    }

    /// Clamp the view window to valid bounds and advance if auto-scrolling.
    /// Returns the visible sample range `[start, end)`.
    pub fn clamp_view(&mut self, sample_count: usize, is_running: bool) -> Range<usize> {
        if sample_count == 0 {
            self.view_start = 0;
            return 0..0;
        }
        self.view_samples = self.view_samples.clamp(16, sample_count);
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

    /// Zoom: `factor < 1.0` = zoom in (fewer visible samples), `factor > 1.0` = zoom out.
    pub fn zoom(&mut self, factor: f64, sample_count: usize) {
        if sample_count == 0 {
            return;
        }
        let center = self.view_start + self.view_samples / 2;
        let new_samples = ((self.view_samples as f64 * factor) as usize).clamp(16, sample_count);
        self.view_samples = new_samples;
        self.view_start = center
            .saturating_sub(new_samples / 2)
            .min(sample_count.saturating_sub(new_samples));
        // Zoom does NOT disable auto-scroll — only manual pan/drag does.
    }

    /// Update decoder config based on handle's digital trace.
    /// Call after `clamp_view` to ensure decoder has latest data.
    pub fn update_decoder(&mut self, handle: &DeviceHandle) {
        self.decoder.update_from_handle(handle);
    }
}
