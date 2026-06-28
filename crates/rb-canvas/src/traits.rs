//! The `Canvas` trait — abstract 2D drawing surface.

use crate::color::RgbaColor;

/// Abstract canvas for 2D drawing operations.
///
/// Modelled on a subset of the HTML Canvas 2D Context API.
/// Coordinates are in canvas pixel space (origin top-left, Y-down).
pub trait Canvas {
    // ── State ──────────────────────────────────────────────────────────

    /// Set the color used for `fill_rect` and `fill_circle`.
    fn set_fill_style(&mut self, color: &RgbaColor);

    /// Set the color used for `stroke`, `stroke_line`.
    fn set_stroke_style(&mut self, color: &RgbaColor);

    /// Set stroke line width in pixels. Widths < 1.0 are treated as 1 px.
    fn set_line_width(&mut self, width: f64);

    /// Set a dash pattern for stroked lines, e.g. `&[4.0, 4.0]`.
    fn set_line_dash(&mut self, segments: &[f64]);

    /// Clear any active dash pattern (solid lines).
    fn clear_line_dash(&mut self);

    /// Set global alpha multiplier (0.0–1.0) applied to all drawing.
    fn set_global_alpha(&mut self, alpha: f64);

    // ── Drawing ────────────────────────────────────────────────────────

    /// Fill the entire canvas with the current `fill_style`.
    fn clear(&mut self);

    /// Fill a rectangle with the current `fill_style` (respects `global_alpha`).
    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64);

    /// Begin a new path (clears any previous un-stroked path).
    fn begin_path(&mut self);

    /// Move the path cursor to `(x, y)` without drawing.
    fn move_to(&mut self, x: f64, y: f64);

    /// Add a line segment from the current cursor to `(x, y)`.
    fn line_to(&mut self, x: f64, y: f64);

    /// Stroke the current path with `stroke_style` (respects `line_width`,
    /// `line_dash`, `global_alpha`).
    fn stroke(&mut self);

    /// Draw a filled circle centered at `(x, y)` with radius `r`,
    /// using the current `fill_style`.
    fn fill_circle(&mut self, x: f64, y: f64, r: f64);

    /// Convenience: stroke a single line segment from `(x1,y1)` to `(x2,y2)`
    /// with the current stroke settings. Equivalent to
    /// `begin_path(); move_to(x1,y1); line_to(x2,y2); stroke()`.
    fn stroke_line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64);

    /// Draw text at `(x, y)`. The exact rendering is implementation-defined
    /// (placeholder in `PixelCanvas`, real text in `JsCanvasRenderer`).
    fn fill_text(&mut self, text: &str, x: f64, y: f64);
}
