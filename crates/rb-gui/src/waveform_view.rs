//! Per-device waveform view: analog min/max envelope + digital signal rows.
//!
//! [`WaveformView`] holds the visible sample window (pan/zoom state) for one
//! connected device. Each frame, [`WaveformView::draw`] reads directly from the
//! device's stores via the [`DeviceHandle`] — no copies, no blocking I/O.
//!
//! # Waveform rendering
//! - **Analog**: one row per channel, auto-scaled to the visible min/max. The
//!   [`AnalogMipMap`] selects the pyramid level that fits `pixel_width` buckets,
//!   so draw cost is constant regardless of zoom.
//! - **Digital**: one row per channel, drawn as a step-waveform. Transitions
//!   come from [`DigitalMipMap::edges_in`], so only visible edges are visited.
//!
//! # Pan / zoom
//! - Scroll wheel over the panel: zoom in/out around the view centre.
//! - Drag: pan left/right.
//! - "Follow" checkbox: auto-scrolls to the newest samples while running.

use std::ops::Range;

use eframe::egui;
use rb_core::{AcquisitionState, DeviceHandle};
use rb_model::{AnalogTrace, DigitalTrace};

/// Height of one analog trace row in logical pixels.
const ANALOG_ROW_H: f32 = 80.0;

/// Height of one digital channel row in logical pixels.
const DIGITAL_ROW_H: f32 = 22.0;

/// Left margin reserved for channel labels inside a digital row.
const LABEL_W: f32 = 36.0;

// ── View state ────────────────────────────────────────────────────────────────

/// Pan/zoom state for one device's waveform display.
pub struct WaveformView {
    /// Index of the first visible sample.
    view_start: usize,
    /// Number of samples in the visible window (controls zoom level).
    view_samples: usize,
    /// When `true`, the view tracks the newest data while the device is running.
    auto_scroll: bool,
}

impl Default for WaveformView {
    fn default() -> Self {
        Self {
            view_start: 0,
            view_samples: 1_000,
            auto_scroll: true,
        }
    }
}

impl WaveformView {
    /// Draws the waveform for `handle`'s stores into `ui`.
    ///
    /// Reads sample data without blocking; mutates only pan/zoom state.
    pub fn draw(&mut self, ui: &mut egui::Ui, handle: &DeviceHandle) {
        let sample_count = handle.sample_count();

        // ── Status bar ────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            let (col, txt) = match handle.state() {
                AcquisitionState::Running => (egui::Color32::GREEN, "● Running"),
                AcquisitionState::Idle => (egui::Color32::GRAY, "○ Idle"),
                AcquisitionState::Stopped => (egui::Color32::GRAY, "○ Stopped"),
                AcquisitionState::Error(_) => (egui::Color32::RED, "⚠ Error"),
            };
            ui.colored_label(col, txt);
            if let AcquisitionState::Error(msg) = handle.state() {
                ui.colored_label(egui::Color32::RED, msg.as_str());
            }
            ui.separator();
            ui.label(format!("{sample_count} samples"));
            ui.separator();
            ui.checkbox(&mut self.auto_scroll, "Follow");
        });
        ui.separator();

        if sample_count == 0 {
            ui.weak("No samples yet — press ▶ in the sidebar to start acquisition.");
            return;
        }

        // ── Clamp and advance view window ─────────────────────────────────────
        self.view_samples = self.view_samples.clamp(16, sample_count);
        if self.auto_scroll && matches!(handle.state(), AcquisitionState::Running) {
            self.view_start = sample_count.saturating_sub(self.view_samples);
        }
        self.view_start = self
            .view_start
            .min(sample_count.saturating_sub(self.view_samples));
        let view_end = (self.view_start + self.view_samples).min(sample_count);
        let range = self.view_start..view_end;

        // ── Draw traces ───────────────────────────────────────────────────────
        for trace in handle.analog_traces() {
            draw_analog(ui, trace, range.clone());
            ui.add_space(2.0);
        }
        if let Some(dt) = handle.digital_trace() {
            draw_digital(ui, dt, range.clone());
        }

        // ── Pan / zoom via pointer events ─────────────────────────────────────
        // Interact retroactively with the entire area drawn above.
        let drawn_rect = ui.min_rect();
        let response = ui.interact(drawn_rect, ui.id().with("waveform"), egui::Sense::drag());

        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if response.hovered() && scroll.y != 0.0 {
            // Scroll up → zoom in (fewer visible samples), scroll down → out.
            let factor: f64 = if scroll.y > 0.0 { 0.8 } else { 1.25 };
            let center = self.view_start + self.view_samples / 2;
            let new_samples =
                ((self.view_samples as f64 * factor) as usize).clamp(16, sample_count);
            self.view_samples = new_samples;
            self.view_start = center
                .saturating_sub(new_samples / 2)
                .min(sample_count.saturating_sub(new_samples));
            self.auto_scroll = false;
        }

        if response.dragged() {
            let dx = response.drag_delta().x;
            if dx.abs() > 0.5 {
                let spx = self.view_samples as f32 / drawn_rect.width().max(1.0);
                let delta = (dx * spx) as isize;
                let max_start = sample_count.saturating_sub(self.view_samples) as isize;
                self.view_start = (self.view_start as isize - delta).clamp(0, max_start) as usize;
                self.auto_scroll = false;
            }
        }
    }
}

