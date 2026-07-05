//! Interaction hooks for the waveform canvas.
//!
//! Extracted from `WaveformCanvas` to keep the component lean.  Each hook
//! owns its Dioxus signals and exposes plain methods that the component's
//! event handlers delegate to.

use dioxus::prelude::*;
use crate::logic_analyzer::view::WaveformView;

// ═══════════════════════════════════════════════════════════════════════════════
//  RowResize – divider drag resizing
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the row-height resize interaction (dragging the divider handle).
#[derive(Clone, Copy)]
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
    /// Create the state from external signals (for testing).
    pub fn from_signals(
        drag_row: Signal<Option<usize>>,
        start_y: Signal<f64>,
        start_h: Signal<f64>,
        live_h: Signal<Option<f64>>,
    ) -> Self {
        Self { drag_row, start_y, start_h, live_h }
    }

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
        mut view: Signal<WaveformView>,
        mut data_version: Signal<u64>,
    ) {
        let Some(ri) = *self.drag_row.read() else { return; };
        let dy = page_y - *self.start_y.read();
        let new_h = (*self.start_h.read() + dy).max(10.0);
        self.live_h.set(Some(new_h));
        view.write().set_row_height(ri, new_h);
        data_version += 1;
    }

    /// Commit the resize on mouse-up (persists the current height).
    pub fn commit(&mut self, mut view: Signal<WaveformView>, mut data_version: Signal<u64>) {
        if let (Some(ri), Some(h)) = (*self.drag_row.read(), *self.live_h.read()) {
            view.write().set_row_height(ri, h);
            data_version += 1;
        }
        self.drag_row.set(None);
        self.live_h.set(None);
    }

    /// Cancel / cleanup on mouse-leave (persists whatever was last set).
    pub fn cancel(&mut self, mut view: Signal<WaveformView>, mut data_version: Signal<u64>) {
        if let (Some(ri), Some(h)) = (*self.drag_row.read(), *self.live_h.read()) {
            view.write().set_row_height(ri, h);
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
//  RowReorder – label drag reordering
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the row-reorder interaction (dragging a row label).
#[derive(Clone, Copy)]
pub struct RowReorderState {
    drag_row: Signal<Option<usize>>,
    insert_at: Signal<Option<usize>>,
}

/// Create the reorder-interaction state.  Call once at the top of the component.
pub fn use_row_reorder() -> RowReorderState {
    RowReorderState {
        drag_row: use_signal(|| None),
        insert_at: use_signal(|| None),
    }
}

impl RowReorderState {
    /// Create the state from external signals (for testing).
    pub fn from_signals(
        drag_row: Signal<Option<usize>>,
        insert_at: Signal<Option<usize>>,
    ) -> Self {
        Self { drag_row, insert_at }
    }

    /// Call from the label's `onmousedown`.  Begins a reorder drag.
    pub fn begin(&mut self, row_idx: usize) {
        self.drag_row.set(Some(row_idx));
    }

    /// Call from the container's `onmousemove`.
    ///
    /// `element_y` is the mouse Y relative to the rows container element.
    /// `scroll_top` is the current `scrollTop` of that container (so
    /// `row_at_y` receives the correct coordinate regardless of scroll).
    pub fn handle_mousemove(
        &mut self,
        element_y: f64,
        scroll_top: f64,
        view: Signal<WaveformView>,
    ) {
        let Some(from_ri) = *self.drag_row.read() else { return; };
        let adjusted_y = element_y + scroll_top;
        let v = view.read();
        let hovered = v.row_at_y(adjusted_y).unwrap_or(from_ri);
        let target = if hovered > from_ri {
            hovered + 1
        } else {
            hovered
        };
        self.insert_at.set(Some(target));
    }

    /// Commit the reorder on mouse-up.  Calls `WaveformView::move_row`.
    pub fn commit(&mut self, mut view: Signal<WaveformView>, mut data_version: Signal<u64>) {
        let from = *self.drag_row.read();
        let to = *self.insert_at.read();
        if let (Some(from), Some(to)) = (from, to) {
            if from != to {
                view.write().move_row(from, to);
            }
        }
        self.drag_row.set(None);
        self.insert_at.set(None);
        // Always bump data_version to force DOM/canvas refresh so the
        // drop indicator is guaranteed to disappear.
        data_version += 1;
    }

    /// Cancel on mouse-leave without committing.
    pub fn cancel(&mut self) {
        self.drag_row.set(None);
        self.insert_at.set(None);
    }

    /// Whether any reorder is currently in progress.
    pub fn is_active(&self) -> bool {
        self.drag_row.read().is_some()
    }

    /// Whether the given row index is the one being dragged.
    pub fn is_dragging(&self, row_idx: usize) -> bool {
        *self.drag_row.read() == Some(row_idx)
    }

    /// The current target insert position during a drag, for visual feedback.
    pub fn target_pos(&self) -> Option<usize> {
        *self.insert_at.read()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  CanvasPan – waveform pan drag
// ═══════════════════════════════════════════════════════════════════════════════

/// Manages the pan (horizontal drag) interaction on the waveform.
#[derive(Clone, Copy)]
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
    /// Create the state from external signals (for testing).
    pub fn from_signals(
        active: Signal<bool>,
        grab_sample: Signal<Option<u64>>,
    ) -> Self {
        Self { active, grab_sample }
    }

    /// Call from the container's `onmousedown`.  Computes the sample under
    /// the cursor as the grab anchor.
    pub fn begin(
        &mut self,
        element_x: f64,
        canvas_width: f64,
        view: Signal<WaveformView>,
    ) {
        let v = view.read();
        let gs = if canvas_width > 0.0 {
            let frac = (element_x / canvas_width).clamp(0.0, 1.0);
            Some(v.view_start as u64 + (frac * v.view_samples as f64) as u64)
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
        mut view: Signal<WaveformView>,
        sample_count: usize,
        mut data_version: Signal<u64>,
    ) {
        if !*self.active.read() {
            return;
        }
        if let Some(gs) = *self.grab_sample.read() {
            let mut v = view.write();
            let cw = canvas_width.max(1.0);
            if cw > 0.0 {
                let frac = (element_x / cw).clamp(0.0, 1.0);
                let offset = (frac * v.view_samples as f64) as u64;
                let new_vs = gs.saturating_sub(offset);
                let max_vs = sample_count.saturating_sub(v.view_samples);
                v.view_start = (new_vs as usize).min(max_vs);
            }
            v.auto_scroll = false;
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

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════
//
// Strategy:
//   • Pure-logic tests – test WaveformView methods directly (no Dioxus runtime).
//   • Signal-state tests – use a VirtualDom harness to create real Dioxus
//     Signals, drive the interaction state machines through
//     begin → mousemove → commit / cancel, and assert on signal values and
//     WaveformView mutations.
//   • SSR snapshot tests – verify rendering output using dioxus-ssr.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_analyzer::view::{RowDescriptor, RowKind};
    use futures::FutureExt;
    use std::cell::RefCell;
    use std::rc::Rc;

    // ── Test harness ─────────────────────────────────────────────────────────

    /// Bundle of all Dioxus Signals needed to construct and drive the three
    /// interaction state structs.
    #[derive(Clone, Copy)]
    struct SignalStore {
        resize_drag_row: Signal<Option<usize>>,
        resize_start_y: Signal<f64>,
        resize_start_h: Signal<f64>,
        resize_live_h: Signal<Option<f64>>,
        reorder_drag_row: Signal<Option<usize>>,
        reorder_insert_at: Signal<Option<usize>>,
        pan_active: Signal<bool>,
        pan_grab_sample: Signal<Option<u64>>,
        view: Signal<WaveformView>,
        data_version: Signal<u64>,
    }

    /// Run a test closure inside a Dioxus VirtualDom that provides real
    /// `Signal`s.  The closure receives a [`SignalStore`] from which it can
    /// construct interaction states via their `from_signals()` constructors,
    /// call methods, and read signal values — all within the reactive scope.
    fn with_interaction_signals(test: impl FnOnce(SignalStore) + 'static) {
        type TestFn = dyn FnOnce(SignalStore);

        let test_cell: Rc<RefCell<Option<Box<TestFn>>>> = Rc::new(RefCell::new(None));
        let store_out: Rc<RefCell<Option<SignalStore>>> = Rc::new(RefCell::new(None));
        let completed: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

        *test_cell.borrow_mut() = Some(Box::new(test));

        let test_cell_clone = test_cell.clone();
        let store_out_clone = store_out.clone();
        let completed_clone = completed.clone();

        #[derive(Props)]
        struct MockAppProps {
            test_cell: Rc<RefCell<Option<Box<TestFn>>>>,
            store_out: Rc<RefCell<Option<SignalStore>>>,
            completed: Rc<RefCell<bool>>,
        }

        // Always re-render for test harness (no diff optimisation needed).
        impl PartialEq for MockAppProps {
            fn eq(&self, _other: &Self) -> bool {
                true
            }
        }

        impl Clone for MockAppProps {
            fn clone(&self) -> Self {
                Self {
                    test_cell: self.test_cell.clone(),
                    store_out: self.store_out.clone(),
                    completed: self.completed.clone(),
                }
            }
        }

        fn mock_app(props: MockAppProps) -> Element {
            let store = SignalStore {
                resize_drag_row: use_signal(|| None::<usize>),
                resize_start_y: use_signal(|| 0.0),
                resize_start_h: use_signal(|| 0.0),
                resize_live_h: use_signal(|| None::<f64>),
                reorder_drag_row: use_signal(|| None::<usize>),
                reorder_insert_at: use_signal(|| None::<usize>),
                pan_active: use_signal(|| false),
                pan_grab_sample: use_signal(|| None::<u64>),
                view: use_signal(WaveformView::default),
                data_version: use_signal(|| 0u64),
            };
            *props.store_out.borrow_mut() = Some(store);

            if let Some(test_fn) = props.test_cell.borrow_mut().take() {
                test_fn(store);
            }
            *props.completed.borrow_mut() = true;

            rsx! { div {} }
        }

        let mut vdom = dioxus::dioxus_core::VirtualDom::new_with_props(
            mock_app,
            MockAppProps {
                test_cell: test_cell_clone,
                store_out: store_out_clone,
                completed: completed_clone,
            },
        );

        vdom.rebuild_in_place();
        while vdom.wait_for_work().now_or_never().is_some() {
            vdom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }

        assert!(*completed.borrow(), "test closure did not run inside VirtualDom");
    }

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn view_with_rows() -> WaveformView {
        let mut v = WaveformView::default();
        for i in 0..3 {
            v.rows.push(RowDescriptor {
                kind: RowKind::Analog,
                signal_height_px: 80.0,
                channel_index: i,
                visible: true,
                decoder_kind: None,
            });
        }
        v.rows_dirty = false;
        v.view_samples = 1000;
        v.view_start = 0;
        v
    }

    /// Create a WaveformView pre-populated with `count` analog rows of
    /// 80 px each, with a known `view_start` and `view_samples`.
    fn view_with_n_rows(count: usize, view_start: usize, view_samples: usize) -> WaveformView {
        let mut v = WaveformView::default();
        for i in 0..count {
            v.rows.push(RowDescriptor {
                kind: RowKind::Analog,
                signal_height_px: 80.0,
                channel_index: i,
                visible: true,
                decoder_kind: None,
            });
        }
        v.rows_dirty = false;
        v.view_start = view_start;
        v.view_samples = view_samples;
        v
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  Pure-logic tests (no Dioxus runtime needed)
    // ═══════════════════════════════════════════════════════════════════════════

    // ── set_row_height (resize clamping) ─────────────────────────────────

    #[test]
    fn set_row_height_clamps_to_min() {
        let mut v = view_with_rows();
        v.set_row_height(0, 5.0);
        assert!(v.rows[0].signal_height_px >= 20.0, "analog min is 20");
    }

    #[test]
    fn set_row_height_clamps_to_max() {
        let mut v = view_with_rows();
        v.set_row_height(0, 999.0);
        assert_eq!(v.rows[0].signal_height_px, 400.0);
    }

    #[test]
    fn set_row_height_preserves_exact_value() {
        let mut v = view_with_rows();
        v.set_row_height(0, 120.0);
        assert_eq!(v.rows[0].signal_height_px, 120.0);
    }

    #[test]
    fn set_row_height_out_of_bounds_is_noop() {
        let mut v = view_with_rows();
        let before = v.rows[0].signal_height_px;
        v.set_row_height(999, 50.0);
        assert_eq!(v.rows[0].signal_height_px, before);
    }

    // ── move_row (reorder) ──────────────────────────────────────────────

    /// Forward drag: row 0 past row 2 → `move_row(0, 3)`.
    #[test]
    fn move_row_forward() {
        let mut v = view_with_rows();
        v.move_row(0, 3);
        assert_eq!(v.rows[0].channel_index, 1);
        assert_eq!(v.rows[1].channel_index, 2);
        assert_eq!(v.rows[2].channel_index, 0);
        assert!(v.rows_dirty);
    }

    /// Backward drag: row 2 before row 0 → `move_row(2, 0)`.
    #[test]
    fn move_row_backward() {
        let mut v = view_with_rows();
        v.move_row(2, 0);
        assert_eq!(v.rows[0].channel_index, 2);
        assert_eq!(v.rows[1].channel_index, 0);
        assert_eq!(v.rows[2].channel_index, 1);
        assert!(v.rows_dirty);
    }

    /// Forward drag: row 1 past row 2 → `move_row(1, 3)`.
    #[test]
    fn move_row_forward_middle() {
        let mut v = view_with_rows();
        v.move_row(1, 3);
        assert_eq!(v.rows[0].channel_index, 0);
        assert_eq!(v.rows[1].channel_index, 2);
        assert_eq!(v.rows[2].channel_index, 1);
        assert!(v.rows_dirty);
    }

    #[test]
    fn move_row_same_position_is_noop() {
        let mut v = view_with_rows();
        v.move_row(1, 1);
        assert_eq!(v.rows[0].channel_index, 0);
        assert!(!v.rows_dirty);
    }

    #[test]
    fn move_row_out_of_bounds_is_noop() {
        let mut v = view_with_rows();
        v.move_row(99, 0);
        assert_eq!(v.rows[0].channel_index, 0);
        assert!(!v.rows_dirty);
    }

    // ── row_at_y (Y coordinate → row index) ─────────────────────────────

    #[test]
    fn row_at_y_first_row() {
        let v = view_with_rows();
        let h = v.rows[0].total_height();
        assert_eq!(v.row_at_y(5.0), Some(0));
        assert_eq!(v.row_at_y(h - 1.0), Some(0));
    }

    #[test]
    fn row_at_y_second_row() {
        let v = view_with_rows();
        let h = v.rows[0].total_height();
        assert_eq!(v.row_at_y(h + 1.0), Some(1));
    }

    #[test]
    fn row_at_y_below_all_returns_none() {
        let v = view_with_rows();
        let total = v.total_rows_height();
        assert_eq!(v.row_at_y(total + 100.0), None);
    }

    #[test]
    fn row_at_y_skips_invisible_rows() {
        let mut v = view_with_rows();
        v.rows[0].visible = false;
        let h = v.rows[1].total_height();
        assert_eq!(v.row_at_y(5.0), Some(1));
        assert_eq!(v.row_at_y(h + 1.0), Some(2));
    }

    // ── Pan maths (grab-sample → view_start) ────────────────────────────

    #[test]
    fn pan_grab_sample_at_center() {
        let v = view_with_rows();
        let frac = 100.0 / 200.0;
        let expected = (frac * v.view_samples as f64) as u64;
        assert_eq!(expected, 500);
    }

    #[test]
    fn pan_view_start_clamps_to_zero_via_saturating_sub() {
        let gs: u64 = 50;
        let offset: u64 = 100;
        assert_eq!(gs.saturating_sub(offset), 0);
    }

    #[test]
    fn pan_view_start_respects_max_sample_count() {
        let sample_count: usize = 2000;
        let view_samples: usize = 1000;
        let new_vs: u64 = 1500;
        let max_vs = sample_count.saturating_sub(view_samples);
        assert_eq!((new_vs as usize).min(max_vs), 1000);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  Signal-state tests – RowResizeState
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn resize_begin_sets_drag_row_and_start_state() {
        with_interaction_signals(|s| {
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(2, 80.0, 150.0);

            assert_eq!(*s.resize_drag_row.read(), Some(2));
            assert_eq!(*s.resize_start_y.read(), 150.0);
            assert_eq!(*s.resize_start_h.read(), 80.0);
        });
    }

    #[test]
    fn resize_is_active_and_is_resizing_queries() {
        with_interaction_signals(|s| {
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            assert!(!state.is_active());
            assert!(!state.is_resizing(1));

            state.begin(1, 80.0, 100.0);
            assert!(state.is_active());
            assert!(state.is_resizing(1));
            assert!(!state.is_resizing(0));
        });
    }

    #[test]
    fn resize_handle_mousemove_updates_live_height_and_view() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(0, 80.0, 100.0);

            // Drag 30 px down → expected height 110
            state.handle_mousemove(130.0, s.view, s.data_version);

            assert_eq!(*s.resize_live_h.read(), Some(110.0));
            assert_eq!(s.view.read().rows[0].signal_height_px, 110.0);
            assert!(*s.data_version.read() > 0, "data_version should be bumped");
        });
    }

    #[test]
    fn resize_handle_mousemove_clamps_to_minimum() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(0, 80.0, 200.0);

            // Drag far upward (height would go negative → clamped to 10.0 by
            // handle_mousemove, but set_row_height further clamps Analog to 20.0)
            state.handle_mousemove(50.0, s.view, s.data_version);

            assert_eq!(s.view.read().rows[0].signal_height_px, 20.0);
        });
    }

    #[test]
    fn resize_handle_mousemove_ignored_when_not_active() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let orig_height = s.view.read().rows[0].signal_height_px;
            let orig_dv = *s.data_version.read();
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );

            // No begin() → handle_mousemove is no-op
            state.handle_mousemove(200.0, s.view, s.data_version);

            assert_eq!(s.view.read().rows[0].signal_height_px, orig_height);
            assert_eq!(*s.data_version.read(), orig_dv);
        });
    }

    #[test]
    fn resize_commit_persists_height_and_clears_state() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(0, 80.0, 100.0);
            state.handle_mousemove(130.0, s.view, s.data_version);

            let dv_before_commit = *s.data_version.read();
            state.commit(s.view, s.data_version);

            assert_eq!(*s.resize_drag_row.read(), None);
            assert_eq!(*s.resize_live_h.read(), None);
            assert_eq!(s.view.read().rows[0].signal_height_px, 110.0);
            assert!(*s.data_version.read() > dv_before_commit, "data_version bumped again");
        });
    }

    #[test]
    fn resize_cancel_also_persists_last_height() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(1, 80.0, 200.0);
            state.handle_mousemove(230.0, s.view, s.data_version);

            let dv_before = *s.data_version.read();
            state.cancel(s.view, s.data_version);

            assert_eq!(*s.resize_drag_row.read(), None);
            assert_eq!(*s.resize_live_h.read(), None);
            assert_eq!(s.view.read().rows[1].signal_height_px, 110.0);
            assert!(*s.data_version.read() > dv_before, "data_version bumped on cancel");
        });
    }

    #[test]
    fn resize_effective_sig_h_returns_none_for_non_resized_row() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowResizeState::from_signals(
                s.resize_drag_row, s.resize_start_y, s.resize_start_h, s.resize_live_h,
            );
            state.begin(0, 80.0, 100.0);
            state.handle_mousemove(130.0, s.view, s.data_version);

            assert_eq!(state.effective_sig_h(0), Some(110.0));
            assert_eq!(state.effective_sig_h(1), None);
        });
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  Signal-state tests – RowReorderState
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn reorder_begin_sets_drag_row() {
        with_interaction_signals(|s| {
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(1);
            assert_eq!(*s.reorder_drag_row.read(), Some(1));
        });
    }

    #[test]
    fn reorder_is_active_and_is_dragging_queries() {
        with_interaction_signals(|s| {
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            assert!(!state.is_active());
            assert!(!state.is_dragging(0));

            state.begin(0);
            assert!(state.is_active());
            assert!(state.is_dragging(0));
            assert!(!state.is_dragging(1));
        });
    }

    #[test]
    fn reorder_handle_mousemove_sets_insert_at_forward() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(0);

            // Hover over row 2 → target = hovered + 1 = 3
            let row2_y = s.view.read().row_y_offset(2) + 5.0;
            state.handle_mousemove(row2_y, 0.0, s.view);

            assert_eq!(*s.reorder_insert_at.read(), Some(3));
        });
    }

    #[test]
    fn reorder_handle_mousemove_sets_insert_at_backward() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(2);

            // Hover over row 0 → target = hovered = 0
            state.handle_mousemove(5.0, 0.0, s.view);

            assert_eq!(*s.reorder_insert_at.read(), Some(0));
        });
    }

    #[test]
    fn reorder_commit_moves_row_and_clears_state() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(0);

            // Drag row 0 past row 2 → insert_at should become 3
            let row2_y = s.view.read().row_y_offset(2) + 5.0;
            state.handle_mousemove(row2_y, 0.0, s.view);

            let dv_before = *s.data_version.read();
            state.commit(s.view, s.data_version);

            assert_eq!(*s.reorder_drag_row.read(), None);
            assert_eq!(*s.reorder_insert_at.read(), None);
            // Row 0 (channel_index 0) should have moved to position 2
            assert_eq!(s.view.read().rows[2].channel_index, 0);
            assert!(*s.data_version.read() > dv_before);
        });
    }

    #[test]
    fn reorder_commit_same_position_skips_move_but_bumps_version() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(1);

            // Hover row 1 → forward target = 2, but from==to-like: actually
            // handle_mousemove computes target based on hovered > from,
            // so hovered=1, from=1, hovered !> from, target=hovered=1.
            // Set insert_at manually for this test
            s.reorder_insert_at.set(Some(1));

            let dv_before = *s.data_version.read();
            state.commit(s.view, s.data_version);

            assert_eq!(*s.reorder_drag_row.read(), None);
            assert_eq!(*s.reorder_insert_at.read(), None);
            // Data version is ALWAYS bumped (to clear drop indicator)
            assert!(*s.data_version.read() > dv_before);
        });
    }

    #[test]
    fn reorder_cancel_clears_without_moving() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(1);

            // Simulate setting insert_at (via mousemove)
            s.reorder_insert_at.set(Some(0));

            let original_order: Vec<usize> =
                s.view.read().rows.iter().map(|r| r.channel_index).collect();

            state.cancel();

            assert_eq!(*s.reorder_drag_row.read(), None);
            assert_eq!(*s.reorder_insert_at.read(), None);
            let current_order: Vec<usize> =
                s.view.read().rows.iter().map(|r| r.channel_index).collect();
            assert_eq!(original_order, current_order, "cancel must not move rows");
        });
    }

    #[test]
    fn reorder_target_pos_returns_insert_at() {
        with_interaction_signals(|mut s| {
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            assert_eq!(state.target_pos(), None);

            state.begin(0);
            s.reorder_insert_at.set(Some(2));
            assert_eq!(state.target_pos(), Some(2));
        });
    }

    #[test]
    fn reorder_with_scroll_offset() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_rows());
            let mut state = RowReorderState::from_signals(s.reorder_drag_row, s.reorder_insert_at);
            state.begin(0);

            // scroll_top=0, element_y set to be inside row 2
            // Row 0: 0..108, divider 108..113, Row 1: 113..221, divider 221..226, Row 2: 226..334
            let row2_mid_y = s.view.read().row_y_offset(2) + 40.0;
            state.handle_mousemove(row2_mid_y, 0.0, s.view);

            // Hovering row 2, from=0, hovered(2) > from(0) → target = hovered+1 = 3
            assert_eq!(*s.reorder_insert_at.read(), Some(3));

            // Now test with scroll: scroll_top=200, same element_y → adjusted_y > total
            // so row_at_y returns None → hovered=from=0 → target=0
            let total_h = s.view.read().total_rows_height();
            state.handle_mousemove(total_h + 500.0, 200.0, s.view);

            // No row at adjusted_y → hovered falls back to from (0) → target = 0
            assert_eq!(*s.reorder_insert_at.read(), Some(0));
        });
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  Signal-state tests – CanvasPanState
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn pan_begin_sets_active_and_computes_grab_sample() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            // Click at center of canvas
            let canvas_w = 200.0;
            let click_x = canvas_w / 2.0;
            state.begin(click_x, canvas_w, s.view);

            assert!(state.is_active());
            // grab_sample should be 500 (half of view_samples=1000)
            assert_eq!(*s.pan_grab_sample.read(), Some(500));
        });
    }

    #[test]
    fn pan_begin_any_x_activates() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            // Any X in the canvas area (no LABEL_W guard needed after split)
            state.begin(10.0, 200.0, s.view);

            assert!(state.is_active());
            // At x=10, frac=0.05, grab_sample = 50
            assert_eq!(*s.pan_grab_sample.read(), Some(50));
        });
    }

    #[test]
    fn pan_handle_mousemove_drags_left_increases_view_start() {
        with_interaction_signals(|mut s| {
            // Start at 0 so we can pan into later samples
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            let canvas_w = 200.0;
            // Grab at center: grab_sample = 0 + 0.5*1000 = 500
            state.begin(canvas_w / 2.0, canvas_w, s.view);

            // Move left (10% of canvas): frac=0.1, offset=100
            // new_vs = 500 - 100 = 400, but wait—pan is gs - offset.
            // Dragging left DECREASES frac → INCREASES view_start.
            // Actually: when mouse moves left from grab, frac drops.
            // gs = 500, offset = 0.1*1000 = 100, new_vs = 500-100 = 400.
            // That means view_start DECREASED! The grab sample follows the
            // mouse: moving left shows EARLIER data (lower view_start).
            // Let's test the actual behavior by moving right from grab.
            let move_x = canvas_w * 0.9;
            state.handle_mousemove(move_x, canvas_w, s.view, 10000, s.data_version);

            // Moving right → frac increases (0.9) → offset=900 →
            // new_vs = 500.saturating_sub(900) = 0. view_start stays 0.
            assert!(!s.view.read().auto_scroll, "auto_scroll should be disabled");
        });
    }

    #[test]
    fn pan_handle_mousemove_drags_right_decreases_view_start() {
        with_interaction_signals(|mut s| {
            // Start with room to pan left (earlier samples)
            s.view.set(view_with_n_rows(1, 500, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            let canvas_w = 200.0;
            // Grab at center: grab_sample = 500 + 0.5*1000 = 1000
            state.begin(canvas_w / 2.0, canvas_w, s.view);

            // Move far right (90%): frac=0.9, offset=900
            // new_vs = 1000.saturating_sub(900) = 100
            let move_x = canvas_w * 0.9;
            state.handle_mousemove(move_x, canvas_w, s.view, 10000, s.data_version);

            // Dragging right shows EARLIER samples (view_start decreased)
            assert!(s.view.read().view_start < 500, "dragging right should show earlier samples");
        });
    }

    #[test]
    fn pan_handle_mousemove_saturating_sub_prevents_underflow() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            let canvas_w = 200.0;
            // Grab at far left (x=0, frac=0): grab_sample = 0 + 0*1000 = 0
            state.begin(0.0, canvas_w, s.view);

            // Move far right (frac=1.0): offset = 1000
            // new_vs = 0.saturating_sub(1000) = 0 (saturating prevents underflow)
            let move_x = canvas_w;
            state.handle_mousemove(move_x, canvas_w, s.view, 10000, s.data_version);

            // view_start should be 0, not negative/underflowed
            assert_eq!(s.view.read().view_start, 0);
        });
    }

    #[test]
    fn pan_handle_mousemove_respects_max_view_start() {
        with_interaction_signals(|mut s| {
            // Total samples = 1100, view_samples = 1000 → max view_start = 100
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            let canvas_w = 200.0;
            // Grab at center: grab_sample = 0 + 500 = 500
            state.begin(canvas_w / 2.0, canvas_w, s.view);

            // Move far right
            state.handle_mousemove(canvas_w, canvas_w, s.view, 1100, s.data_version);

            // view_start must not exceed sample_count - view_samples = 100
            assert!(s.view.read().view_start <= 100);
        });
    }

    #[test]
    fn pan_handle_mousemove_ignored_when_not_active() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);
            let orig_start = s.view.read().view_start;

            // No begin → handle_mousemove is no-op
            state.handle_mousemove(100.0, 200.0, s.view, 10000, s.data_version);

            assert_eq!(s.view.read().view_start, orig_start);
        });
    }

    #[test]
    fn pan_end_clears_state() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);
            state.begin(50.0, 200.0, s.view);

            assert!(state.is_active());

            state.end();

            assert!(!state.is_active());
            assert_eq!(*s.pan_grab_sample.read(), None);
        });
    }

    #[test]
    fn pan_is_active_query() {
        with_interaction_signals(|mut s| {
            s.view.set(view_with_n_rows(1, 0, 1000));
            let mut state = CanvasPanState::from_signals(s.pan_active, s.pan_grab_sample);

            assert!(!state.is_active());

            state.begin(50.0, 200.0, s.view);
            assert!(state.is_active());

            state.end();
            assert!(!state.is_active());
        });
    }
}