//! Shared row-drawing helpers and row-level interaction hooks.
//!
//! Contains the grid/background/zero-line drawing helpers shared by all
//! row types, plus `use_row_resize` and `use_row_reorder` hooks (adapted
//! to operate on `WaveformState` instead of the old `WaveformView`).

use dioxus::prelude::*;
use rb_canvas::{Canvas, RgbaColor};

use crate::logic_analyzer::waveform_state::WaveformState;

/// Theme-aware canvas colors (re-export for row drawers).
pub struct CanvasColors {
    pub bg: RgbaColor,
    pub grid: RgbaColor,
    pub zero_line: RgbaColor,
    pub decoder_bg: RgbaColor,
    pub decoder_text: RgbaColor,
}

pub const DARK_COLORS: CanvasColors = CanvasColors {
    bg: RgbaColor { r: 0x0d, g: 0x11, b: 0x17, a: 255 },
    grid: RgbaColor { r: 0x1a, g: 0x1a, b: 0x2e, a: 255 },
    zero_line: RgbaColor { r: 0x30, g: 0x36, b: 0x3d, a: 255 },
    decoder_bg: RgbaColor { r: 0x16, g: 0x1b, b: 0x22, a: 255 },
    decoder_text: RgbaColor { r: 0xc9, g: 0xd1, b: 0xd9, a: 255 },
};

pub const LIGHT_COLORS: CanvasColors = CanvasColors {
    bg: RgbaColor { r: 0xff, g: 0xff, b: 0xff, a: 255 },
    grid: RgbaColor { r: 0xe5, g: 0xe7, b: 0xeb, a: 255 },
    zero_line: RgbaColor { r: 0xd1, g: 0xd5, b: 0xdb, a: 255 },
    decoder_bg: RgbaColor { r: 0xf3, g: 0xf4, b: 0xf6, a: 255 },
    decoder_text: RgbaColor { r: 0x37, g: 0x41, b: 0x51, a: 255 },
};

