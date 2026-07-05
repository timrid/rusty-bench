//! Shared test helpers for waveform_view canvas tests.
//! Only compiled in `#[cfg(test)]` mode.

#![cfg(test)]

use rb_canvas::pixel::PixelCanvas;
use rb_canvas::RgbaColor;

use crate::logic_analyzer::waveform_state::row_layout::{RowDescriptor, RowKind};

/// Count how many pixels in the canvas match `color` exactly.
pub fn count_pixels(canvas: &PixelCanvas, color: RgbaColor) -> usize {
    let mut n = 0;
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            if canvas.pixel(x, y) == color {
                n += 1;
            }
        }
    }
    n
}

/// Check that a horizontal line of grid-colored pixels exists at row `y`
/// spanning at least `min_len` pixels.
pub fn assert_grid_line_at(canvas: &PixelCanvas, y: u32, min_len: u32, grid_color: RgbaColor) {
    let mut run = 0u32;
    for x in 0..canvas.width() {
        if canvas.pixel(x, y) == grid_color {
            run += 1;
        } else {
            run = 0;
        }
        if run >= min_len {
            return;
        }
    }
    panic!("no grid line at y={y} with length ≥ {min_len}");
}

/// Check that any pixel in the canvas matches `color`.
pub fn any_pixel_is(canvas: &PixelCanvas, color: RgbaColor) -> bool {
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            if canvas.pixel(x, y) == color {
                return true;
            }
        }
    }
    false
}

/// Save a `PixelCanvas` as a PNG file in `target/test-screenshots/`.
pub fn save_canvas_png(canvas: &PixelCanvas, name: &str) {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("test-screenshots");
    std::fs::create_dir_all(&out_dir).expect("create screenshot dir");
    canvas.save_png(out_dir.join(name)).expect("save png");
}

/// Create an analog RowDescriptor for testing.
pub fn make_analog_row(sig_h: f64) -> RowDescriptor {
    RowDescriptor {
        kind: RowKind::Analog,
        signal_height_px: sig_h,
        channel_index: 0,
        visible: true,
        decoder_kind: None,
    }
}

/// Create a digital RowDescriptor for testing.
pub fn make_digital_row(sig_h: f64, ch_idx: usize) -> RowDescriptor {
    RowDescriptor {
        kind: RowKind::Digital,
        signal_height_px: sig_h,
        channel_index: ch_idx,
        visible: true,
        decoder_kind: None,
    }
}
