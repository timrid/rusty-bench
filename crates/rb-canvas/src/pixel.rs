//! `PixelCanvas` — renders to an in-memory RGBA pixel buffer.
//!
//! Uses Bresenham line drawing and midpoint circle algorithms.
//! All coordinates are rounded to the nearest integer pixel and clamped
//! to `[0, width)` / `[0, height)`.

use crate::color::RgbaColor;
use crate::traits::Canvas;

/// A canvas that renders to an in-memory RGBA pixel buffer.
///
/// Each pixel is stored as a packed `u32` (`0xRRGGBBAA`).
/// Pixel access via [`pixel`](Self::pixel) returns an [`RgbaColor`].
pub struct PixelCanvas {
    width: u32,
    height: u32,
    pixels: Vec<u32>,

    // ── State ──────────────────────────────────────────────────────────
    fill_style: RgbaColor,
    stroke_style: RgbaColor,
    line_width: f64,
    line_dash: Option<Vec<f64>>,
    global_alpha: f64,

    // Path buffer
    path: Vec<(f64, f64)>,
}

impl PixelCanvas {
    /// Create a new pixel canvas with the given dimensions.
    /// All pixels are initialized to transparent black.
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize);
        Self {
            width,
            height,
            pixels: vec![RgbaColor::TRANSPARENT.pack(); len],
            fill_style: RgbaColor::BLACK,
            stroke_style: RgbaColor::BLACK,
            line_width: 1.0,
            line_dash: None,
            global_alpha: 1.0,
            path: Vec::new(),
        }
    }

    /// Get the color of a pixel at `(x, y)`. Returns transparent
    /// if out of bounds (for robustness in tests).
    pub fn pixel(&self, x: u32, y: u32) -> RgbaColor {
        if x >= self.width || y >= self.height {
            return RgbaColor::TRANSPARENT;
        }
        RgbaColor::unpack(self.pixels[(y as usize) * (self.width as usize) + (x as usize)])
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Raw pixel buffer as `&[u32]` (packed `0xRRGGBBAA`).
    pub fn raw_pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Save the canvas as a PNG file (requires feature `png-export`).
    ///
    /// Creates the file at `path`, writing RGBA pixel data.
    #[cfg(feature = "png-export")]
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> Result<(), std::io::Error> {
        use std::io::BufWriter;

        let file = std::fs::File::create(path.as_ref())?;
        let writer = BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let rgba: Vec<u8> = self.pixels.iter()
            .flat_map(|&p| {
                let c = RgbaColor::unpack(p);
                [c.r, c.g, c.b, c.a]
            })
            .collect();

        png_writer.write_image_data(&rgba)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        png_writer.finish()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(())
    }

    // ── Internal helpers ───────────────────────────────────────────────

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some((y as usize) * (self.width as usize) + (x as usize))
    }

    fn set_pixel_blended(&mut self, x: i32, y: i32, color: RgbaColor) {
        if let Some(idx) = self.index(x, y) {
            let dst = RgbaColor::unpack(self.pixels[idx]);
            // `color` already has global_alpha factored in by apply_*_color().
            self.pixels[idx] = RgbaColor::blend(color, dst).pack();
        }
    }

    fn apply_fill_color(&self) -> RgbaColor {
        self.fill_style.with_alpha(self.global_alpha)
    }

    fn apply_stroke_color(&self) -> RgbaColor {
        self.stroke_style.with_alpha(self.global_alpha)
    }
}

/// Bresenham line from (x0,y0) to (x1,y1), calling `plot(x, y)` for
/// each pixel. The optional `dash` pattern controls dash on/off cadence
/// (e.g. `[4.0, 4.0]` means draw 4 px, skip 4 px, repeat).
///
/// This is a free function (not a method) to avoid borrow-conflicts
/// between the dash state (immutable borrow) and the plot closure's
/// mutable borrow of the canvas.
fn bresenham_line<F: FnMut(i32, i32)>(
    x0: i32, y0: i32, x1: i32, y1: i32,
    dash: Option<Vec<f64>>,
    mut plot: F,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;

    // Dash tracking
    let dash_ref = dash.as_deref();
    let mut dash_idx: usize = 0;
    let mut dash_remaining: i32 = 0;
    let mut dash_on = true;

    if let Some(pat) = dash_ref {
        if !pat.is_empty() {
            dash_remaining = pat[0].round() as i32;
            dash_on = true;
        }
    }

    let mut pixel_count: i32 = 0;
    loop {
        let should_draw = match dash_ref {
            Some(_) => dash_on,
            None => true,
        };
        if should_draw {
            plot(x, y);
        }

        // Advance dash state
        if dash_ref.is_some() {
            pixel_count += 1;
            if pixel_count >= dash_remaining && dash_remaining > 0 {
                dash_idx = (dash_idx + 1) % dash_ref.unwrap().len();
                dash_on = !dash_on;
                pixel_count = 0;
                dash_remaining = dash_ref.unwrap()[dash_idx].round() as i32;
            }
        }

        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if x == x1 { break; }
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            if y == y1 { break; }
            err += dx;
            y += sy;
        }
    }
}

