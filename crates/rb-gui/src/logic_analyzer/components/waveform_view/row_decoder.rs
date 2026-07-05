//! Decoder row drawing: renders protocol decoder annotations as
//! colored blocks with text labels.

use rb_canvas::{Canvas, RgbaColor};
use rb_decode::{Annotation, AnnotationKind};

use crate::logic_analyzer::waveform_state::row_layout::RowDescriptor;

use super::row_base::CanvasColors;

/// Draw a single decoder row into the given canvas renderer.
pub fn build_decoder_row(
    renderer: &mut dyn Canvas,
    annotations: &[Annotation],
    range_start: usize,
    range_end: usize,
    range_len: f64,
    row: &RowDescriptor,
    colors: &CanvasColors,
) {
    let sig_h = row.signal_height_px;
    renderer.set_fill_style(&colors.decoder_bg);
    renderer.clear();

    let data_color = RgbaColor { r: 0x1f, g: 0x3a, b: 0x6b, a: 255 };
    let addr_color = RgbaColor { r: 0x6b, g: 0x3d, b: 0x1f, a: 255 };
    let frame_color = RgbaColor { r: 0x2d, g: 0x2d, b: 0x2d, a: 255 };
    let error_color = RgbaColor { r: 0x6b, g: 0x1f, b: 0x1f, a: 255 };

    for ann in annotations {
        if ann.range.end <= range_start || ann.range.start >= range_end {
            continue;
        }
        let rs = range_start;
        let x0 = ann.range.start.saturating_sub(rs) as f64 / range_len;
        let x1 = ann.range.end.min(range_end).saturating_sub(rs) as f64 / range_len;
        let ww = (x1 - x0).max(0.001);
        let color = match ann.kind {
            AnnotationKind::Data => &data_color,
            AnnotationKind::Address => &addr_color,
            AnnotationKind::Frame => &frame_color,
            AnnotationKind::Error => &error_color,
        };
        renderer.set_fill_style(color);
        renderer.fill_rect(x0, 1.0, ww, sig_h - 2.0);
        if ww > 0.03 {
            renderer.set_fill_style(&colors.decoder_text);
            renderer.fill_text(&ann.label, x0 + 2.0, sig_h * 0.5 + 3.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_analyzer::components::waveform_view::row_base::DARK_COLORS;
    use crate::logic_analyzer::components::waveform_view::test_utils;
    use rb_canvas::PixelCanvas;
    use rb_decode::AnnotationKind;

    use test_utils::save_canvas_png;

    #[test]
    fn decoder_empty_annotations() {
        let row = test_utils::make_analog_row(40.0); // reusing, decoder_kind ignored
        let mut row_dec = row.clone();
        row_dec.kind = crate::logic_analyzer::waveform_state::row_layout::RowKind::Decoder;
        let mut canvas = PixelCanvas::new(400, 40);

        build_decoder_row(&mut canvas, &[], 0, 100, 100.0, &row_dec, &DARK_COLORS);

        // Decoder background should be painted.
        assert!(test_utils::any_pixel_is(&canvas, DARK_COLORS.decoder_bg));
        save_canvas_png(&canvas, "decoder_empty.png");
    }

    #[test]
    fn decoder_single_annotation() {
        let mut row = test_utils::make_analog_row(40.0);
        row.kind = crate::logic_analyzer::waveform_state::row_layout::RowKind::Decoder;
        let mut canvas = PixelCanvas::new(400, 40);

        let ann = rb_decode::Annotation {
            range: 10..50,
            kind: AnnotationKind::Data,
            label: "0x42".into(),
            data_byte: Some(0x42),
        };

        build_decoder_row(&mut canvas, &[ann], 0, 100, 100.0, &row, &DARK_COLORS);

        // Should contain non-background pixels for the data block.
        let mut found_color = false;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let p = canvas.pixel(x, y);
                if p != DARK_COLORS.decoder_bg
                    && p != DARK_COLORS.grid
                    && p != RgbaColor::TRANSPARENT
                {
                    found_color = true;
                    break;
                }
            }
        }
        assert!(found_color, "decoder annotation should draw colored blocks");
        save_canvas_png(&canvas, "decoder_single.png");
    }
}
