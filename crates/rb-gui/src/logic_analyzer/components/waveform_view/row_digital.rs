//! Digital signal row drawing: sparse mode (few edges) and dense mode
//! (many edges, uses MipMap).

use rb_canvas::{Canvas, RgbaColor};
use rb_model::DigitalTrace;

use crate::logic_analyzer::waveform_state::row_layout::RowDescriptor;

use super::row_base::CanvasColors;

/// Digital signal color.
pub const DIGITAL_COLOR_RGBA: RgbaColor = RgbaColor { r: 0x58, g: 0xa6, b: 0xff, a: 255 };

/// Draw a single digital row into the given canvas renderer.
pub fn build_digital_row(
    renderer: &mut dyn Canvas,
    dt: &DigitalTrace,
    range_start: usize,
    range_end: usize,
    range_len: f64,
    row: &RowDescriptor,
    signal_width: f64,
    colors: &CanvasColors,
) {
    let sig_h = row.signal_height_px;
    let bit = dt.channels().get(row.channel_index).map(|c| c.bit as usize).unwrap_or(0);
    let mip = dt.transitions();
    let rs_u64 = range_start as u64;
    let initial = mip.value_at(bit, rs_u64);
    let high_row = (sig_h * 0.25).round();
    let low_row = (sig_h * 0.75).round();
    let mid_y = (sig_h * 0.5).round();
    let full_h = low_row - high_row + 1.0;
    let initial_row = if initial { high_row } else { low_row };

    let num_pixels = signal_width.ceil() as usize;
    let edge_count = mip.edge_count_in(bit, rs_u64..range_end as u64);
    let dense = edge_count > num_pixels;

    // Background
    renderer.set_fill_style(&colors.bg);
    renderer.clear();

    // Mid line
    renderer.set_stroke_style(&colors.grid);
    renderer.set_line_width(0.5);
    renderer.clear_line_dash();
    renderer.stroke_line(0.0, mid_y, signal_width, mid_y);

    renderer.set_fill_style(&DIGITAL_COLOR_RGBA);

    if dense {
        let q = mip.query_dense(bit, rs_u64..range_end as u64, num_pixels);
        let bw = signal_width / q.has_edge.len() as f64;
        let mut cur = initial_row;
        for (x, (&has_edge, &parity)) in q.has_edge.iter().zip(q.parity.iter()).enumerate() {
            let px = (x as f64 * bw).round();
            let nx = ((x + 1) as f64 * bw).round();
            let pw = (nx - px).max(0.0);
            if pw <= 0.0 { continue; }
            if has_edge {
                renderer.fill_rect(px, high_row, pw, full_h);
            } else {
                renderer.fill_rect(px, cur, pw, 1.0);
            }
            if parity {
                cur = if (cur - high_row).abs() < 0.5 { low_row } else { high_row };
            }
        }
    } else {
        let edges: Vec<u64> = mip.edges_in(bit, rs_u64..range_end as u64).to_vec();
        let xp = |s: u64| -> f64 {
            ((s.saturating_sub(range_start as u64) as f64) / range_len * signal_width).round()
        };
        if !edges.is_empty() || initial {
            let mut cur: f64 = initial_row;
            let mut px = rs_u64;
            for &ei in &edges {
                renderer.fill_rect(xp(px), cur, xp(ei) - xp(px), 1.0);
                let next: f64 = if (cur - high_row).abs() < 0.5 { low_row } else { high_row };
                let top = cur.min(next);
                let h = (next - cur).abs() + 1.0;
                renderer.fill_rect(xp(ei), top, 1.0, h);
                cur = next;
                px = ei;
            }
            renderer.fill_rect(xp(px), cur, xp(range_end as u64) - xp(px), 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_analyzer::components::waveform_view::row_base::DARK_COLORS;
    use crate::logic_analyzer::components::waveform_view::test_utils;
    use rb_canvas::PixelCanvas;
    use rb_model::{ChannelId, DigitalChannel, DigitalTrace, Timebase};

    use test_utils::{count_pixels, make_digital_row, save_canvas_png};

    #[test]
    fn sparse_few_edges() {
        let channels = vec![DigitalChannel::new(ChannelId(0), "D0", 0)];
        let mut trace = DigitalTrace::new(channels, Timebase::new(1_000_000.0, 0.0));
        let words: Vec<u64> = vec![0b0, 0b0, 0b0, 0b1, 0b1, 0b1, 0b0, 0b0, 0b1, 0b1];
        trace.push_words(&words[..5]);
        trace.push_words(&words[5..]);

        let row = make_digital_row(40.0, 0);
        let mut canvas = PixelCanvas::new(400, 40);

        build_digital_row(&mut canvas, &trace, 0, 10, 10.0, &row, 400.0, &DARK_COLORS);

        let sig = count_pixels(&canvas, DIGITAL_COLOR_RGBA);
        assert!(sig > 0, "no digital signal pixels drawn, got {sig}");

        let edge_x = 120u32;
        let mut edge_pixels = 0;
        for y in 10..=30 {
            if canvas.pixel(edge_x, y) == DIGITAL_COLOR_RGBA {
                edge_pixels += 1;
            }
        }
        assert!(
            edge_pixels > 0,
            "vertical edge at x={edge_x}, found {edge_pixels} signal pixels in [10..30]"
        );
        save_canvas_png(&canvas, "digital_sparse.png");
    }

    #[test]
    fn dense_many_edges() {
        let channels = vec![DigitalChannel::new(ChannelId(0), "D0", 0)];
        let mut trace = DigitalTrace::new(channels, Timebase::new(1_000_000.0, 0.0));
        let words: Vec<u64> = (0..50).map(|i| if i % 2 == 0 { 0b0 } else { 0b1 }).collect();
        trace.push_words(&words[..25]);
        trace.push_words(&words[25..]);

        let row = make_digital_row(40.0, 0);
        let mut canvas = PixelCanvas::new(20, 40);

        build_digital_row(&mut canvas, &trace, 0, 50, 50.0, &row, 20.0, &DARK_COLORS);

        assert!(count_pixels(&canvas, DARK_COLORS.bg) > 0);
        let sig = count_pixels(&canvas, DIGITAL_COLOR_RGBA);
        assert!(sig > 0, "no digital signal pixels in dense mode, got {sig}");
        save_canvas_png(&canvas, "digital_dense.png");
    }
}