impl Canvas for PixelCanvas {
    fn set_fill_style(&mut self, color: &RgbaColor) {
        self.fill_style = *color;
    }

    fn set_stroke_style(&mut self, color: &RgbaColor) {
        self.stroke_style = *color;
    }

    fn set_line_width(&mut self, width: f64) {
        self.line_width = width;
    }

    fn set_line_dash(&mut self, segments: &[f64]) {
        if segments.is_empty() {
            self.line_dash = None;
        } else {
            self.line_dash = Some(segments.to_vec());
        }
    }

    fn clear_line_dash(&mut self) {
        self.line_dash = None;
    }

    fn set_global_alpha(&mut self, alpha: f64) {
        self.global_alpha = alpha.clamp(0.0, 1.0);
    }

    fn clear(&mut self) {
        let c = self.apply_fill_color().pack();
        self.pixels.fill(c);
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let x0 = (x.round() as i32).clamp(0, self.width as i32 - 1);
        let y0 = (y.round() as i32).clamp(0, self.height as i32 - 1);
        let x1 = ((x + w).round() as i32).clamp(0, self.width as i32);
        let y1 = ((y + h).round() as i32).clamp(0, self.height as i32);

        let color = self.apply_fill_color();
        for py in y0..y1 {
            for px in x0..x1 {
                self.set_pixel_blended(px, py, color);
            }
        }
    }

    fn begin_path(&mut self) {
        self.path.clear();
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.path.push((x, y));
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.path.push((x, y));
    }

    fn stroke(&mut self) {
        if self.path.len() < 2 {
            return;
        }
        let color = self.apply_stroke_color();
        let dash = self.line_dash.clone();
        // Collect segments to avoid borrow-conflict between self.path
        // and the bresenham closure which needs &mut self.
        let segments: Vec<(i32, i32, i32, i32)> = self.path.windows(2)
            .map(|w| {
                (w[0].0.round() as i32, w[0].1.round() as i32,
                 w[1].0.round() as i32, w[1].1.round() as i32)
            })
            .collect();
        for (x0, y0, x1, y1) in segments {
            bresenham_line(x0, y0, x1, y1, dash.clone(), |px, py| {
                self.set_pixel_blended(px, py, color);
            });
        }
    }

    fn fill_circle(&mut self, x: f64, y: f64, r: f64) {
        let cx = x.round() as i32;
        let cy = y.round() as i32;
        let radius = r.round() as i32;
        if radius <= 0 {
            return;
        }

        let color = self.apply_fill_color();

        // Midpoint circle algorithm: find boundary pixels, then fill
        // horizontal scanlines between left/right edges.
        let mut edges: Vec<Vec<i32>> = vec![Vec::new(); (2 * radius + 1) as usize];

        let mut px = 0i32;
        let mut py = radius;
        let mut d = 1 - radius;

        while px <= py {
            // Eight octants → record horizontal edges per Y
            let y_offsets = [
                cy + py, cy - py,
                cy + px, cy - px,
            ];
            let x_pairs = [
                (cx - px, cx + px),
                (cx - px, cx + px),
                (cx - py, cx + py),
                (cx - py, cx + py),
            ];

            for (yo, (xl, xr)) in y_offsets.iter().zip(x_pairs.iter()) {
                let row = (*yo - (cy - radius)) as usize;
                if row < edges.len() {
                    edges[row].push(*xl);
                    edges[row].push(*xr);
                }
            }

            px += 1;
            if d < 0 {
                d += 2 * px + 1;
            } else {
                py -= 1;
                d += 2 * (px - py) + 1;
            }
        }

        // Fill each scanline
        for (row_idx, x_vals) in edges.iter_mut().enumerate() {
            if x_vals.is_empty() {
                continue;
            }
            x_vals.sort_unstable();
            x_vals.dedup();
            let y_pos = cy - radius + row_idx as i32;
            for chunk in x_vals.chunks(2) {
                if chunk.len() >= 2 {
                    let xl = chunk[0].max(0);
                    let xr = chunk[1].min(self.width as i32 - 1);
                    for xp in xl..=xr {
                        self.set_pixel_blended(xp, y_pos, color);
                    }
                }
            }
        }
    }

