//! Row layout: row descriptors, height management, reordering, and
//! vertical scroll state for the waveform display.

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

/// Describes one Row in the waveform canvas layout.
///
/// Rows form a flat, ordered list.  Decoder Rows can be freely interleaved with
/// Analog and Digital Rows.
#[derive(Clone, Debug, PartialEq)]
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

// ── RowLayout ─────────────────────────────────────────────────────────────────

/// Manages the ordered list of Rows in the waveform display.
#[derive(Clone, Debug, PartialEq)]
pub struct RowLayout {
    /// Ordered list of Rows in the waveform canvas.
    pub rows: Vec<RowDescriptor>,
    /// Whether the row list needs to be rebuilt from device channels.
    /// Initialized to `true` so the first `rebuild_rows` creates rows.
    pub rows_dirty: bool,
    /// Width of the left label panel in pixels (default [`LABEL_W`]).
    pub label_width: f64,
    /// Vertical scroll offset of the row area (below headers) in pixels.
    pub scroll_y: f64,
}

impl Default for RowLayout {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            rows_dirty: true,
            label_width: LABEL_W,
            scroll_y: 0.0,
        }
    }
}

impl RowLayout {
    // ── Row management ────────────────────────────────────────────────────

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

    /// Toggle visibility of the first Row matching `kind` + `channel_index`.
    pub fn toggle_row_visible_by_kind(&mut self, kind: RowKind, channel_index: usize) {
        if let Some(row) = self.rows.iter_mut()
            .find(|r| r.kind == kind && r.channel_index == channel_index)
        {
            row.visible = !row.visible;
        }
    }

    /// Move a row from `from` index to `to` index.
    /// `to` is the 0-based position in the array BEFORE removal.
    /// After removal, elements at indices > `from` shift left.
    /// `to.min(len)` handles both in-bounds and end-of-array targets.
    pub fn move_row(&mut self, from: usize, to: usize) {
        if from < self.rows.len() && to <= self.rows.len() && from != to {
            let row = self.rows.remove(from);
            // When removing an element BEFORE the target, the target
            // index shifts left by one. Compensate so the row lands
            // exactly where the visual drop gap appeared.
            let insert_pos = if from < to { to.saturating_sub(1) } else { to };
            let insert_pos = insert_pos.min(self.rows.len());
            self.rows.insert(insert_pos, row);
        }
    }

    /// Compute the target insertion position for a drag-and-drop reorder.
    ///
    /// `page_y` – page-level Y of the cursor (must be relative to the row
    ///   area's page-level top, e.g. `page_y - labels_panel_page_y`).
    /// `exclude_row` – the row being dragged (its space is preserved but it
    ///   is excluded from the midpoint calculation).
    ///
    /// Returns the 0-based insertion index: 0 = before first row,
    /// `rows.len()` = after last row.
    ///
    /// Uses the *original* row layout (un-expanded dividers) to compute
    /// midpoints, so the result is flicker-free regardless of the visual
    /// target-gap expansion.
    pub fn compute_reorder_target(&self, y_px: f64, exclude_row: usize) -> usize {
        let mut offset = 0.0;
        let mut best_target = 0usize;
        let mut best_dist = f64::MAX;

        for (i, row) in self.rows.iter().enumerate() {
            if !row.visible {
                continue;
            }
            let h = row.total_height();
            if h <= 0.0 {
                continue;
            }

            // Gap above this row (distance from cursor to top of this row).
            // Skip the gap above the excluded row — we never want to
            // target the dragged row's own position.
            if i != exclude_row {
                let gap_above_dist = (y_px - offset).abs();
                if gap_above_dist < best_dist {
                    best_dist = gap_above_dist;
                    best_target = i;
                }
            }

            offset += h;

            // Divider zone (gap below this row).
            // Also skip when the target (i+1) would be the excluded row.
            let has_next_visible = self.rows[i + 1..].iter().any(|r| r.visible);
            if has_next_visible && i + 1 != exclude_row {
                let gap_below_dist = (y_px - offset).abs();
                if gap_below_dist < best_dist {
                    best_dist = gap_below_dist;
                    best_target = i + 1;
                }
                offset += DIVIDER_H;
            }
        }

        // Gap after the last visible row.
        let gap_end_dist = (y_px - offset).abs();
        if gap_end_dist < best_dist {
            best_target = self.rows.len();
        }

        // best_target can never equal exclude_row because we skipped
        // both the gap-above and gap-below positions that would produce it.
        best_target
    }

    /// Compute the total height of all visible Rows plus dividers between them.
    pub fn total_rows_height(&self) -> f64 {
        let visible_count = self.rows.iter().filter(|r| r.visible).count();
        if visible_count == 0 {
            return 0.0;
        }
        let row_sum: f64 = self
            .rows
            .iter()
            .filter(|r| r.visible)
            .map(|r| r.total_height())
            .sum();
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

    // ── Vertical scroll ───────────────────────────────────────────────────

    /// Scroll the row area vertically by `delta_px` pixels.
    pub fn scroll_rows(&mut self, delta_px: f64) {
        let total_h = self.total_rows_height();
        let max_scroll = (total_h - 200.0).max(0.0);
        self.scroll_y = (self.scroll_y + delta_px).clamp(0.0, max_scroll);
    }
}