/// Draw background, grid lines, and zero line shared by all row types.
pub fn draw_row_background(
    canvas: &mut dyn Canvas,
    sig_h: f64,
    signal_width: f64,
    p_lo: f64,
    p_hi: f64,
    p_span: f64,
    colors: &CanvasColors,
) {
    // Background
    canvas.set_fill_style(&colors.bg);
    canvas.clear();

    // Grid
    canvas.set_stroke_style(&colors.grid);
    canvas.set_line_width(0.5);
    canvas.clear_line_dash();
    for i in 0..=5 {
        let gy = i as f64 / 5.0 * sig_h;
        canvas.stroke_line(0.0, gy, signal_width, gy);
    }

    // Zero line
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = sig_h - ((0.0 - p_lo) / p_span * sig_h);
        canvas.set_stroke_style(&colors.zero_line);
        canvas.set_line_width(1.0);
        canvas.set_line_dash(&[4.0, 4.0]);
        canvas.stroke_line(0.0, zy, signal_width, zy);
        canvas.clear_line_dash();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  RowResize – divider drag resizing
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the row-height resize interaction (dragging the divider handle).
#[derive(Clone, Copy, PartialEq)]
pub struct RowResizeState {
    drag_row: Signal<Option<usize>>,
    start_y: Signal<f64>,
    start_h: Signal<f64>,
    live_h: Signal<Option<f64>>,
}

/// Create the resize-interaction state.  Call once at the top of the component.
pub fn use_row_resize() -> RowResizeState {
    RowResizeState {
        drag_row: use_signal(|| None),
        start_y: use_signal(|| 0.0),
        start_h: use_signal(|| 0.0),
        live_h: use_signal(|| None),
    }
}

impl RowResizeState {
    /// Call from the divider's `onmousedown`.  Begins a resize drag.
    pub fn begin(&mut self, row_idx: usize, sig_h: f64, page_y: f64) {
        self.drag_row.set(Some(row_idx));
        self.start_y.set(page_y);
        self.start_h.set(sig_h);
    }

    /// Call from the container's `onmousemove`.  Updates the row height
    /// live so the canvas redraws at the new size immediately.
    pub fn handle_mousemove(
        &mut self,
        page_y: f64,
        mut wf_state: Signal<WaveformState>,
        mut data_version: Signal<u64>,
    ) {
        let Some(ri) = *self.drag_row.read() else { return; };
        let dy = page_y - *self.start_y.read();
        let new_h = (*self.start_h.read() + dy).max(10.0);
        self.live_h.set(Some(new_h));
        wf_state.write().row_layout.set_row_height(ri, new_h);
        data_version += 1;
    }

    /// Commit the resize on mouse-up (persists the current height).
    pub fn commit(
        &mut self,
        mut wf_state: Signal<WaveformState>,
        mut data_version: Signal<u64>,
    ) {
        if let (Some(ri), Some(h)) = (*self.drag_row.read(), *self.live_h.read()) {
            wf_state.write().row_layout.set_row_height(ri, h);
            data_version += 1;
        }
        self.drag_row.set(None);
        self.live_h.set(None);
    }

    /// Cancel / cleanup on mouse-leave (persists whatever was last set).
    pub fn cancel(
        &mut self,
        mut wf_state: Signal<WaveformState>,
        mut data_version: Signal<u64>,
    ) {
        if let (Some(ri), Some(h)) = (*self.drag_row.read(), *self.live_h.read()) {
            wf_state.write().row_layout.set_row_height(ri, h);
            data_version += 1;
        }
        self.drag_row.set(None);
        self.live_h.set(None);
    }

    /// Whether any resize is currently in progress.
    pub fn is_active(&self) -> bool {
        self.drag_row.read().is_some()
    }

    /// Whether the given row index is the one being resized.
    pub fn is_resizing(&self, row_idx: usize) -> bool {
        *self.drag_row.read() == Some(row_idx)
    }

    /// The live signal height for a row during resize, or `None` if
    /// this row is not being resized.
    pub fn effective_sig_h(&self, row_idx: usize) -> Option<f64> {
        if self.is_resizing(row_idx) {
            *self.live_h.read()
        } else {
            None
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  RowReorder – label drag reordering (drag-and-drop)
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the row-reorder interaction (dragging a row label).
#[derive(Clone, Copy, PartialEq)]
pub struct RowReorderState {
    drag_row: Signal<Option<usize>>,
    insert_at: Signal<Option<usize>>,
    /// X offset within the label element where the click happened.
    click_offset_x: Signal<f64>,
    /// Y offset within the label element where the click happened.
    click_offset_y: Signal<f64>,
    /// Current page-X of the cursor (for positioning the floating clone).
    cursor_page_x: Signal<f64>,
    /// Current page-Y of the cursor (for positioning the floating clone).
    cursor_page_y: Signal<f64>,
    /// Total height of the dragged row (for gap visualization).
    source_row_height: Signal<f64>,
}

/// Create the reorder-interaction state.  Call once at the top of the component.
pub fn use_row_reorder() -> RowReorderState {
    RowReorderState {
        drag_row: use_signal(|| None),
        insert_at: use_signal(|| None),
        click_offset_x: use_signal(|| 0.0),
        click_offset_y: use_signal(|| 0.0),
        cursor_page_x: use_signal(|| 0.0),
        cursor_page_y: use_signal(|| 0.0),
        source_row_height: use_signal(|| 0.0),
    }
}

impl RowReorderState {
    /// Call from the label's `onmousedown`.  Begins a reorder drag.
    ///
    /// `row_idx` – which row is being dragged.
    /// `click_element_x` / `click_element_y` – offset within the label where
    ///   the click happened (makes the floating clone "stick" to the cursor).
    /// `page_x` / `page_y` – page-level cursor coordinates at the moment
    ///   of the click (so the floating clone appears at the cursor immediately,
    ///   not at (0,0)).
    /// `row_height` – total height of the row (for the target gap).
    pub fn begin(
        &mut self,
        row_idx: usize,
        click_element_x: f64,
        click_element_y: f64,
        page_x: f64,
        page_y: f64,
        row_height: f64,
    ) {
        self.drag_row.set(Some(row_idx));
        self.click_offset_x.set(click_element_x);
        self.click_offset_y.set(click_element_y);
        // Position the floating clone at the cursor immediately.
        self.cursor_page_x.set(page_x);
        self.cursor_page_y.set(page_y);
        self.source_row_height.set(row_height);
    }

    /// Call from the container's `onmousemove`.
    ///
    /// `page_x` / `page_y` — page-level cursor coordinates (for the
    ///   floating label).  `unscrolled_y` — Y in the row layout's
    ///   unscrolled coordinate space (for target computation).
    pub fn handle_mousemove(
        &mut self,
        page_x: f64,
        page_y: f64,
        unscrolled_y: f64,
        wf_state: Signal<WaveformState>,
    ) {
        let Some(from_ri) = *self.drag_row.read() else { return; };
        self.cursor_page_x.set(page_x);
        self.cursor_page_y.set(page_y);

        // Compute target insertion position based on the UNSCROLLED row
        // layout.  Targets are computed from row midpoints, excluding
        // the dragged row, so the result is flicker-free.
        let v = wf_state.read();
        let target = v.row_layout.compute_reorder_target(unscrolled_y, from_ri);
        self.insert_at.set(Some(target));
    }

    /// Commit the reorder on mouse-up.
    pub fn commit(
        &mut self,
        mut wf_state: Signal<WaveformState>,
        mut data_version: Signal<u64>,
    ) {
        let from = *self.drag_row.read();
        let to = *self.insert_at.read();
        if let (Some(from), Some(to)) = (from, to) {
            if from != to {
                wf_state.write().row_layout.move_row(from, to);
            }
        }
        self.drag_row.set(None);
        self.insert_at.set(None);
        self.click_offset_x.set(0.0);
        self.click_offset_y.set(0.0);
        self.cursor_page_x.set(0.0);
        self.cursor_page_y.set(0.0);
        self.source_row_height.set(0.0);
        data_version += 1;
    }

    /// Cancel on mouse-leave / Escape without committing.
    pub fn cancel(&mut self, mut data_version: Signal<u64>) {
        self.drag_row.set(None);
        self.insert_at.set(None);
        self.click_offset_x.set(0.0);
        self.click_offset_y.set(0.0);
        self.cursor_page_x.set(0.0);
        self.cursor_page_y.set(0.0);
        self.source_row_height.set(0.0);
        data_version += 1;
    }

    /// Whether any reorder is currently in progress.
    pub fn is_active(&self) -> bool {
        self.drag_row.read().is_some()
    }

    /// Whether the given row index is the one being dragged.
    pub fn is_dragging(&self, row_idx: usize) -> bool {
        *self.drag_row.read() == Some(row_idx)
    }

    /// The index of the row currently being dragged, if any.
    pub fn dragged_row_index(&self) -> Option<usize> {
        *self.drag_row.read()
    }

    /// The current target insert position during a drag, for visual feedback.
    pub fn target_pos(&self) -> Option<usize> {
        *self.insert_at.read()
    }

    // ── Accessors for floating-label positioning ──────────────────────

    /// X offset within the label where the click happened.
    pub fn click_offset_x(&self) -> f64 {
        *self.click_offset_x.read()
    }

    /// Y offset within the label where the click happened.
    pub fn click_offset_y(&self) -> f64 {
        *self.click_offset_y.read()
    }

    /// Current page-X of the cursor during a drag.
    pub fn cursor_page_x(&self) -> f64 {
        *self.cursor_page_x.read()
    }

    /// Current page-Y of the cursor during a drag.
    pub fn cursor_page_y(&self) -> f64 {
        *self.cursor_page_y.read()
    }

    /// Total height of the dragged row, for rendering the source gap
    /// and the target insertion slot.
    pub fn source_row_height(&self) -> f64 {
        *self.source_row_height.read()
    }


}

// ═══════════════════════════════════════════════════════════════════════════════
//  CanvasPan – waveform pan drag
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the pan (horizontal drag) interaction on the waveform.
#[derive(Clone, Copy, PartialEq)]
pub struct CanvasPanState {
    active: Signal<bool>,
    grab_sample: Signal<Option<u64>>,
}

/// Create the pan-interaction state.  Call once at the top of the component.
pub fn use_canvas_pan() -> CanvasPanState {
    CanvasPanState {
        active: use_signal(|| false),
        grab_sample: use_signal(|| None),
    }
}

impl CanvasPanState {
    /// Call from the container's `onmousedown`.  Computes the sample under
    /// the cursor as the grab anchor.
    pub fn begin(
        &mut self,
        element_x: f64,
        canvas_width: f64,
        wf_state: Signal<WaveformState>,
    ) {
        let v = wf_state.read();
        let gs = if canvas_width > 0.0 {
            let frac = (element_x / canvas_width).clamp(0.0, 1.0);
            Some(v.viewport.view_start as u64 + (frac * v.viewport.view_samples as f64) as u64)
        } else {
            None
        };
        self.grab_sample.set(gs);
        self.active.set(true);
    }

    /// Call from the container's `onmousemove`.  Pans the view so the
    /// grab sample follows the mouse 1:1.
    pub fn handle_mousemove(
        &mut self,
        element_x: f64,
        canvas_width: f64,
        mut wf_state: Signal<WaveformState>,
        sample_count: usize,
        mut data_version: Signal<u64>,
    ) {
        if !*self.active.read() {
            return;
        }
        if let Some(gs) = *self.grab_sample.read() {
            let cw = canvas_width.max(1.0);
            let frac = (element_x / cw).clamp(0.0, 1.0);
            let offset = (frac * wf_state.read().viewport.view_samples as f64) as u64;
            let new_vs = gs.saturating_sub(offset);
            let mut v = wf_state.write();
            let max_vs = sample_count.saturating_sub(v.viewport.view_samples);
            v.viewport.view_start = (new_vs as usize).min(max_vs);
            v.viewport.auto_scroll = false;
            data_version += 1;
        }
    }

    /// End the pan on mouse-up.
    pub fn end(&mut self) {
        self.active.set(false);
        self.grab_sample.set(None);
    }

    /// Whether a pan is currently in progress.
    pub fn is_active(&self) -> bool {
        *self.active.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use std::cell::RefCell;
    use std::rc::Rc;

    // ── Helper components that extract hook state ─────────────────────────

    #[component]
    fn ResizeCapture(hook_out: Rc<RefCell<Option<RowResizeState>>>) -> Element {
        let resize = use_row_resize();
        *hook_out.borrow_mut() = Some(resize);
        rsx! { div {} }
    }

    #[component]
    fn ReorderCapture(hook_out: Rc<RefCell<Option<RowReorderState>>>) -> Element {
        let reorder = use_row_reorder();
        *hook_out.borrow_mut() = Some(reorder);
        rsx! { div {} }
    }

    #[component]
    fn PanCapture(hook_out: Rc<RefCell<Option<CanvasPanState>>>) -> Element {
        let pan = use_canvas_pan();
        *hook_out.borrow_mut() = Some(pan);
        rsx! { div {} }
    }

    // ── RowResizeState ────────────────────────────────────────────────────

    #[test]
    fn resize_initial_state() {
        let out = Rc::new(RefCell::new(None));
        let mut vdom = VirtualDom::new_with_props(
            ResizeCapture,
            ResizeCaptureProps { hook_out: out.clone() },
        );
        vdom.rebuild_in_place();
        while vdom.wait_for_work().now_or_never().is_some() {
            vdom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        let resize = out.take().expect("hook not captured");
        assert!(!resize.is_active(), "resize should not be active initially");
        assert!(!resize.is_resizing(0));
        assert_eq!(resize.effective_sig_h(0), None);
    }

    // ── RowReorderState ───────────────────────────────────────────────────

    #[test]
    fn reorder_initial_state() {
        let out = Rc::new(RefCell::new(None));
        let mut vdom = VirtualDom::new_with_props(
            ReorderCapture,
            ReorderCaptureProps { hook_out: out.clone() },
        );
        vdom.rebuild_in_place();
        while vdom.wait_for_work().now_or_never().is_some() {
            vdom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        let reorder = out.take().expect("hook not captured");
        assert!(!reorder.is_active(), "reorder should not be active initially");
        assert!(!reorder.is_dragging(0));
        assert_eq!(reorder.target_pos(), None);
    }

    // ── CanvasPanState ────────────────────────────────────────────────────

    #[test]
    fn pan_initial_state() {
        let out = Rc::new(RefCell::new(None));
        let mut vdom = VirtualDom::new_with_props(
            PanCapture,
            PanCaptureProps { hook_out: out.clone() },
        );
        vdom.rebuild_in_place();
        while vdom.wait_for_work().now_or_never().is_some() {
            vdom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        let pan = out.take().expect("hook not captured");
        assert!(!pan.is_active(), "pan should not be active initially");
    }
}
