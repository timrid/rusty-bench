//! Analog signal row drawing: per-sample mode (close zoom) and
//! envelope mode (zoomed out).

use rb_canvas::{Canvas, RgbaColor};
use rb_model::AnalogTrace;

use crate::logic_analyzer::waveform_state::row_layout::RowDescriptor;

use super::row_base::{draw_row_background, CanvasColors};

/// Per-channel analog signal colors.
pub const ANALOG_COLORS_RGBA: [RgbaColor; 8] = [
    RgbaColor { r: 0xfa, g: 0xcc, b: 0x15, a: 255 },
    RgbaColor { r: 0x60, g: 0xa5, b: 0xfa, a: 255 },
    RgbaColor { r: 0xf8, g: 0x71, b: 0x71, a: 255 },
    RgbaColor { r: 0x34, g: 0xd3, b: 0x99, a: 255 },
    RgbaColor { r: 0xc0, g: 0x84, b: 0xfc, a: 255 },
    RgbaColor { r: 0xfb, g: 0x92, b: 0x3c, a: 255 },
    RgbaColor { r: 0x2d, g: 0xd4, b: 0xbf, a: 255 },
    RgbaColor { r: 0xf4, g: 0x72, b: 0xb6, a: 255 },
];

/// Draw a single analog row into the given canvas renderer.
/// Dispatches to per-sample or envelope mode based on zoom level.
pub fn build_analog_row(
    renderer: &mut dyn Canvas,
    trace: &AnalogTrace,
    range_start: usize,
    range_end: usize,
    range_len: f64,
    row: &RowDescriptor,
    signal_width: f64,
    colors: &CanvasColors,
) {
    let sig_h = row.signal_height_px;
    let color = &ANALOG_COLORS_RGBA[row.channel_index % ANALOG_COLORS_RGBA.len()];
    let samples_per_px = range_len / signal_width.max(1.0);

    if samples_per_px < 1.0 {
        build_analog_per_sample(renderer, trace, range_start, range_end,
                                range_len, sig_h, signal_width, color, samples_per_px, colors);
    } else {
        build_analog_envelope(renderer, trace, range_start, range_end, range_len,
                              sig_h, color, signal_width, colors);
    }
}