    fn stroke_line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        let color = self.apply_stroke_color();
        let dash = self.line_dash.clone();
        let ix1 = x1.round() as i32;
        let iy1 = y1.round() as i32;
        let ix2 = x2.round() as i32;
        let iy2 = y2.round() as i32;
        bresenham_line(ix1, iy1, ix2, iy2, dash, |px, py| {
            self.set_pixel_blended(px, py, color);
        });
    }

    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        // Placeholder: draw a small horizontal line as a text-position marker.
        // Approximate width: ~6 px per character.
        let px = x.round() as i32;
        let py = y.round() as i32;
        let approx_w = (text.len() as i32 * 6).max(4);
        let color = self.apply_fill_color();
        for dx in 0..approx_w {
            self.set_pixel_blended(px + dx, py, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_canvas_is_transparent() {
        let c = PixelCanvas::new(10, 10);
        assert_eq!(c.pixel(0, 0), RgbaColor::TRANSPARENT);
        assert_eq!(c.pixel(9, 9), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn clear_fills_all_pixels() {
        let mut c = PixelCanvas::new(4, 4);
        c.set_fill_style(&RgbaColor::from_hex("#ff0000"));
        c.clear();
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(c.pixel(x, y), RgbaColor::from_hex("#ff0000"),
                    "pixel ({x},{y}) should be red");
            }
        }
    }

    #[test]
    fn fill_rect_sets_correct_pixels() {
        let mut c = PixelCanvas::new(10, 10);
        c.set_fill_style(&RgbaColor::from_hex("#00ff00"));
        c.fill_rect(2.0, 2.0, 4.0, 4.0);
        // Inside the rect
        assert_eq!(c.pixel(2, 2), RgbaColor::from_hex("#00ff00"));
        assert_eq!(c.pixel(5, 5), RgbaColor::from_hex("#00ff00"));
        // Outside
        assert_eq!(c.pixel(0, 0), RgbaColor::TRANSPARENT);
        assert_eq!(c.pixel(6, 6), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn fill_rect_alpha_blends() {
        let mut c = PixelCanvas::new(10, 10);
        // Fill white background
        c.set_fill_style(&RgbaColor::WHITE);
        c.clear();
        // Draw 50% red over it
        c.set_fill_style(&RgbaColor::from_hex("#ff0000"));
        c.set_global_alpha(0.5);
        c.fill_rect(0.0, 0.0, 10.0, 10.0);
        // 50% red over white: R stays 255, G/B become ~128
        let p = c.pixel(5, 5);
        assert_eq!(p.r, 255, "red channel should stay 255");
        assert!(p.g > 120 && p.g < 136, "expected g~128, got {}", p.g);
        assert!(p.b > 120 && p.b < 136, "expected b~128, got {}", p.b);
        assert_eq!(p.a, 255, "result should be opaque");
    }

    #[test]
    fn stroke_line_horizontal() {
        let mut c = PixelCanvas::new(10, 10);
        c.set_stroke_style(&RgbaColor::from_hex("#ffffff"));
        c.stroke_line(2.0, 5.0, 7.0, 5.0);
        // Line pixels
        assert_eq!(c.pixel(2, 5), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(5, 5), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(7, 5), RgbaColor::from_hex("#ffffff"));
        // Above/below
        assert_eq!(c.pixel(5, 4), RgbaColor::TRANSPARENT);
        assert_eq!(c.pixel(5, 6), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn stroke_line_diagonal() {
        let mut c = PixelCanvas::new(10, 10);
        c.set_stroke_style(&RgbaColor::from_hex("#ffffff"));
        c.stroke_line(0.0, 0.0, 5.0, 5.0);
        // Diagonal should hit (0,0), (1,1), ..., (5,5)
        for i in 0..=5 {
            assert_eq!(c.pixel(i, i), RgbaColor::from_hex("#ffffff"),
                "pixel ({i},{i}) should be on diagonal");
        }
        assert_eq!(c.pixel(5, 4), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn stroke_line_dashed() {
        let mut c = PixelCanvas::new(30, 5);
        c.set_stroke_style(&RgbaColor::from_hex("#ffffff"));
        c.set_line_dash(&[4.0, 4.0]);
        // Horizontal line from (2, 2) to (27, 2) → 26 pixels, dash[4,4]
        // Expect: pixels 2-5 on, 6-9 off, 10-13 on, 14-17 off, 18-21 on, 22-25 off, 26 on...
        c.stroke_line(2.0, 2.0, 27.0, 2.0);
        // Check a few
        assert_eq!(c.pixel(2, 2), RgbaColor::from_hex("#ffffff"), "dash on at 2");
        assert_eq!(c.pixel(5, 2), RgbaColor::from_hex("#ffffff"), "dash on at 5");
        assert_eq!(c.pixel(6, 2), RgbaColor::TRANSPARENT, "dash off at 6");
        assert_eq!(c.pixel(10, 2), RgbaColor::from_hex("#ffffff"), "dash on at 10");
    }

    #[test]
    fn fill_circle_small() {
        let mut c = PixelCanvas::new(10, 10);
        c.set_fill_style(&RgbaColor::from_hex("#ff0000"));
        c.fill_circle(5.0, 5.0, 3.0);
        // Center should be filled
        assert_eq!(c.pixel(5, 5), RgbaColor::from_hex("#ff0000"));
        // Far corner should be empty
        assert_eq!(c.pixel(0, 0), RgbaColor::TRANSPARENT);
        // Edge at radius 3: (5+3,5) = (8,5) should be filled or close
        // Midpoint circle may or may not fill exactly at radius; check ~inside
        assert_eq!(c.pixel(6, 5), RgbaColor::from_hex("#ff0000"));
        // Outside
        assert_eq!(c.pixel(9, 5), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn global_alpha_affects_drawing() {
        let mut c = PixelCanvas::new(10, 10);
        // White background
        c.set_fill_style(&RgbaColor::WHITE);
        c.clear();
        // Draw black line at 50% alpha → should be gray
        c.set_stroke_style(&RgbaColor::BLACK);
        c.set_global_alpha(0.5);
        c.stroke_line(0.0, 5.0, 9.0, 5.0);
        let p = c.pixel(5, 5);
        // 50% black over white = ~127 gray (rounding may give 127 or 128)
        assert!(p.r >= 126 && p.r <= 128, "expected r~127, got {}", p.r);
        assert_eq!(p.r, p.g);
        assert_eq!(p.g, p.b);
    }

    #[test]
    fn stroke_uses_path_buffer() {
        let mut c = PixelCanvas::new(20, 20);
        c.set_stroke_style(&RgbaColor::from_hex("#ffffff"));
        // V-shape path
        c.begin_path();
        c.move_to(2.0, 2.0);
        c.line_to(10.0, 10.0);
        c.line_to(18.0, 2.0);
        c.stroke();
        // Check segments
        assert_eq!(c.pixel(2, 2), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(10, 10), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(18, 2), RgbaColor::from_hex("#ffffff"));
        // Middle of V should be empty (not filled, only stroked)
        assert_eq!(c.pixel(10, 5), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn begin_path_clears_previous() {
        let mut c = PixelCanvas::new(20, 20);
        c.set_stroke_style(&RgbaColor::from_hex("#ff0000"));
        c.begin_path();
        c.move_to(0.0, 0.0);
        c.line_to(5.0, 5.0);
        // Don't stroke — start new path
        c.begin_path();
        c.move_to(10.0, 10.0);
        c.line_to(15.0, 15.0);
        c.stroke();
        // First path was cleared, only second should render
        assert_eq!(c.pixel(10, 10), RgbaColor::from_hex("#ff0000"));
        assert_eq!(c.pixel(0, 0), RgbaColor::TRANSPARENT);
    }

    #[test]
    fn fill_text_placeholder_draws_line() {
        let mut c = PixelCanvas::new(40, 10);
        c.set_fill_style(&RgbaColor::from_hex("#ffffff"));
        c.fill_text("Hello", 2.0, 5.0);
        // Should draw ~30px horizontal line at y=5 starting at x=2
        assert_eq!(c.pixel(2, 5), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(20, 5), RgbaColor::from_hex("#ffffff"));
        assert_eq!(c.pixel(35, 5), RgbaColor::TRANSPARENT); // past ~32 px
        assert_eq!(c.pixel(5, 4), RgbaColor::TRANSPARENT); // above
        assert_eq!(c.pixel(5, 6), RgbaColor::TRANSPARENT); // below
    }

    #[test]
    fn out_of_bounds_pixel_returns_transparent() {
        let c = PixelCanvas::new(5, 5);
        assert_eq!(c.pixel(10, 10), RgbaColor::TRANSPARENT);
        assert_eq!(c.pixel(5, 0), RgbaColor::TRANSPARENT);
    }
}