// ── Analog trace ──────────────────────────────────────────────────────────────

/// Renders one analog trace row using the mip-map bucket API.
fn draw_analog(ui: &mut egui::Ui, trace: &AnalogTrace, range: Range<usize>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ANALOG_ROW_H), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(18, 18, 28));

    let pixel_width = rect.width() as usize;
    let buckets = trace.buckets(range.clone(), pixel_width.max(1));
    if buckets.is_empty() {
        return;
    }

    // Auto-scale Y axis to the visible data.
    let (raw_lo, raw_hi) = buckets.iter().fold((i32::MAX, i32::MIN), |(lo, hi), b| {
        (lo.min(b.min), hi.max(b.max))
    });
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    // Add a 10 % margin so signals don't touch the top/bottom edges.
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let y_of = |phys: f64| -> f32 {
        rect.bottom() - ((phys - p_lo) / p_span * rect.height() as f64) as f32
    };

    // Zero line (when visible).
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = y_of(0.0);
        painter.line_segment(
            [egui::pos2(rect.left(), zy), egui::pos2(rect.right(), zy)],
            egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
        );
    }

    // Draw one vertical bar per bucket (min → max).
    let range_len = (range.end - range.start).max(1) as f32;
    let color = egui::Color32::from_rgb(60, 210, 120);
    for b in &buckets {
        let x = rect.left() + (b.start as f32 - range.start as f32) / range_len * rect.width();
        let y_top = y_of(trace.to_physical(b.max));
        let y_bot = y_of(trace.to_physical(b.min));
        // Give single-pixel-height buckets a minimum visual thickness.
        let (y0, y1) = if (y_bot - y_top).abs() < 1.0 {
            (y_bot - 0.5, y_bot + 0.5)
        } else {
            (y_top, y_bot)
        };
        painter.line_segment(
            [egui::pos2(x, y0), egui::pos2(x, y1)],
            egui::Stroke::new(1.0, color),
        );
    }

    // Channel label + unit in the top-left corner.
    let unit = trace.channel().unit.as_deref().unwrap_or("");
    let label = if unit.is_empty() {
        trace.channel().name.clone()
    } else {
        format!("{} [{}]", trace.channel().name, unit)
    };
    painter.text(
        rect.left_top() + egui::vec2(4.0, 4.0),
        egui::Align2::LEFT_TOP,
        &label,
        egui::FontId::monospace(11.0),
        egui::Color32::from_gray(200),
    );
}

// ── Digital trace ─────────────────────────────────────────────────────────────

/// Renders all digital channels as step-waveforms using the transition index.
fn draw_digital(ui: &mut egui::Ui, dt: &DigitalTrace, range: Range<usize>) {
    let channels = dt.channels();
    if channels.is_empty() {
        return;
    }

    let width = ui.available_width();
    let total_h = channels.len() as f32 * DIGITAL_ROW_H + 4.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, total_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(18, 18, 28));

    let range_len = (range.end - range.start).max(1) as f32;
    let signal_left = rect.left() + LABEL_W;
    let signal_width = rect.width() - LABEL_W;
    let mip = dt.transitions();
    let color = egui::Color32::from_rgb(100, 200, 255);

    let x_of = |sample: usize| -> f32 {
        signal_left + (sample.saturating_sub(range.start)) as f32 / range_len * signal_width
    };

    for (ch_idx, ch) in channels.iter().enumerate() {
        let row_top = rect.top() + ch_idx as f32 * DIGITAL_ROW_H;

        // Label on the left margin.
        painter.text(
            egui::pos2(rect.left() + 2.0, row_top + DIGITAL_ROW_H * 0.5),
            egui::Align2::LEFT_CENTER,
            &ch.name,
            egui::FontId::monospace(10.0),
            egui::Color32::from_gray(180),
        );

        let high_y = row_top + 3.0;
        let low_y = row_top + DIGITAL_ROW_H - 4.0;

        // Level at the start of the visible window and all edges within it.
        // `ch.bit` is the bit index in both the LogicWord and the mip-map.
        let bit = ch.bit as usize;
        let initial = mip.value_at(bit, range.start as u64);
        let edges = mip.edges_in(bit, range.start as u64..range.end as u64);

        let mut current_y = if initial { high_y } else { low_y };
        let mut prev_x = x_of(range.start);

        for &edge_idx in edges {
            let edge_x = x_of(edge_idx as usize);
            // Horizontal segment from the last transition to this one.
            painter.line_segment(
                [egui::pos2(prev_x, current_y), egui::pos2(edge_x, current_y)],
                egui::Stroke::new(1.5, color),
            );
            // Vertical transition.
            let next_y = if current_y == high_y { low_y } else { high_y };
            painter.line_segment(
                [egui::pos2(edge_x, current_y), egui::pos2(edge_x, next_y)],
                egui::Stroke::new(1.5, color),
            );
            current_y = next_y;
            prev_x = edge_x;
        }

        // Final horizontal segment to the right edge of the viewport.
        let end_x = x_of(range.end);
        painter.line_segment(
            [egui::pos2(prev_x, current_y), egui::pos2(end_x, current_y)],
            egui::Stroke::new(1.5, color),
        );
    }
}