/// Per-sample line/point rendering for close-up zoom.
fn build_analog_per_sample(
    canvas: &mut dyn Canvas, trace: &AnalogTrace,
    range_start: usize, range_end: usize, range_len: f64,
    sig_h: f64, signal_width: f64, color: &RgbaColor, samples_per_px: f64,
    colors: &CanvasColors,
) {
    let store = trace.store();
    let raw = store.raw();
    let r_start = range_start.min(raw.len());
    let r_end = range_end.min(raw.len());
    if r_start >= r_end {
        return;
    }
    let samples = &raw[r_start..r_end];

    let (raw_lo, raw_hi) = samples.iter()
        .fold((i32::MAX, i32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let show_dots = samples_per_px < 0.1;

    draw_row_background(canvas, sig_h, signal_width, p_lo, p_hi, p_span, colors);

    let inv_range = 1.0 / range_len.max(1.0);
    canvas.set_stroke_style(color);
    canvas.set_line_width(1.0);
    canvas.begin_path();
    let mut first = true;
    for (i, &v) in samples.iter().enumerate() {
        let sample_idx = r_start + i;
        let px = (sample_idx as f64 - range_start as f64) * inv_range * signal_width;
        let py = sig_h - ((trace.to_physical(v).clamp(p_lo, p_hi) - p_lo) / p_span * sig_h);
        if first {
            canvas.move_to(px, py);
            first = false;
        } else {
            canvas.line_to(px, py);
        }
    }
    canvas.stroke();

    if show_dots {
        canvas.set_fill_style(color);
        for (i, &v) in samples.iter().enumerate() {
            let sample_idx = r_start + i;
            let px = ((sample_idx as f64 - range_start as f64) * inv_range * signal_width).round();
            let py = (sig_h - ((trace.to_physical(v).clamp(p_lo, p_hi) - p_lo) / p_span * sig_h)).round();
            canvas.fill_circle(px, py, 2.5);
        }
    }
}

/// Envelope (min/max fill) rendering for zoomed-out views.
fn build_analog_envelope(
    canvas: &mut dyn Canvas, trace: &AnalogTrace,
    range_start: usize, range_end: usize, range_len: f64,
    sig_h: f64, color: &RgbaColor, signal_width: f64,
    colors: &CanvasColors,
) {
    let pixel_count = signal_width.ceil() as usize;
    let range = range_start..range_end;
    let buckets = trace.envelope_buckets(range.clone(), pixel_count);
    if buckets.is_empty() {
        canvas.set_fill_style(&colors.bg);
        canvas.clear();
        return;
    }

    let (raw_lo, raw_hi) = buckets.iter()
        .filter(|b| b.min != 0 || b.max != 0)
        .fold((i32::MAX, i32::MIN), |(lo, hi), b| (lo.min(b.min), hi.max(b.max)));
    if raw_lo == i32::MAX {
        canvas.set_fill_style(&colors.bg);
        canvas.clear();
        return;
    }
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    draw_row_background(canvas, sig_h, signal_width, p_lo, p_hi, p_span, colors);

    let n_c: usize = signal_width.round() as usize;
    let mut c_min = vec![f64::INFINITY; n_c];
    let mut c_max = vec![f64::NEG_INFINITY; n_c];

    for i in 0..buckets.len() {
        let b = &buckets[i];
        let min_p = trace.to_physical(b.min).clamp(p_lo, p_hi);
        let max_p = trace.to_physical(b.max).clamp(p_lo, p_hi);
        let bx = ((b.start as f64 - range_start as f64) / range_len) * signal_width;
        let nx = if i + 1 < buckets.len() {
            ((buckets[i + 1].start as f64 - range_start as f64) / range_len) * signal_width
        } else {
            signal_width + 1.0
        };
        let c0 = (bx.floor() as usize).min(n_c.saturating_sub(1));
        let c1 = (nx.ceil() as usize).min(n_c);
        for c in c0..c1 {
            c_min[c] = c_min[c].min(min_p);
            c_max[c] = c_max[c].max(max_p);
        }
    }

    canvas.set_fill_style(color);
    canvas.set_global_alpha(0.8);
    for c in 0..n_c {
        if c_min[c] <= c_max[c] {
            let y0 = (sig_h - ((c_max[c] - p_lo) / p_span * sig_h)).round();
            let y1 = (sig_h - ((c_min[c] - p_lo) / p_span * sig_h)).round();
            let h = (y1 - y0).max(1.0);
            canvas.fill_rect(c as f64, y0, 1.0, h);
        }
    }
    canvas.set_global_alpha(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_analyzer::components::waveform_view::row_base::DARK_COLORS;
    use crate::logic_analyzer::components::waveform_view::test_utils;
    use rb_canvas::PixelCanvas;
    use rb_model::{AnalogChannel, AnalogFormat, AnalogTrace, ChannelId, Timebase};

    use test_utils::{assert_grid_line_at, count_pixels, make_analog_row, save_canvas_png};

    // ═══════════════════════════════════════════════════════════════════════
    //  Analog – per-sample mode
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn per_sample_sine_no_dots() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let amplitude = 10_000i32;
        let data: Vec<i32> = (0..50)
            .map(|i| {
                let phase = i as f64 * 2.0 / 50.0 * 2.0 * std::f64::consts::PI;
                (phase.sin() * amplitude as f64) as i32
            })
            .collect();
        trace.push_raw(&data[..25]);
        trace.push_raw(&data[25..]);

        let row = make_analog_row(80.0);
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(400, 80);

        build_analog_row(&mut canvas, &trace, 0, 50, 50.0, &row, 400.0, &DARK_COLORS);

        assert_grid_line_at(&canvas, 0, 200, DARK_COLORS.grid);
        let signal_px = count_pixels(&canvas, color);
        assert!(signal_px > 20, "expected signal polyline, found {signal_px} pixels");

        let mid_x = 200u32;
        let mut found_signal_at_mid = false;
        for y in 0..canvas.height() {
            if canvas.pixel(mid_x, y) == color {
                found_signal_at_mid = true;
                break;
            }
        }
        assert!(found_signal_at_mid, "signal polyline not found at x=200");
        save_canvas_png(&canvas, "analog_per_sample_sine.png");
    }

    #[test]
    fn per_sample_with_dots() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let amplitude = 5_000i32;
        let data: Vec<i32> = (0..10)
            .map(|i| {
                let phase = i as f64 * 2.0 / 10.0 * 2.0 * std::f64::consts::PI;
                (phase.sin() * amplitude as f64) as i32
            })
            .collect();
        trace.push_raw(&data);

        let row = make_analog_row(80.0);
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(200, 80);

        build_analog_row(&mut canvas, &trace, 0, 10, 10.0, &row, 200.0, &DARK_COLORS);

        let signal_px = count_pixels(&canvas, color);
        assert!(signal_px > 50, "expected dots with many signal pixels, got {signal_px}");
        assert_grid_line_at(&canvas, 0, 100, DARK_COLORS.grid);
        save_canvas_png(&canvas, "analog_per_sample_dots.png");
    }

    #[test]
    fn per_sample_empty() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let row = make_analog_row(80.0);
        let _color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(200, 80);

        build_analog_row(&mut canvas, &trace, 0, 0, 1.0, &row, 200.0, &DARK_COLORS);

        let signal_px = count_pixels(&canvas, _color);
        assert_eq!(signal_px, 0);
        save_canvas_png(&canvas, "analog_per_sample_empty.png");
    }

    #[test]
    fn per_sample_zero_line() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let amplitude = 3_000i32;
        let data: Vec<i32> = (0..20)
            .map(|i| {
                let phase = i as f64 * 1.0 / 20.0 * 2.0 * std::f64::consts::PI;
                (phase.sin() * amplitude as f64) as i32
            })
            .collect();
        trace.push_raw(&data);

        let row = make_analog_row(80.0);
        let _color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(200, 80);

        build_analog_row(&mut canvas, &trace, 0, 20, 20.0, &row, 200.0, &DARK_COLORS);

        assert!(
            test_utils::any_pixel_is(&canvas, DARK_COLORS.zero_line),
            "dashed zero line should be visible"
        );
        save_canvas_png(&canvas, "analog_per_sample_zero_line.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Analog – envelope mode
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn envelope_sine() {
        let amplitude = 20_000i32;
        let data: Vec<i32> = (0..1000)
            .map(|i| {
                let phase = i as f64 * 10.0 / 1000.0 * 2.0 * std::f64::consts::PI;
                (phase.sin() * amplitude as f64) as i32
            })
            .collect();
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::new(0.001, 0.0));
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        trace.push_raw(&data[..500]);
        trace.push_raw(&data[500..]);

        let row = make_analog_row(80.0);
        let mut canvas = PixelCanvas::new(800, 80);

        build_analog_row(&mut canvas, &trace, 0, 1000, 1000.0, &row, 800.0, &DARK_COLORS);

        assert_grid_line_at(&canvas, 0, 400, DARK_COLORS.grid);

        let bg_count = count_pixels(&canvas, DARK_COLORS.bg);
        assert!(
            bg_count < canvas.width() as usize * canvas.height() as usize,
            "envelope should draw over background"
        );

        let mut min_y = canvas.height();
        let mut max_y = 0u32;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let p = canvas.pixel(x, y);
                if p != DARK_COLORS.bg && p != DARK_COLORS.grid && p != RgbaColor::TRANSPARENT {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let span = max_y - min_y;
        assert!(span > 30, "envelope should span >30 px, got {span}");

        assert!(
            test_utils::any_pixel_is(&canvas, DARK_COLORS.zero_line),
            "zero line should appear for AC signal"
        );
        save_canvas_png(&canvas, "analog_envelope_sine.png");
    }

    #[test]
    fn envelope_empty() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let row = make_analog_row(80.0);
        let _color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(800, 80);

        build_analog_row(&mut canvas, &trace, 0, 0, 1.0, &row, 800.0, &DARK_COLORS);

        assert_eq!(canvas.pixel(400, 40), RgbaColor::TRANSPARENT);
        save_canvas_png(&canvas, "analog_envelope_empty.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  MipMap pipeline
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn mipmap_pipeline_incremental() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let mut trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));
        let total = 800usize;
        for chunk_start in (0..total).step_by(200) {
            let chunk_end = (chunk_start + 200).min(total);
            let chunk: Vec<i32> = (chunk_start..chunk_end)
                .map(|i| ((i as f64 * 0.05).sin() * 5000.0) as i32)
                .collect();
            trace.push_raw(&chunk);
        }

        let row = make_analog_row(80.0);
        let _color = ANALOG_COLORS_RGBA[2];
        let mut canvas = PixelCanvas::new(600, 80);

        build_analog_row(&mut canvas, &trace, 0, total, total as f64, &row, 600.0, &DARK_COLORS);

        assert_grid_line_at(&canvas, 0, 300, DARK_COLORS.grid);

        let mut min_y = canvas.height();
        let mut max_y = 0u32;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let p = canvas.pixel(x, y);
                if p != DARK_COLORS.bg && p != DARK_COLORS.grid && p != RgbaColor::TRANSPARENT {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let span = max_y.saturating_sub(min_y);
        assert!(span > 20, "mip-map envelope span >20 px, got {span} (y {min_y}..{max_y})");
        save_canvas_png(&canvas, "analog_mipmap_pipeline.png");
    }
}
