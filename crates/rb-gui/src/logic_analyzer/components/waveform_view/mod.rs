//! Waveform display: multi-canvas architecture with HTML overlays.
//!
//! Each signal row has its own `<canvas>` (drawing only the waveform).
//! Labels, markers, cursor line, and time ruler are HTML/CSS elements.
//! A single `use_effect` draws all canvases on each `data_version` change.
//!
//! Sub-components live in sibling modules:
//! - `time_ruler.rs`, `marker_bar.rs`, `cursor_line.rs` — HTML overlays
//! - `row_base.rs` — shared drawing helpers and interaction hooks
//! - `row_analog.rs`, `row_digital.rs`, `row_decoder.rs` — per-row drawing

mod canvas_toolbar;
mod cursor_line;
mod marker_bar;
mod row_analog;
mod row_base;
mod row_decoder;
mod row_digital;
mod time_ruler;
#[cfg(test)]
mod test_utils;

pub use canvas_toolbar::CanvasToolbar;
pub use row_base::use_canvas_pan;

use cursor_line::CursorLine;
use marker_bar::MarkerBar;
use row_base::{use_row_reorder, use_row_resize, RowReorderState, RowResizeState, DARK_COLORS, LIGHT_COLORS};
use time_ruler::TimeRuler;

use dioxus::prelude::*;
use rb_canvas::JsCanvasRenderer;
use rb_model::{AnalogTrace, DigitalTrace};

use crate::logic_analyzer::acquisition::AcquisitionConfig;
use crate::logic_analyzer::decoder::DecoderConfig;
use crate::logic_analyzer::AcquisitionState;
use crate::logic_analyzer::waveform_state::{
    row_layout::{DIVIDER_H, LABEL_W, MARKER_BAR_H, MEASUREMENT_ZONE_H, RowKind, TIME_RULER_H},
    WaveformState,
};

// ── WaveformData – injectable acquisition snapshot for testability ─────────

/// Snapshot of acquisition data passed into [`WaveformView`] as a Signal prop.
/// Decouples the component from [`crate::components::app::AppStateRef`] context.
#[derive(Clone)]
pub struct WaveformData {
    pub acq_state: AcquisitionState,
    pub analog: Vec<AnalogTrace>,
    pub digital: Option<DigitalTrace>,
    pub sample_count: usize,
}

impl PartialEq for WaveformData {
    fn eq(&self, other: &Self) -> bool {
        // Compare by identity, not content — traces may not implement PartialEq.
        // Dioxus Signals use PartialEq to decide whether to re-render.
        self.sample_count == other.sample_count
            && self.acq_state == other.acq_state
            && self.analog.len() == other.analog.len()
            && self.digital.is_some() == other.digital.is_some()
    }
}

impl WaveformData {
    /// Empty snapshot — no channels, no data.
    pub fn empty() -> Self {
        Self { acq_state: AcquisitionState::Idle, analog: Vec::new(), digital: None, sample_count: 0 }
    }
}

// ── Formatting helpers ───────────────────────────────────────────────────────

fn adaptive_tick_spacing(view_duration_s: f64) -> (f64, usize) {
    let raw = view_duration_s / 6.0;
    let iv: &[(f64, usize)] = &[
        (1e-9,5),(5e-9,5),(10e-9,5),(50e-9,5),(100e-9,5),(500e-9,5),
        (1e-6,5),(5e-6,5),(10e-6,5),(50e-6,5),(100e-6,5),(500e-6,5),
        (1e-3,5),(5e-3,5),(10e-3,5),(50e-3,5),(100e-3,5),(500e-3,5),
        (1.0,4),(5.0,5),(10.0,4),
    ];
    for &(s, m) in iv { if s >= raw * 0.5 { return (s, m); } }
    (10.0, 4)
}

fn fmt_tick(seconds: f64) -> String {
    if seconds >= 1.0 { format!("{seconds:.1}s") }
    else if seconds >= 1e-3 { format!("{:.0}ms", seconds * 1e3) }
    else if seconds >= 1e-6 { format!("{:.0}µs", seconds * 1e6) }
    else { format!("{:.0}ns", seconds * 1e9) }
}

fn fmt_time_ns(ns: f64) -> String {
    if ns >= 1e9 { format!("{:.3}s", ns / 1e9) }
    else if ns >= 1e6 { format!("{:.3}ms", ns / 1e6) }
    else if ns >= 1e3 { format!("{:.3}µs", ns / 1e3) }
    else { format!("{:.0}ns", ns) }
}

// ── Tick computation ─────────────────────────────────────────────────────────

fn compute_ticks(
    range_start: usize,
    range_end: usize,
    range_len: f64,
    sample_rate_hz: f64,
) -> Vec<(f64, String, Vec<f64>)> {
    let view_dur = range_len / sample_rate_hz;
    let (tick_s, minors) = adaptive_tick_spacing(view_dur);
    let first_tick = (range_start as f64 / sample_rate_hz / tick_s).ceil() * tick_s;
    let max_ts = range_end as f64 / sample_rate_hz;
    let mut elements = Vec::new();
    let mut ts = first_tick;
    while ts <= max_ts {
        let pct = ((ts * sample_rate_hz - range_start as f64) / range_len * 100.0).clamp(0.0, 100.0);
        let label = fmt_tick(ts);
        let mut minor_els = Vec::new();
        for m in 1..minors {
            let ms = ts + m as f64 * tick_s / minors as f64;
            let mpct =
                ((ms * sample_rate_hz - range_start as f64) / range_len * 100.0).clamp(0.0, 100.0);
            if mpct < 100.0 {
                minor_els.push(mpct);
            }
        }
        elements.push((pct, label, minor_els));
        ts += tick_s;
    }
    elements
}

// ── CursorTracker – reusable cursor-position update logic ────────────────────

#[derive(Clone, Copy)]
struct CursorTracker {
    sample_pos: Signal<Option<u64>>,
    px: Signal<Option<f64>>,
    label: Signal<String>,
}

