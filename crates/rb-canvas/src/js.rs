//! `JsCanvasRenderer` — generates JavaScript strings for browser/webview
//! canvas elements.

use crate::color::RgbaColor;
use crate::traits::Canvas;

/// A canvas renderer that builds a JavaScript string for a `<canvas>`
/// element's 2D context.
///
/// Call [`finish`](Self::finish) to get the complete JS code block
/// (including the `getElementById` preamble and DPR setup).
pub struct JsCanvasRenderer {
    buf: String,

    // ── State ──────────────────────────────────────────────────────────
    fill_style: RgbaColor,
    stroke_style: RgbaColor,
    line_width: f64,
    line_dash: Option<Vec<f64>>,
    global_alpha: f64,
}

impl JsCanvasRenderer {
    /// Create a new JS renderer with default state.
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            fill_style: RgbaColor::BLACK,
            stroke_style: RgbaColor::BLACK,
            line_width: 1.0,
            line_dash: None,
            global_alpha: 1.0,
        }
    }

    /// Consume the renderer and return the complete JS code block.
    ///
    /// The returned string is a self-contained JS block (wrapped in `{...}`)
    /// that:
    /// 1. Looks up the canvas element by `canvas_id`
    /// 2. Sets up DPR-aware dimensions
    /// 3. Runs all recorded drawing commands
    pub fn finish(self, canvas_id: &str, sig_h: f64) -> String {
        let inner = &self.buf;
        format!(
            "{{let c=document.getElementById('{canvas_id}');\
              if(!c){{console.warn('Canvas not found: {canvas_id}');return;}}\
              var dpr=window.devicePixelRatio||1;\
              var w=c.clientWidth;\
              var h={sig_h};\
              c.width=w*dpr;c.height=h*dpr;\
              var ctx=c.getContext('2d');\
              ctx.scale(dpr,dpr);\
              {inner}}}"
        )
    }
}

impl Default for JsCanvasRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas for JsCanvasRenderer {
    fn set_fill_style(&mut self, color: &RgbaColor) {
        self.fill_style = *color;
        self.buf.push_str(&format!(
            "ctx.fillStyle='{}';",
            color.to_hex()
        ));
    }

    fn set_stroke_style(&mut self, color: &RgbaColor) {
        self.stroke_style = *color;
        self.buf.push_str(&format!(
            "ctx.strokeStyle='{}';",
            color.to_hex()
        ));
    }

    fn set_line_width(&mut self, width: f64) {
        self.line_width = width;
        self.buf.push_str(&format!("ctx.lineWidth={width};"));
    }

    fn set_line_dash(&mut self, segments: &[f64]) {
        if segments.is_empty() {
            self.line_dash = None;
            self.buf.push_str("ctx.setLineDash([]);");
        } else {
            self.line_dash = Some(segments.to_vec());
            let js_arr: String = segments.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",");
            self.buf.push_str(&format!("ctx.setLineDash([{js_arr}]);"));
        }
    }

    fn clear_line_dash(&mut self) {
        self.line_dash = None;
        self.buf.push_str("ctx.setLineDash([]);");
    }

    fn set_global_alpha(&mut self, alpha: f64) {
        self.global_alpha = alpha.clamp(0.0, 1.0);
        self.buf.push_str(&format!("ctx.globalAlpha={alpha};"));
    }

    fn clear(&mut self) {
        // fillRect over entire canvas area
        self.buf.push_str("ctx.fillRect(0,0,w,h);");
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.buf.push_str(&format!(
            "ctx.fillRect({x},{y},{w},{h});"
        ));
    }

    fn begin_path(&mut self) {
        self.buf.push_str("ctx.beginPath();");
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.buf.push_str(&format!("ctx.moveTo({x},{y});"));
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.buf.push_str(&format!("ctx.lineTo({x},{y});"));
    }

    fn stroke(&mut self) {
        self.buf.push_str("ctx.stroke();");
    }

    fn fill_circle(&mut self, x: f64, y: f64, r: f64) {
        self.buf.push_str(&format!(
            "ctx.beginPath();ctx.arc({x},{y},{r},0,6.283);ctx.fill();"
        ));
    }

    fn stroke_line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        self.buf.push_str(&format!(
            "ctx.beginPath();ctx.moveTo({x1},{y1});ctx.lineTo({x2},{y2});ctx.stroke();"
        ));
    }

    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        // Escape JS string
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
        self.buf.push_str(&format!(
            "ctx.font='9px monospace';ctx.fillText('{escaped}',{x},{y});"
        ));
    }
}