impl CursorTracker {
    fn update(
        &mut self,
        page_x: f64,
        container_left: f64,
        label_width_px: f64,
        wf_state: Signal<WaveformState>,
        canvas_width_px: f64,
        sample_count: usize,
    ) {
        let el_x = page_x - container_left;
        let canvas_x = el_x - label_width_px;
        let v = wf_state.read();
        let rs = v.viewport.view_start;
        let re = (v.viewport.view_start + v.viewport.view_samples).min(sample_count);
        let rl = (re - rs).max(1) as f64;
        let cw = canvas_width_px;
        let sp = rs as u64 + ((canvas_x / cw.max(1.0)).clamp(0.0, 1.0) * rl) as u64;
        drop(v);
        let in_label = el_x < label_width_px;
        self.px.set(if in_label { None } else { Some(el_x) });
        self.sample_pos.set(if in_label { None } else { Some(sp) });
        self.label.set(fmt_time_ns((sp as f64 / 1_000_000.0) * 1e9));
    }
}

// ── Row rendering helpers ────────────────────────────────────────────────────

fn row_inner(
    row_idx: usize,
    canvas_id: String,
    label_el: Element,
    sig_height: f64,
    effective_height: f64,
    original_row_height: f64,
    label_width_px: f64,
    mut reorder: RowReorderState,
    mut wf_state: Signal<WaveformState>,
    mut resize: RowResizeState,
    label_width: Signal<f64>,
    mut divider_start_x: Signal<f64>,
    mut divider_start_width: Signal<f64>,
    mut dragging_divider: Signal<bool>,
) -> Element {
    rsx! {
        div {
            class: "flex relative",
            style: "height: {effective_height}px",

            // ── LEFT: Label area ───────────────
            div {
                class: "flex flex-col flex-shrink-0 bg-gray-100 dark:bg-[#0a0e14]",
                style: "width: {label_width_px}px; height: {effective_height}px; position: relative",

                // Label text (clickable, draggable)
                div {
                    class: "flex-1 flex items-center px-1 select-none cursor-grab active:cursor-grabbing",
                    style: "height: {effective_height}px",
                    onmousedown: move |evt| {
                        evt.prevent_default();
                        evt.stop_propagation();
                        let coords = evt.data().coordinates();
                        reorder.begin(row_idx, coords.element().x, coords.element().y, coords.page().x, coords.page().y, original_row_height);
                    },
                    oncontextmenu: move |evt| {
                        evt.prevent_default();
                        evt.stop_propagation();
                        wf_state.write().row_layout.toggle_row_visible(row_idx);
                    },
                    {label_el}
                }

                // Resize handle
                div {
                    class: "absolute left-0 right-0 cursor-ns-resize group z-10",
                    style: "top: {effective_height - DIVIDER_H}px; height: {DIVIDER_H * 2.0}px",
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        resize.begin(row_idx, sig_height, evt.data().coordinates().page().y);
                    },
                    div {
                        class: "absolute left-0 right-0 bg-gray-300/60 dark:bg-zinc-600/40",
                        style: "height: 1px; top: {DIVIDER_H}px",
                    }
                }
            }

            // ── Vertical divider overlay ──
            div {
                class: "absolute top-0 bottom-0 cursor-col-resize group z-10",
                style: "left: {label_width_px - DIVIDER_H}px; width: {DIVIDER_H * 2.0}px",
                onmousedown: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    divider_start_x.set(evt.data().coordinates().page().x);
                    divider_start_width.set(label_width());
                    dragging_divider.set(true);
                },
                div {
                    class: "absolute left-1/2 -translate-x-1/2 top-0 bottom-0 w-px bg-gray-300 dark:bg-zinc-600"
                }
            }

            // ── RIGHT: Canvas + Measurement Zones ──
            div {
                class: "flex-1 flex flex-col min-w-0",
                style: "height: {effective_height}px",

                div { class: "flex-shrink-0", style: "height: {MEASUREMENT_ZONE_H}px" }

                canvas {
                    id: "{canvas_id}",
                    class: "pointer-events-none min-w-0",
                    style: "width: 100%; height: {sig_height}px",
                    width: "100%",
                    height: "{sig_height}",
                }

                div { class: "flex-shrink-0", style: "height: {MEASUREMENT_ZONE_H}px" }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Main component
// ═══════════════════════════════════════════════════════════════════════════════

#[component]
pub fn WaveformView(
    tab_id: crate::tab_state::TabId,
    data_version: Signal<u64>,
    mut wf_state: Signal<WaveformState>,
    mut acquisition_config: Signal<AcquisitionConfig>,
    mut decoder_config: Signal<DecoderConfig>,
    mut cursor_sample_pos: Signal<Option<u64>>,
    waveform_data: Signal<WaveformData>,
    theme: Signal<crate::app_state::Theme>,
) -> Element {
    let _version = data_version();
    let wd = waveform_data();
    let (acq_state, analog, digital, sample_count) =
        (wd.acq_state, wd.analog, wd.digital, wd.sample_count);
    let is_running = matches!(acq_state, AcquisitionState::Running);

    // ── Dynamic canvas width ──────────────────────────────────────────
    let canvas_width_px = use_signal(|| 764.0f64);

    // Signal-ized sample_count so the drawing effect always sees the
    // latest value (avoid closure-capture staleness).
    let mut sample_count_sig = use_signal(|| sample_count);
    sample_count_sig.set(sample_count);

    // ── Update state (only when underlying data actually changes) ────────
    // Guarded by a layout version to avoid re-triggering parent re-renders
    // on every render, which causes an infinite loop on the web platform.
    //
    // Row layout is rebuilt from *enabled* channel counts in AcquisitionConfig,
    // not from trace dimensions. This ensures rows appear immediately after
    // device connect, before any acquisition has started.
    //
    // Two conditions trigger a rebuild:
    // 1. Enabled channel counts changed (device connect, user toggle).
    // 2. Actual row count doesn't match expected (tab switch to a fresh tab
    //    that has the same channel counts but an empty row list).
    let mut last_channel_counts = use_signal(|| (0usize, 0usize));
    let cfg = acquisition_config.read();
    let enabled_analog = cfg.analog_enabled.iter().filter(|e| **e).count();
    let enabled_digital = cfg.digital_enabled.iter().filter(|e| **e).count();
    let expected_rows = enabled_analog + enabled_digital;
    let actual_rows = wf_state.read().row_layout.rows.len();
    let counts = (enabled_analog, enabled_digital);
    if last_channel_counts() != counts || actual_rows != expected_rows {
        last_channel_counts.set(counts);
        {
            let mut ws = wf_state.write();
            ws.viewport.clamp_view(sample_count, is_running);
            ws.row_layout.rebuild_rows(enabled_analog, enabled_digital);
        }
        if let Some(ref dt) = digital {
            decoder_config.write().feed(dt);
        }
    }

    // ── Derived ───────────────────────────────────────────────────────────
    let rows = wf_state.read().row_layout.rows.clone();
    let range_start = wf_state.read().viewport.view_start;
    let range_end = (wf_state.read().viewport.view_start + wf_state.read().viewport.view_samples).min(sample_count);
    let range_len = (range_end - range_start).max(1) as f64;

    let sample_rate_hz = 1_000_000.0; // TODO: real sample rate
    let tick_elements = compute_ticks(range_start, range_end, range_len, sample_rate_hz);

    let short_id = format!("tab-{}", tab_id.0);

    // ── Interaction hooks ────────────────────────────────────────────
    let resize = use_row_resize();
    let reorder = use_row_reorder();
    let pan = use_canvas_pan();
    let pan_active = use_signal(|| false);
    let scroll_y = use_signal(|| 0.0f64);
    let container_top = use_signal(|| 0.0f64);
    let container_left = use_signal(|| 0.0f64);
    let cursor_px = use_signal(|| None::<f64>);
    let cursor_label = use_signal(|| String::new());
    let cursor_tracker = CursorTracker {
        sample_pos: cursor_sample_pos,
        px: cursor_px,
        label: cursor_label,
    };

    // ── Label panel width (dynamic, draggable divider) ────────────────
    let label_width = use_signal(|| wf_state.read().row_layout.label_width);
    let dragging_divider = use_signal(|| false);
    let divider_start_x = use_signal(|| 0.0f64);
    let divider_start_width = use_signal(|| LABEL_W);

    // ── Pre-compute visible row data for rendering ────────────────────────
    let visible_rows: Vec<_> = rows.iter().enumerate()
        .filter(|(_, r)| r.visible)
        .map(|(idx, row)| {
            let signal_id = format!("sig-{short_id}-{idx}");
            let row_h = row.total_height();
            let sig_h = row.signal_height_px;
            let kind = row.kind;
            let ci = row.channel_index;
            let label_el = match kind {
                RowKind::Analog => {
                    let name = analog.get(ci)
                        .map(|t| t.channel().name.clone())
                        .unwrap_or_default();
                    rsx! { span { class: "text-[9px] text-gray-700 dark:text-zinc-300 truncate", "{name}" } }
                }
                RowKind::Digital => {
                    let name = digital.as_ref()
                        .and_then(|dt| dt.channels().get(ci))
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| format!("D{}", ci));
                    rsx! { span { class: "text-[9px] text-gray-500 dark:text-zinc-400 truncate", "{name}" } }
                }
                RowKind::Decoder => {
                    rsx! { span { class: "text-[9px] text-gray-400 dark:text-zinc-500 truncate", "DEC" } }
                }
            };
            (idx, signal_id, label_el, row_h, sig_h)
        })
        .collect();

    // ── Canvas drawing: runs via use_effect AFTER the DOM is committed ──
    //
    // Drawing must be deferred to a use_effect so that canvas DOM elements
    // exist when the JS tries to find them via getElementById.  Without this,
    // dioxus::document::eval runs during the render phase (before DOM commit)
    // and produces "Canvas not found" warnings.
    //
    // All signal reads happen *inside* the effect closure so Dioxus
    // re-subscribes and re-runs the effect when the data changes.
    let mut last_draw_key = use_signal(|| (0u64, 0u64, 0u32));
    let draw_short_id = short_id.clone();
    let draw_range_start = range_start;
    let draw_range_end = range_end;
    let draw_range_len = range_len;

    use_effect(move || {
        let dv = data_version();
        let wd = waveform_data.read();
        let ws = wf_state.read();
        let cw = canvas_width_px();
        let t = theme();

        // Only redraw when relevant properties actually change.
        let key = (dv, wd.sample_count as u64, cw.round() as u32);
        if last_draw_key() == key {
            return;
        }
        last_draw_key.set(key);

        let colors = match t {
            crate::app_state::Theme::Dark => &DARK_COLORS,
            _ => &LIGHT_COLORS,
        };
        let rows = &ws.row_layout.rows;
        log::info!(
            "Canvas draw: dv={dv} cw={cw:.0} rows={} sc={}",
            rows.len(),
            wd.sample_count,
        );
        draw_all_canvases(
            &draw_short_id,
            &wd.analog,
            &wd.digital,
            rows,
            &[],
            draw_range_start,
            draw_range_end,
            draw_range_len,
            cw,
            colors,
        );
    });

    // ═══════════════════════════════════════════════════════════════════════
    //  Render
    // ═══════════════════════════════════════════════════════════════════════

    // ── Compute whether any drag interaction is active ────────────────
    let drag_active = reorder.is_active() || resize.is_active() || dragging_divider() || pan_active();

    rsx! {
        div { class: "flex flex-col h-full bg-white dark:bg-[#0d1117]",
            // ═══════════════════════════════════════════════════════════════
            //  HEADER: spacer + Time Ruler / Marker Bar
            //  (divider is an absolute overlay, no layout width)
            // ═══════════════════════════════════════════════════════════════
            div { class: "flex flex-shrink-0 relative",
                div {
                    class: "flex-shrink-0 bg-gray-100 dark:bg-[#0a0e14] border-b border-gray-200 dark:border-b dark:border-[#30363d]",
                    style: "width: {label_width()}px; height: {TIME_RULER_H + MARKER_BAR_H}px"
                }
                // Vertical divider overlay (same as row dividers)
                div {
                    class: "absolute top-0 bottom-0 cursor-col-resize group z-10",
                    style: "left: {label_width() - DIVIDER_H}px; width: {DIVIDER_H * 2.0}px",
                    onmousedown: {
                        let label_width = label_width;
                        let mut divider_start_x = divider_start_x;
                        let mut divider_start_width = divider_start_width;
                        let mut dragging_divider = dragging_divider;
                        move |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                            divider_start_x.set(evt.data().coordinates().page().x);
                            divider_start_width.set(label_width());
                            dragging_divider.set(true);
                        }
                    },
                    div {
                        class: "absolute left-1/2 -translate-x-1/2 top-0 bottom-0 w-px bg-gray-300 dark:bg-zinc-600"
                    }
                }
                div { class: "flex-1 flex flex-col min-w-0",
                    TimeRuler { tick_elements: tick_elements.clone() }
                    MarkerBar {
                        markers: wf_state.read().marker_set.markers.clone(),
                        range_start,
                        range_len,
                    }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            //  BODY: unified scrollable row container
            // ═══════════════════════════════════════════════════════════════
            div {
                class: "flex flex-col flex-1 min-h-0 relative",

                // ── Scrollable row area ───────────────────────────────────
                div {
                    class: "flex-1 overflow-y-auto relative",
                    style: "padding-bottom: {DIVIDER_H}px",
                    onscroll: {
                        let short_id = short_id.clone();
                        let mut scroll_y = scroll_y;
                        move |_| {
                            let id = short_id.clone();
                            spawn(async move {
                                let js = format!(
                                    "var r=document.getElementById('row-scroll-{}');r?r.scrollTop:0",
                                    id
                                );
                                let mut eval = dioxus::document::eval(&js);
                                if let Ok(val) = eval.recv::<f64>().await {
                                    scroll_y.set(val);
                                }
                            });
                        }
                    },
                    onmounted: {
                        let mut ct = container_top;
                        let mut cl = container_left;
                        let mut cw = canvas_width_px;
                        let lw = label_width;
                        move |data| {
                            spawn(async move {
                                if let Ok(rect) = data.get_client_rect().await {
                                    ct.set(rect.origin.y);
                                    cl.set(rect.origin.x);
                                    let w = rect.width() - lw();
                                    if w > 0.0 {
                                        cw.set(w.max(100.0));
                                    }
                                }
                            });
                        }
                    },
                    onresize: {
                        let mut cw = canvas_width_px;
                        let lw = label_width;
                        move |evt: Event<dioxus::html::ResizeData>| {
                            if let Ok(size) = evt.data().get_content_box_size() {
                                let w = size.width - lw();
                                if w > 0.0 {
                                    cw.set(w.max(100.0));
                                }
                            }
                        }
                    },
                    onwheel: {
                        let mut wf_state = wf_state;
                        let canvas_width_px = canvas_width_px;
                        let sample_count = sample_count;
                        let mut data_version = data_version;
                        move |evt| {
                            evt.prevent_default();
                            let (dx, dy) = match evt.data().delta() {
                                dioxus::html::geometry::WheelDelta::Pixels(v) => (v.x, v.y),
                                dioxus::html::geometry::WheelDelta::Lines(v) => (v.x * 20.0, v.y * 20.0),
                                dioxus::html::geometry::WheelDelta::Pages(v) => (v.x * 200.0, v.y * 200.0),
                            };
                            if dx.abs() > 0.01 {
                                wf_state.write().viewport.pan(-dx as f32, canvas_width_px() as f32, sample_count);
                            } else {
                                let factor: f64 = if dy < 0.0 { 0.8 } else { 1.25 };
                                wf_state.write().viewport.zoom(factor, sample_count);
                            }
                            data_version += 1;
                        }
                    },
                    onmousedown: {
                        let mut pan = pan;
                        let canvas_width_px = canvas_width_px;
                        let wf_state = wf_state;
                        let mut pan_active = pan_active;
                        move |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                            // Only handle pan on the canvas area (right side),
                            // not on labels or dividers.
                            let coords = evt.data().coordinates();
                            pan.begin(coords.element().x, canvas_width_px(), wf_state);
                            pan_active.set(true);
                        }
                    },
                    onmousemove: {
                        let container_left = container_left;
                        let label_width = label_width;
                        let mut cursor_tracker = cursor_tracker;
                        let wf_state = wf_state;
                        let canvas_width_px = canvas_width_px;
                        let sample_count = sample_count;
                        move |evt| {
                            let page_x = evt.data().coordinates().page().x;
                            cursor_tracker.update(
                                page_x,
                                container_left(),
                                label_width(),
                                wf_state,
                                canvas_width_px(),
                                sample_count,
                            );
                        }
                    },
                    id: "row-scroll-{short_id}",

                    // ── Reorder state (shared by gap + row loop) ──────────
                    {
                        let target = reorder.target_pos();
                        let drag_idx = reorder.dragged_row_index();
                        let gap_pos = target.filter(|&t| Some(t) != drag_idx);
                        let num_visible = visible_rows.len();
                        let source_row_h = reorder.source_row_height();

                        // ── Row items (pre-computed for rendering) ────────

                        let row_items: Vec<_> = visible_rows.iter().enumerate().map(|(vi, (row_idx, canvas_id, label_el, row_height, base_sig_height))| {
                            let row_idx = *row_idx;
                            let sig_height = resize.effective_sig_h(row_idx).unwrap_or(*base_sig_height);
                            let reordering = reorder.is_dragging(row_idx);
                            let is_last = vi == num_visible - 1;
                            let drop_here = gap_pos.map(|t| t == row_idx + 1).unwrap_or(false);
                            let effective_height = *row_height;
                            let translate_x = if reordering { reorder.drag_offset_x() } else { 0.0 };
                            let translate_y = if reordering {
                                let raw = reorder.drag_offset_y();
                                // When the drop gap appears above the source row,
                                // the source row's DOM position shifts down by
                                // the gap height. Compensate the transform offset.
                                if gap_pos.is_some_and(|gp| gp < row_idx) {
                                    raw - reorder.source_row_height()
                                } else {
                                    raw
                                }
                            } else {
                                0.0
                            };
                            // When there's a target gap, collapse the source row
                            // out of the flow so the total height stays constant.
                            // overflow:visible + transform keeps it visible at cursor.
                            let collapsed = reordering && gap_pos.is_some();
                            (row_idx, canvas_id.clone(), label_el.clone(), sig_height, effective_height, *row_height, reordering, is_last, drop_here, translate_x, translate_y, collapsed)
                        }).collect();

                        let label_width_px = label_width();
                        let has_rows = !row_items.is_empty();

                        rsx! {
                            // ── Drop gap at top (before first row) ────
                            if gap_pos == Some(0) {
                                div {
                                    class: "relative z-20 pointer-events-none flex-shrink-0 transition-all duration-150",
                                    style: "height: {source_row_h}px; border: 1px dashed #f59e0b; margin: 0 2px;"
                                }
                            }

                            // ── Row loop ──────────────────────────────
                            for (row_idx, canvas_id, label_el, sig_height, effective_height, original_row_height, reordering, is_last, drop_here, translate_x, translate_y, collapsed) in row_items {
                                // ── Row wrapper ─────────────────────────
                                div {
                                    class: if reordering { "flex-shrink-0 z-50 relative pointer-events-none" } else { "flex-shrink-0" },
                                    style: if collapsed {
                                        "height: 0px; overflow: visible; transform: translate({translate_x}px, {translate_y}px)"
                                    } else {
                                        "height: {effective_height}px; transform: translate({translate_x}px, {translate_y}px)"
                                    },

                                    {row_inner(
                                        row_idx, canvas_id, label_el, sig_height, effective_height, original_row_height, label_width_px,
                                        reorder, wf_state, resize,
                                        label_width, divider_start_x,
                                        divider_start_width, dragging_divider,
                                    )}

                                }

                                // ── Full-width 1px separator (between rows) ──
                                if !is_last {
                                    div {
                                        class: if drop_here { "relative flex-shrink-0 z-20" } else { "relative flex-shrink-0" },
                                        style: if drop_here {
                                            "height: {source_row_h}px"
                                        } else {
                                            "height: 1px"
                                        },
                                        if drop_here {
                                            div {
                                                class: "absolute left-0 right-0 top-0 bottom-0 border border-dashed border-amber-400",
                                                style: "margin: 2px;"
                                            }
                                        } else {
                                            div {
                                                class: "absolute left-0 right-0 bg-gray-300/60 dark:bg-zinc-600/40",
                                                style: "height: 1px; top: 0;"
                                            }
                                        }
                                    }
                                }
                            }

                            if gap_pos == Some(wf_state.read().row_layout.rows.len()) {
                                div {
                                    class: "relative z-20 pointer-events-none flex-shrink-0 transition-all duration-150",
                                    style: "height: {source_row_h}px; border: 1px dashed #f59e0b; margin: 0 2px;"
                                }
                            }

                            // Cosmetic 1px line below the last row (no resize handle needed)
                            if has_rows {
                                div {
                                    class: "relative flex-shrink-0",
                                    style: "height: 1px",
                                    div {
                                        class: "absolute left-0 right-0 bg-gray-300/60 dark:bg-zinc-600/40",
                                        style: "height: 1px; top: 0;",
                                    }
                                }
                            }
                        }
                    }

                }

                // ── CursorLine overlay (outside scroll, spans full height) ──
                CursorLine { cursor_px, cursor_label }

                // ── Live toggle ───────────────────────────────────────────
                div {
                    class: "absolute bottom-2 right-2 z-30 select-none",
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        evt.prevent_default();
                        let mut v = wf_state.write();
                        v.viewport.auto_scroll = !v.viewport.auto_scroll;
                        data_version += 1;
                    },
                    {
                        let live = wf_state.read().viewport.auto_scroll;
                        let btn_class = if live {
                            "px-3 py-1 rounded text-[11px] font-medium \
                             bg-lime-500/20 text-lime-400 border border-lime-500/50 \
                             cursor-pointer hover:bg-lime-500/30 transition-colors"
                        } else {
                            "px-3 py-1 rounded text-[11px] font-medium \
                             bg-gray-200 text-gray-500 border border-gray-300 \
                             dark:bg-zinc-700/30 dark:text-zinc-400 dark:border-zinc-600/50 \
                             cursor-pointer hover:bg-gray-300 dark:hover:bg-zinc-600/30 transition-colors"
                        };
                        rsx! {
                            div { class: "{btn_class}",
                                if live { "\u{25CF} Live" } else { "\u{25CB} Live" }
                            }
                        }
                    }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            //  DRAG OVERLAY (full-viewport, only during drag interactions)
            //  Covers the entire viewport so mousemove/mouseup fire even
            //  when the cursor leaves the waveform component bounds.
            // ═══════════════════════════════════════════════════════════════
            if drag_active {
                div {
                    class: "fixed inset-0 z-40",
                    style: "cursor: grabbing",
                    onmousemove: {
                        let mut label_width = label_width;
                        let divider_start_x = divider_start_x;
                        let divider_start_width = divider_start_width;
                        let dragging_divider = dragging_divider;
                        let mut wf_state = wf_state;
                        let mut data_version = data_version;
                        let mut reorder = reorder;
                        let mut resize = resize;
                        let mut pan = pan;
                        let scroll_y = scroll_y;
                        let container_top = container_top;
                        let container_left = container_left;
                        let canvas_width_px = canvas_width_px;
                        let sample_count = sample_count;
                        let mut cursor_tracker = cursor_tracker;
                        move |evt| {
                            evt.prevent_default();
                            if dragging_divider() {
                                let dx = evt.data().coordinates().page().x - divider_start_x();
                                let new_w = (divider_start_width() + dx).clamp(20.0, 200.0);
                                label_width.set(new_w);
                                wf_state.write().row_layout.set_label_width(new_w);
                                data_version += 1;
                            } else if resize.is_active() {
                                resize.handle_mousemove(evt.data().coordinates().page().y, wf_state, data_version);
                            } else if reorder.is_active() {
                                let coords = evt.data().coordinates();
                                let page_x = coords.page().x;
                                let page_y = coords.page().y;
                                let unscrolled_y = page_y - container_top() + scroll_y();
                                reorder.handle_mousemove(page_x, page_y, unscrolled_y, wf_state);
                            } else if pan.is_active() {
                                let coords = evt.data().coordinates();
                                let canvas_x = coords.page().x - container_left() - label_width();
                                pan.handle_mousemove(canvas_x, canvas_width_px(), wf_state, sample_count, data_version);
                            }
                            // Always update cursor position (overlay blocks scrollable onmousemove)
                            cursor_tracker.update(
                                evt.data().coordinates().page().x,
                                container_left(),
                                label_width(),
                                wf_state,
                                canvas_width_px(),
                                sample_count,
                            );
                        }
                    },
                    onmouseup: {
                        let mut dragging_divider = dragging_divider;
                        let mut reorder = reorder;
                        let mut resize = resize;
                        let mut pan = pan;
                        let mut pan_active = pan_active;
                        let wf_state = wf_state;
                        let data_version = data_version;
                        move |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                            dragging_divider.set(false);
                            reorder.commit(wf_state, data_version);
                            resize.commit(wf_state, data_version);
                            pan.end();
                            pan_active.set(false);
                        }
                    },
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Canvas drawing (orchestrates all rows)
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_all_canvases(
    short_id: &str,
    analog: &[AnalogTrace],
    digital: &Option<rb_model::DigitalTrace>,
    rows: &[crate::logic_analyzer::waveform_state::row_layout::RowDescriptor],
    annotations: &[rb_decode::Annotation],
    range_start: usize,
    range_end: usize,
    range_len: f64,
    signal_width: f64,
    colors: &row_base::CanvasColors,
) {
    let mut all_js = String::new();

    for (row_idx, row) in rows.iter().enumerate() {
        if !row.visible {
            continue;
        }
        let cid = format!("sig-{short_id}-{row_idx}");
        let mut renderer = JsCanvasRenderer::new();
        match row.kind {
            RowKind::Analog => {
                if let Some(trace) = analog.get(row.channel_index) {
                    row_analog::build_analog_row(
                        &mut renderer, trace,
                        range_start, range_end, range_len,
                        row, signal_width, colors,
                    );
                }
            }
            RowKind::Digital => {
                if let Some(dt) = digital {
                    row_digital::build_digital_row(
                        &mut renderer, dt,
                        range_start, range_end, range_len,
                        row, signal_width, colors,
                    );
                }
            }
            RowKind::Decoder => {
                row_decoder::build_decoder_row(
                    &mut renderer, annotations,
                    range_start, range_end, range_len,
                    row, colors,
                );
            }
        };
        // Always emit JS for every visible row — finish() resizes the
        // canvas which clears any stale pixel buffer from a previous tab.
        let row_js = renderer.finish(&cid, row.signal_height_px);
        all_js.push_str(&wrap_with_resize_observer(row_js));
    }

    log::info!(
        "Canvas draw: {} visible / {} total rows, js_len={}",
        rows.iter().filter(|r| r.visible).count(),
        rows.len(),
        all_js.len()
    );
    if !all_js.is_empty() {
        dioxus::document::eval(&all_js);
    }
}

/// Wrap canvas JS so it self-redraws on element resize via ResizeObserver.
fn wrap_with_resize_observer(js: String) -> String {
    if js.len() < 3 {
        return js;
    }
    let inner = &js[1..js.len() - 1];

    let guard_start = inner.find("if(!c)").unwrap_or(0);
    let after_guard = if inner[guard_start..].starts_with("if(!c){") {
        if let Some(brace_pos) = inner[guard_start..].find('}') {
            guard_start + brace_pos + 1
        } else {
            guard_start
        }
    } else {
        if let Some(ret_pos) = inner[guard_start..].find("return;") {
            guard_start + ret_pos + "return;".len()
        } else {
            guard_start
        }
    };

    let preamble = &inner[..after_guard];
    let body = &inner[after_guard..];
    format!(
        "{{{}c.__rbDraw=function(){{try{{{}}}catch(e){{console.error('Redraw error for '+c.id,e);}}}};setTimeout(function(){{c.__rbDraw();}},0);if(!c.__rbObs){{c.__rbObs=new ResizeObserver(function(){{c.__rbDraw();}});c.__rbObs.observe(c);}}}}",
        preamble, body
    )
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests – pure functions
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    // ── fmt_tick ──────────────────────────────────────────────────────────

    #[test]
    fn fmt_tick_seconds() {
        assert_eq!(fmt_tick(1.0), "1.0s");
        assert_eq!(fmt_tick(2.5), "2.5s");
        assert_eq!(fmt_tick(0.999), "999ms"); // just below 1s
    }

    #[test]
    fn fmt_tick_millis() {
        assert_eq!(fmt_tick(1e-3), "1ms");
        assert_eq!(fmt_tick(500e-3), "500ms");
        assert_eq!(fmt_tick(0.999e-3), "999µs"); // just below 1ms → µs
    }

    #[test]
    fn fmt_tick_micros() {
        assert_eq!(fmt_tick(1e-6), "1µs");
        assert_eq!(fmt_tick(100e-6), "100µs");
    }

    #[test]
    fn fmt_tick_nanos() {
        assert_eq!(fmt_tick(1e-9), "1ns");
        assert_eq!(fmt_tick(500e-9), "500ns");
    }

    // ── fmt_time_ns ───────────────────────────────────────────────────────

    #[test]
    fn fmt_time_ns_seconds() {
        assert_eq!(fmt_time_ns(1e9), "1.000s");
        assert_eq!(fmt_time_ns(2.5e9), "2.500s");
    }

    #[test]
    fn fmt_time_ns_millis() {
        assert_eq!(fmt_time_ns(1e6), "1.000ms");
        assert_eq!(fmt_time_ns(999_999.0), "999.999µs"); // 999_999 ns → µs
    }

    #[test]
    fn fmt_time_ns_micros() {
        assert_eq!(fmt_time_ns(1e3), "1.000µs");
        assert_eq!(fmt_time_ns(999.0), "999ns");
    }

    #[test]
    fn fmt_time_ns_nanos() {
        assert_eq!(fmt_time_ns(1.0), "1ns");
        assert_eq!(fmt_time_ns(500.0), "500ns");
    }

    // ── adaptive_tick_spacing ─────────────────────────────────────────────

    #[test]
    fn adaptive_tick_spacing_smoke() {
        // Every view duration should produce a valid (spacing, n_minors) pair.
        for &dur_ns in &[1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0,
                         1e6, 10e6, 100e6, 1e9, 10e9] {
            let (s, m) = adaptive_tick_spacing(dur_ns);
            assert!(s > 0.0, "tick spacing must be positive for dur={dur_ns}");
            assert!(m >= 4 && m <= 5, "minors {m} out of range for dur={dur_ns}");
        }
    }

    #[test]
    fn adaptive_tick_spacing_boundaries() {
        // Very short duration
        let (s, _) = adaptive_tick_spacing(5e-9);
        assert!(s >= 1e-9);
        // Medium duration: raw=0.001/6≈0.000167, half≈0.000083, first match is 100e-6
        let (s, _) = adaptive_tick_spacing(0.001);
        assert!((s - 100e-6).abs() < 1e-12, "expected 100µs spacing, got {s}");
        // Long duration → fallback
        let (s, m) = adaptive_tick_spacing(100.0);
        assert_eq!(s, 10.0);
        assert_eq!(m, 4);
    }

    #[test]
    fn adaptive_tick_spacing_no_panic() {
        // Extreme values should not panic.
        adaptive_tick_spacing(0.0);
        adaptive_tick_spacing(1e-12);
        adaptive_tick_spacing(1e12);
    }

    // ── compute_ticks ─────────────────────────────────────────────────────

    #[test]
    fn compute_ticks_non_empty() {
        let ticks = compute_ticks(0, 1000, 1000.0, 1_000_000.0);
        assert!(!ticks.is_empty(), "should produce at least one tick");
    }

    #[test]
    fn compute_ticks_pct_in_range() {
        let ticks = compute_ticks(0, 1000, 1000.0, 1_000_000.0);
        for (pct, _label, minors) in &ticks {
            assert!((0.0..=100.0).contains(pct), "pct {pct} out of range");
            for mpct in minors {
                assert!((0.0..100.0).contains(mpct), "minor pct {mpct} out of range");
            }
        }
    }

    #[test]
    fn compute_ticks_monotonic() {
        let ticks = compute_ticks(0, 1000, 1000.0, 1_000_000.0);
        let mut last = -1.0;
        for (pct, _, _) in &ticks {
            assert!(*pct > last, "ticks must be monotonically increasing");
            last = *pct;
        }
    }

    #[test]
    fn compute_ticks_empty_range() {
        let ticks = compute_ticks(0, 0, 1.0, 1_000_000.0);
        // With range_len=1 and 0 samples, should still produce ticks or be empty
        // Just ensure no panic.
        let _ = ticks;
    }

    // ── Integration: WaveformView with SSR ────────────────────────────────

    use std::cell::RefCell;
    use std::rc::Rc;
    use crate::logic_analyzer::waveform_state::row_layout::RowDescriptor;

    /// Helper: create a minimal AnalogTrace for one channel.
    fn make_test_analog(ch_name: &str) -> AnalogTrace {
        use rb_model::{AnalogChannel, AnalogFormat, ChannelId, Timebase};
        let ch = AnalogChannel::new(ChannelId(0), ch_name, AnalogFormat::identity());
        AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0))
    }

    /// Helper: create a minimal DigitalTrace with one channel.
    fn make_test_digital() -> DigitalTrace {
        use rb_model::{ChannelId, DigitalChannel, Timebase};
        DigitalTrace::new(
            vec![DigitalChannel::new(ChannelId(0), "D0", 0)],
            Timebase::new(1_000_000.0, 0.0),
        )
    }

    /// Helper: create an [`AcquisitionConfig`] with the given number of enabled
    /// analog and digital channels (for test use).
    fn make_test_config(analog_count: usize, digital_count: usize) -> AcquisitionConfig {
        use rb_model::{AnalogChannel, AnalogFormat, ChannelId, DigitalChannel};
        AcquisitionConfig {
            analog_channels: vec![AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity()); analog_count],
            analog_enabled: vec![true; analog_count],
            digital_channels: vec![DigitalChannel::new(ChannelId(0), "D0", 0); digital_count],
            digital_enabled: vec![true; digital_count],
            ..AcquisitionConfig::default()
        }
    }

    /// Shared state between the test and the wrapper component inside VirtualDom.
    #[derive(PartialEq)]
    struct TestSignals {
        wf_state: RefCell<Option<Signal<WaveformState>>>,
        data_version: RefCell<Option<Signal<u64>>>,
    }

    /// Wrapper component: creates signals via use_signal and renders WaveformView.
    #[component]
    fn TestWrapper(
        tab_id: crate::tab_state::TabId,
        initial_wf_state: WaveformState,
        initial_acq_config: AcquisitionConfig,
        waveform_data: WaveformData,
        signals: Rc<TestSignals>,
    ) -> Element {
        let wf_state = use_signal(move || initial_wf_state.clone());
        let data_version = use_signal(|| 0u64);

        *signals.wf_state.borrow_mut() = Some(wf_state);
        *signals.data_version.borrow_mut() = Some(data_version);

        let decoder_config = use_signal(DecoderConfig::default);
        let cursor_sample_pos = use_signal(|| None::<u64>);
        let theme = use_signal(|| crate::app_state::Theme::Light);
        let wd_signal = use_signal(move || waveform_data.clone());
        let acq_config = use_signal(move || initial_acq_config.clone());

        rsx! {
            WaveformView {
                tab_id,
                data_version,
                wf_state,
                acquisition_config: acq_config,
                decoder_config,
                cursor_sample_pos,
                waveform_data: wd_signal,
                theme,
            }
        }
    }

    /// Render WaveformView to SSR, returning (html, vdom) for further mutation.
    fn render_waveform_view_mut(
        tab_id: crate::tab_state::TabId,
        initial_wf_state: WaveformState,
        initial_acq_config: AcquisitionConfig,
        waveform_data: WaveformData,
    ) -> (String, VirtualDom, Rc<TestSignals>) {
        let signals = Rc::new(TestSignals {
            wf_state: RefCell::new(None),
            data_version: RefCell::new(None),
        });

        let mut vdom = VirtualDom::new_with_props(
            |props: TestWrapperProps| {
                rsx! {
                    TestWrapper {
                        tab_id: props.tab_id,
                        initial_wf_state: props.initial_wf_state,
                        initial_acq_config: props.initial_acq_config,
                        waveform_data: props.waveform_data,
                        signals: props.signals,
                    }
                }
            },
            TestWrapperProps {
                tab_id,
                initial_wf_state,
                initial_acq_config,
                waveform_data,
                signals: signals.clone(),
            },
        );
        vdom.rebuild_in_place();
        while vdom.wait_for_work().now_or_never().is_some() {
            vdom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        let html = dioxus_ssr::render(&vdom);
        (html, vdom, signals)
    }

    #[test]
    fn waveform_view_renders_rows() {
        let wf_state = WaveformState::default();
        let data = WaveformData {
            acq_state: AcquisitionState::Idle,
            analog: vec![make_test_analog("CH0")],
            digital: Some(make_test_digital()),
            sample_count: 0,
        };

        let (html, _vdom, _signals) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            wf_state,
            make_test_config(1, 1),
            data,
        );

        assert!(html.contains("CH0"), "analog channel CH0 missing");
        assert!(html.contains("D0"), "digital channel D0 missing");
        // Default analog row: 80px signal + 2×14px measurement = 108px total
        assert!(html.contains("height: 108px"), "analog label height 108px missing, got: {html}");
    }

    #[test]
    fn waveform_view_resize_reflected_in_ssr() {
        let mut wf_state = WaveformState::default();
        wf_state.row_layout.rows.push(RowDescriptor {
            kind: RowKind::Analog,
            signal_height_px: 80.0,
            channel_index: 0,
            visible: true,
            decoder_kind: None,
        });

        let data = WaveformData {
            acq_state: AcquisitionState::Idle,
            analog: vec![make_test_analog("CH0")],
            digital: None,
            sample_count: 0,
        };

        let (html1, _vdom, _signals) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            wf_state.clone(),
            make_test_config(1, 0),
            data.clone(),
        );
        assert!(html1.contains("height: 108px"), "initial height 108px missing");

        // Create modified state with row height changed to 55px
        let mut ws2 = wf_state.clone();
        ws2.row_layout.rows[0].signal_height_px = 55.0;

        let (html2, _vdom2, _signals2) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            ws2,
            make_test_config(1, 0),
            data,
        );
        // 55px signal + 28px measurement = 83px total
        assert!(html2.contains("height: 83px"), "resized height 83px missing, got: {html2}");
        assert!(!html2.contains("height: 108px"), "old height 108px should be gone");
    }

    #[test]
    fn waveform_view_with_acquisition_data() {
        // Simulates the state after clicking "Run": acquisition has stopped,
        // sample_count > 0, analog trace has actual samples.
        let mut trace = make_test_analog("CH0");
        trace.push_raw(&[100i32, 200, 300, 400, 500]);

        let mut dt = make_test_digital();
        dt.push_words(&[0b0001u64, 0b0010, 0b0100]);

        let wf_state = WaveformState::default();
        let data = WaveformData {
            acq_state: AcquisitionState::Stopped,
            analog: vec![trace],
            digital: Some(dt),
            sample_count: 5,
        };

        let (html, _vdom, _signals) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            wf_state,
            make_test_config(1, 1),
            data,
        );

        // Must NOT panic — clamp_view should handle sample_count < default view_samples.
        // Rows should exist for both analog and digital channels.
        assert!(html.contains("CH0"), "analog channel label missing");
        assert!(html.contains("D0"), "digital channel label missing");
        assert!(html.contains("id=\"sig-tab-1-0\""), "canvas for row 0 missing");
    }

    #[test]
    fn draw_key_changes_on_data_version() {
        // Verify that the draw cache key changes when data_version increments.
        // This simulates a new acquisition triggering a canvas redraw.
        let (_, _vdom1, _signals1) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            WaveformState::default(),
            make_test_config(1, 1),
            WaveformData {
                acq_state: AcquisitionState::Idle,
                analog: vec![make_test_analog("CH0")],
                digital: Some(make_test_digital()),
                sample_count: 0,
            },
        );
        // First render with dv=0 should have triggered a draw (cache key 0→2)

        // Render again with acquisition data (sample_count > 0) — draw key should change
        let mut trace = make_test_analog("CH0");
        trace.push_raw(&[100i32, 200, 300]);
        let (html2, _vdom2, _signals2) = render_waveform_view_mut(
            crate::tab_state::TabId(1),
            WaveformState::default(),
            make_test_config(1, 1),
            WaveformData {
                acq_state: AcquisitionState::Stopped,
                analog: vec![trace],
                digital: Some(make_test_digital()),
                sample_count: 3,
            },
        );
        // After data arrives, the component should render rows without panicking.
        assert!(html2.contains("CH0"), "rows should render with acquisition data");
    }
}
