//! Waveform display: multi-canvas architecture with HTML overlays.
//!
//! Each signal row has its own `<canvas>` (drawing only the waveform).
//! Labels, markers, cursor line, and time ruler are HTML/CSS elements.
//! A single `use_effect` draws all canvases on each `data_version` change.

use dioxus::prelude::*;
use rb_canvas::{Canvas, JsCanvasRenderer, RgbaColor};
use rb_core::AcquisitionState;
use rb_decode::AnnotationKind;
use rb_model::AnalogTrace;

use crate::waveform_state::{
    RowKind, TimeMarker, WaveformView, DIVIDER_H, LABEL_W,
    MARKER_BAR_H, MEASUREMENT_ZONE_H, TIME_RULER_H,
};

use super::app::AppStateRef;

// ── Colors ───────────────────────────────────────────────────────────────────

const ANALOG_COLORS_RGBA: [RgbaColor; 8] = [
    RgbaColor { r: 0xfa, g: 0xcc, b: 0x15, a: 255 },
    RgbaColor { r: 0x60, g: 0xa5, b: 0xfa, a: 255 },
    RgbaColor { r: 0xf8, g: 0x71, b: 0x71, a: 255 },
    RgbaColor { r: 0x34, g: 0xd3, b: 0x99, a: 255 },
    RgbaColor { r: 0xc0, g: 0x84, b: 0xfc, a: 255 },
    RgbaColor { r: 0xfb, g: 0x92, b: 0x3c, a: 255 },
    RgbaColor { r: 0x2d, g: 0xd4, b: 0xbf, a: 255 },
    RgbaColor { r: 0xf4, g: 0x72, b: 0xb6, a: 255 },
];

const DIGITAL_COLOR_RGBA: RgbaColor = RgbaColor { r: 0x58, g: 0xa6, b: 0xff, a: 255 };
const BG_COLOR_RGBA: RgbaColor = RgbaColor { r: 0x0d, g: 0x11, b: 0x17, a: 255 };
const GRID_COLOR_RGBA: RgbaColor = RgbaColor { r: 0x1a, g: 0x1a, b: 0x2e, a: 255 };
const ZERO_LINE_COLOR_RGBA: RgbaColor = RgbaColor { r: 0x30, g: 0x36, b: 0x3d, a: 255 };
const DECODER_BG_RGBA: RgbaColor = RgbaColor { r: 0x16, g: 0x1b, b: 0x22, a: 255 };
const DECODER_TEXT_RGBA: RgbaColor = RgbaColor { r: 0xc9, g: 0xd1, b: 0xd9, a: 255 };

const CURSOR_COLOR: &str = "#f0f6fc";

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

// ═══════════════════════════════════════════════════════════════════════════════
//  Main component
// ═══════════════════════════════════════════════════════════════════════════════

#[component]
pub fn WaveformCanvas(
    device_id: rb_device::DeviceId,
    data_version: Signal<u64>,
    mut view: Signal<WaveformView>,
    mut cursor_sample_pos: Signal<Option<u64>>,
) -> Element {
    let _version = data_version();
    let state: AppStateRef = use_context();

    // ── Gather data ───────────────────────────────────────────────────────
    let (acq_state, analog, digital, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (acq.state().clone(), acq.analog_traces().to_vec(),
             acq.digital_trace().cloned(), acq.sample_count())
        } else if let Some(h) = s.session.device(&device_id) {
            (h.state().clone(), h.analog_traces().to_vec(),
             h.digital_trace().cloned(), h.sample_count())
        } else { (AcquisitionState::Idle, Vec::new(), None, 0) }
    };
    let is_running = matches!(acq_state, AcquisitionState::Running);

    // ── Dynamic canvas width (replaces hardcoded 800.0) ────────────────
    // Default 764 = 800 – LABEL_W (36). Updated async on mount via
    // get_client_rect (works cross-platform: wasm32 + desktop webview).
    let canvas_width_px = use_signal(|| 764.0f64);

    // Signal-ized sample_count so the drawing effect always sees the
    // latest value (avoid closure-capture staleness).
    let mut sample_count_sig = use_signal(|| sample_count);
    sample_count_sig.set(sample_count);

    // ── Update view ───────────────────────────────────────────────────────
    {
        let mut v = view.write();
        if sample_count > 0 { v.clamp_view(sample_count, is_running); }
        let dcc = digital.as_ref().map(|dt| dt.channels().len()).unwrap_or(0);
        v.rebuild_rows(analog.len(), dcc);
        if let Some(ref dt) = digital { v.feed_decoder(dt); }
    }
    { let mut s = state.borrow_mut(); s.views.insert(device_id.clone(), view.read().clone()); }

    // ── Derived ───────────────────────────────────────────────────────────
    let rows = view.read().rows.clone();
    let range_start = view.read().view_start;
    let range_end = (view.read().view_start + view.read().view_samples).min(sample_count);
    let range_len = (range_end - range_start).max(1) as f64;

    let sample_rate_hz = 1_000_000.0; // TODO: real sample rate
    let tick_elements = compute_ticks(range_start, range_end, range_len, sample_rate_hz);

    let short_id = device_id.as_str().replace(':', "-");

    // ── Single drawing effect ─────────────────────────────────────────────
    {
        let short_id = short_id.clone();
        let data_version = data_version;
        let sample_count_sig = sample_count_sig;
        let state = state.clone();
        let device_id = device_id.clone();
        let view = view;
        use_effect(move || {
            let _ver = data_version();
            let sc = sample_count_sig();
            // Re-read fresh analog/digital data from state on every effect run.
            // This avoids the stale-closure bug where the first render (before
            // data arrives) captures empty Vec/None permanently.
            let (analog, digital) = {
                let s = state.borrow();
                if let Some(acq) = s.acquisitions.get(&device_id) {
                    (acq.analog_traces().to_vec(), acq.digital_trace().cloned())
                } else if let Some(h) = s.session.device(&device_id) {
                    (h.analog_traces().to_vec(), h.digital_trace().cloned())
                } else { (Vec::new(), None) }
            };
            let v = view.read();
            let rs = v.view_start;
            let re = (v.view_start + v.view_samples).min(sc);
            let rl = (re - rs).max(1) as f64;
            let rows_snap = v.rows.clone();
            let annotations_snap = v.annotations.clone();
            drop(v);
            draw_all_canvases(&short_id, &analog, &digital, &rows_snap, &annotations_snap, rs, re, rl, canvas_width_px());
        });
    }

    let mut drag_active = use_signal(|| false);
    // Sample position under the cursor at mousedown (grab point).
    let mut grab_sample = use_signal(|| None::<u64>);
    let mut divider_drag_row = use_signal(|| None::<usize>);
    let mut divider_drag_start_y = use_signal(|| 0.0f64);
    let mut divider_drag_start_h = use_signal(|| 0.0f64);
    // Current live height during a divider drag (CSS only, not committed to view).
    let mut divider_drag_live_h = use_signal(|| None::<f64>);
    let mut cursor_px = use_signal(|| None::<f64>);
    let mut cursor_label = use_signal(|| String::new());

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
                    rsx! { span { class: "text-[9px] text-zinc-300 truncate", "{name}" } }
                }
                RowKind::Digital => {
                    let name = digital.as_ref()
                        .and_then(|dt| dt.channels().get(ci))
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| format!("D{}", ci));
                    rsx! { span { class: "text-[9px] text-zinc-400 truncate", "{name}" } }
                }
                RowKind::Decoder => {
                    rsx! { span { class: "text-[9px] text-zinc-500 truncate", "DEC" } }
                }
            };
            (idx, signal_id, label_el, row_h, sig_h)
        })
        .collect();

    // ═══════════════════════════════════════════════════════════════════════
    //  Render
    // ═══════════════════════════════════════════════════════════════════════
    rsx! {
        div { class: "flex flex-col h-full bg-[#0d1117]",
            // ── Time ruler ────────────────────────────────────────────
            TimeRuler { tick_elements }

            // ── Marker bar ────────────────────────────────────────────
            MarkerBar {
                markers: view.read().markers.clone(),
                range_start,
                range_len,
            }

            // ── Rows container ────────────────────────────────────────────
            div {
                id: "rows-{short_id}",
                class: "flex-1 overflow-y-auto relative",
                onmounted: {
                    let canvas_width_px = canvas_width_px;
                    move |data| {
                        let mut cw = canvas_width_px;
                        // get_client_rect is async; spawn a task to read the
                        // actual width once the DOM has settled. Works on both
                        // wasm32 and desktop webview.
                        spawn(async move {
                            if let Ok(rect) = data.get_client_rect().await {
                                let w = rect.width();
                                if w > 0.0 {
                                    cw.set((w - LABEL_W).max(100.0));
                                }
                            }
                        });
                    }
                },
                onwheel: move |evt| {
                    evt.prevent_default();
                    let (dx, dy) = match evt.data().delta() {
                        dioxus::html::geometry::WheelDelta::Pixels(v) => (v.x, v.y),
                        dioxus::html::geometry::WheelDelta::Lines(v) => (v.x * 20.0, v.y * 20.0),
                        dioxus::html::geometry::WheelDelta::Pages(v) => (v.x * 200.0, v.y * 200.0),
                    };
                    // Horizontal scroll (Shift+Wheel, touchpad) → pan
                    if dx.abs() > 0.01 {
                        // Positive dx (scroll right) → show newer data → increase view_start.
                        // pan() with positive delta_px decreases view_start, so negate.
                        view.write().pan(-dx as f32, canvas_width_px() as f32, sample_count);
                    } else {
                        let factor: f64 = if dy < 0.0 { 0.8 } else { 1.25 };
                        view.write().zoom(factor, sample_count);
                    }
                    data_version += 1;
                },
                onmousedown: move |evt| {
                    let coords = evt.data().coordinates();
                    let py = coords.page().y;
                    let el_y = coords.element().y;
                    let el_x = coords.element().x;
                    let v = view.read();
                    if let Some(ri) = v.row_at_y(el_y) {
                        let rt = v.row_y_offset(ri);
                        let rh = v.rows[ri].total_height();
                        if el_y >= rt + rh - DIVIDER_H && el_y < rt + rh {
                            let sh = v.rows.get(ri).map(|r| r.signal_height_px).unwrap_or(22.0);
                            drop(v);
                            divider_drag_row.set(Some(ri));
                            divider_drag_start_y.set(py);
                            divider_drag_start_h.set(sh);
                            return;
                        }
                    }
                    // Grab: remember the sample under the cursor so it
                    // follows the mouse 1:1 during drag.
                    let gs = if el_x >= LABEL_W {
                        let cw = canvas_width_px();
                        if cw > 0.0 {
                            let frac = ((el_x - LABEL_W) / cw).clamp(0.0, 1.0);
                            Some(v.view_start as u64 + (frac * v.view_samples as f64) as u64)
                        } else { None }
                    } else { None };
                    drop(v);
                    grab_sample.set(gs);
                    drag_active.set(true);
                },
                onmousemove: move |evt| {
                    let coords = evt.data().coordinates();
                    let py = coords.page().y;
                    let cx = coords.element().x;
                    if let Some(_ri) = divider_drag_row() {
                        let dy = py - divider_drag_start_y();
                        let new_h = (divider_drag_start_h() + dy).max(10.0);
                        divider_drag_live_h.set(Some(new_h));
                        return;
                    }
                    if drag_active() {
                        if let Some(gs) = grab_sample() {
                            let mut v = view.write();
                            let cw = canvas_width_px();
                            if cw > 0.0 && cx >= LABEL_W {
                                let frac = ((cx - LABEL_W) / cw).clamp(0.0, 1.0);
                                let offset = (frac * v.view_samples as f64) as u64;
                                let new_vs = gs.saturating_sub(offset);
                                let max_vs = sample_count.saturating_sub(v.view_samples);
                                v.view_start = (new_vs as usize).min(max_vs);
                            }
                            v.auto_scroll = false;
                        }
                        data_version += 1;
                    }
                    let v = view.read();
                    let rs = v.view_start;
                    let re = (v.view_start + v.view_samples).min(sample_count);
                    let rl = (re - rs).max(1) as f64;
                    let cw = canvas_width_px();
                    let sp = if cx >= LABEL_W {
                        rs as u64 + (((cx - LABEL_W) / cw.max(1.0)).clamp(0.0, 1.0) * rl) as u64
                    } else { rs as u64 };
                    drop(v);
                    cursor_sample_pos.set(Some(sp));
                    cursor_px.set(Some(cx));
                    cursor_label.set(fmt_time_ns((sp as f64 / 1_000_000.0) * 1e9));
                },
                onmouseup: move |_| {
                    if let (Some(ri), Some(h)) = (divider_drag_row(), divider_drag_live_h()) {
                        view.write().set_row_height(ri, h);
                        data_version += 1;
                    }
                    drag_active.set(false);
                    grab_sample.set(None);
                    divider_drag_row.set(None);
                    divider_drag_live_h.set(None);
                },
                onmouseleave: move |_| {
                    // Commit any in-progress resize on leave
                    if let (Some(ri), Some(h)) = (divider_drag_row(), divider_drag_live_h()) {
                        view.write().set_row_height(ri, h);
                        data_version += 1;
                    }
                    drag_active.set(false); grab_sample.set(None);
                    divider_drag_row.set(None); divider_drag_live_h.set(None);
                    cursor_sample_pos.set(None); cursor_px.set(None);
                },

                // ── Cursor overlay ────────────────────────────────────
                CursorLine {
                    cursor_px,
                    cursor_label,
                }

                // ── Signal rows ───────────────────────────────────────
                for (row_idx_c, signal_id, label_el, row_h, sig_h) in &visible_rows {
                    {
                        let row_idx_c = *row_idx_c;
                        let row_h = *row_h;
                        let sig_h = if divider_drag_row() == Some(row_idx_c) {
                            divider_drag_live_h().unwrap_or(*sig_h)
                        } else { *sig_h };
                        let effective_row_h = if divider_drag_row() == Some(row_idx_c) {
                            sig_h + 2.0 * MEASUREMENT_ZONE_H + DIVIDER_H
                        } else { row_h };
                        rsx! {
                            div {
                                class: "flex relative",
                                style: "height: {effective_row_h}px",
                                // Label
                                div {
                                    class: "flex-shrink-0 bg-[#0a0e14] border-r border-[#1a1a2e] flex items-center px-1 select-none",
                                    style: "width: {LABEL_W}px",
                                    oncontextmenu: {
                                        let mut view = view;
                                        let ri = row_idx_c;
                                        move |evt| {
                                            evt.prevent_default();
                                            evt.stop_propagation();
                                            if let Some(r) = view.write().rows.get_mut(ri) {
                                                r.visible = !r.visible;
                                            }
                                        }
                                    },
                                    {label_el}
                                }
                                // Canvas
                                canvas {
                                    id: "{signal_id}",
                                    class: "flex-1 pointer-events-none",
                                    style: "height: {sig_h}px; margin-top: {MEASUREMENT_ZONE_H}px; margin-bottom: {MEASUREMENT_ZONE_H}px",
                                    width: "100%",
                                    height: "{sig_h}",
                                }
                                // Divider handle (events handled by parent onmousedown/onmousemove)
                                div {
                                    class: "absolute left-0 right-0 cursor-ns-resize bg-[#21262d] hover:bg-zinc-500/40",
                                    style: "height: 1px; bottom: 0",
                                }
                            }
                        }
                    }
                }
            }
        }
    }
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

// ═══════════════════════════════════════════════════════════════════════════════
//  Sub-components
// ═══════════════════════════════════════════════════════════════════════════════

/// Time ruler showing major and minor ticks as HTML elements.
#[component]
fn TimeRuler(tick_elements: Vec<(f64, String, Vec<f64>)>) -> Element {
    rsx! {
        div {
            class: "w-full flex-shrink-0 relative bg-[#0a0e14] border-b border-[#30363d] select-none",
            style: "height: {TIME_RULER_H}px",
            for (pct, label, minor_pcts) in &tick_elements {
                // Major tick
                div {
                    class: "absolute top-1/2 bottom-0 border-l border-[#30363d]",
                    style: "left: calc({LABEL_W}px + {pct:.2}% * (100% - {LABEL_W}px) / 100%)"
                }
                span {
                    class: "absolute text-[9px] text-[#8b949e]",
                    style: "left: calc({LABEL_W}px + {pct:.2}% * (100% - {LABEL_W}px) / 100% + 2px); top: 0",
                    "{label}"
                }
                // Minor ticks
                for mpct in minor_pcts {
                    div {
                        class: "absolute top-[70%] bottom-0 border-l border-[#30363d] opacity-50",
                        style: "left: calc({LABEL_W}px + {mpct:.2}% * (100% - {LABEL_W}px) / 100%)"
                    }
                }
            }
        }
    }
}

/// Marker bar showing user-placed time markers as HTML overlays.
#[component]
fn MarkerBar(markers: Vec<TimeMarker>, range_start: usize, range_len: f64) -> Element {
    rsx! {
        div {
            class: "relative flex-shrink-0 border-b border-[#1a1a2e]",
            style: "height: {MARKER_BAR_H}px",
            for m in &markers {
                {
                    let sp = m.sample_pos;
                    let lbl = m.label.clone().unwrap_or_else(|| format!("M{}", m.id));
                    let rs = range_start as u64;
                    let rl = range_len;
                    let pct = if rl > 0.0 {
                        ((sp.saturating_sub(rs)) as f64 / rl * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    };
                    rsx! {
                        div {
                            class: "absolute top-0 bottom-0 flex items-center select-none",
                            style: "left: calc({LABEL_W}px + {pct:.2}% * (100% - {LABEL_W}px) / 100%)",
                            span { class: "text-[9px] text-amber-400", "\u{25C6}" }
                            span { class: "text-[9px] text-amber-400 ml-0.5", "{lbl}" }
                        }
                    }
                }
            }
        }
    }
}

/// Vertical cursor line with time label, shown on mouse hover.
#[component]
fn CursorLine(cursor_px: Signal<Option<f64>>, cursor_label: Signal<String>) -> Element {
    let px = cursor_px();
    let label = cursor_label();
    rsx! {
        if let Some(px) = px {
            div {
                class: "pointer-events-none absolute top-0 bottom-0 z-20",
                style: "left: {px}px",
                div {
                    class: "absolute top-0 bottom-0 border-l border-dashed",
                    style: "border-color: {CURSOR_COLOR}; opacity: 0.7"
                }
                div {
                    class: "absolute text-[9px] whitespace-nowrap px-1 rounded",
                    style: "top: 0; left: 4px; color: {CURSOR_COLOR}; background: #0d1117aa",
                    "{label}"
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Canvas drawing
// ═══════════════════════════════════════════════════════════════════════════════

fn draw_all_canvases(
    short_id: &str,
    analog: &[AnalogTrace],
    digital: &Option<rb_model::DigitalTrace>,
    rows: &[crate::waveform_state::RowDescriptor],
    annotations: &[rb_decode::Annotation],
    range_start: usize,
    range_end: usize,
    range_len: f64,
    signal_width: f64,
) {
    let mut all_js = String::new();

    for (row_idx, row) in rows.iter().enumerate() {
        if !row.visible { continue; }
        let cid = format!("sig-{short_id}-{row_idx}");
        let mut renderer = JsCanvasRenderer::new();
        match row.kind {
            RowKind::Analog => {
                if let Some(trace) = analog.get(row.channel_index) {
                    let color = &ANALOG_COLORS_RGBA[row.channel_index % ANALOG_COLORS_RGBA.len()];
                    build_analog_signal(&mut renderer, trace, range_start, range_end, range_len,
                                        row, color, signal_width);
                }
            }
            RowKind::Digital => {
                if let Some(dt) = digital {
                    if let Some(ch) = dt.channels().get(row.channel_index) {
                        build_digital_signal(&mut renderer, dt, ch.bit as usize,
                                             range_start, range_end, range_len,
                                             row, signal_width);
                    }
                }
            }
            RowKind::Decoder => {
                build_decoder_signal(&mut renderer, annotations, range_start, range_end, range_len, row);
            }
        };
        let row_js = renderer.finish(&cid, row.signal_height_px);
        if !row_js.is_empty() {
            all_js.push_str(&wrap_with_resize_observer(row_js));
        }
    }

    log::info!("Canvas draw: {} signal canvases, total js_len={}", rows.iter().filter(|r| r.visible).count(), all_js.len());
    if !all_js.is_empty() {
        dioxus::document::eval(&all_js);
    }
}

// ── Individual canvas JS builders ────────────────────────────────────────────

/// Wrap canvas JS so it self-redraws on element resize via ResizeObserver.
/// Works cross-platform (browser + desktop webview).
fn wrap_with_resize_observer(js: String) -> String {
    if js.len() < 3 { return js; }
    // JsCanvasRenderer already uses `let c=`
    let inner = &js[1..js.len()-1];

    // Find the "if(!c)" guard and split after it.
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

fn build_analog_signal(
    canvas: &mut dyn Canvas, trace: &AnalogTrace,
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor, color: &RgbaColor,
    signal_width: f64,
) {
    let sig_h = row.signal_height_px;
    let samples_per_px = range_len / signal_width.max(1.0);

    if samples_per_px < 1.0 {
        build_analog_per_sample(canvas, trace, range_start, range_end,
                                range_len, sig_h, signal_width, color, samples_per_px);
    } else {
        build_analog_envelope(canvas, trace, range_start, range_end, range_len,
                              sig_h, color, signal_width);
    }
}

/// Per-sample line/point rendering for close-up zoom.
fn build_analog_per_sample(
    canvas: &mut dyn Canvas, trace: &AnalogTrace,
    range_start: usize, range_end: usize, range_len: f64,
    sig_h: f64, signal_width: f64, color: &RgbaColor, samples_per_px: f64,
) {
    let store = trace.store();
    let raw = store.raw();
    let r_start = range_start.min(raw.len());
    let r_end = range_end.min(raw.len());
    if r_start >= r_end {
        return;
    }
    let samples = &raw[r_start..r_end];

    // Compute physical range.
    let (raw_lo, raw_hi) = samples.iter()
        .fold((i32::MAX, i32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let show_dots = samples_per_px < 0.1;

    // ── Background ─────────────────────────────────────────────────────
    canvas.set_fill_style(&BG_COLOR_RGBA);
    canvas.clear();

    // ── Grid ───────────────────────────────────────────────────────────
    canvas.set_stroke_style(&GRID_COLOR_RGBA);
    canvas.set_line_width(0.5);
    canvas.clear_line_dash();
    for i in 0..=5 {
        let gy = i as f64 / 5.0 * sig_h;
        canvas.stroke_line(0.0, gy, signal_width, gy);
    }

    // ── Zero line ──────────────────────────────────────────────────────
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = sig_h - ((0.0 - p_lo) / p_span * sig_h);
        canvas.set_stroke_style(&ZERO_LINE_COLOR_RGBA);
        canvas.set_line_width(1.0);
        canvas.set_line_dash(&[4.0, 4.0]);
        canvas.stroke_line(0.0, zy, signal_width, zy);
        canvas.clear_line_dash();
    }

    // ── Polyline ───────────────────────────────────────────────────────
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

    // ── Sample dots (very close zoom) ──────────────────────────────────
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
) {
    let pixel_count = signal_width.ceil() as usize;
    let range = range_start..range_end;
    let buckets = trace.envelope_buckets(range.clone(), pixel_count);
    if buckets.is_empty() {
        log::warn!("Envelope canvas: no buckets for range {range_start}..{range_end}");
        return;
    }
    log::debug!("Envelope canvas: {} buckets for {pixel_count} px", buckets.len());

    // Compute physical Y range from all visible buckets.
    let (raw_lo, raw_hi) = buckets.iter()
        .filter(|b| b.min != 0 || b.max != 0)
        .fold((i32::MAX, i32::MIN), |(lo, hi), b| (lo.min(b.min), hi.max(b.max)));
    if raw_lo == i32::MAX {
        canvas.set_fill_style(&BG_COLOR_RGBA);
        canvas.clear();
        return;
    }
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    // ── Background ─────────────────────────────────────────────────────
    canvas.set_fill_style(&BG_COLOR_RGBA);
    canvas.clear();

    // ── Grid ───────────────────────────────────────────────────────────
    canvas.set_stroke_style(&GRID_COLOR_RGBA);
    canvas.set_line_width(0.5);
    canvas.clear_line_dash();
    for i in 0..=5 {
        let gy = i as f64 / 5.0 * sig_h;
        canvas.stroke_line(0.0, gy, signal_width, gy);
    }

    // ── Zero line ──────────────────────────────────────────────────────
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = sig_h - ((0.0 - p_lo) / p_span * sig_h);
        canvas.set_stroke_style(&ZERO_LINE_COLOR_RGBA);
        canvas.set_line_width(1.0);
        canvas.set_line_dash(&[4.0, 4.0]);
        canvas.stroke_line(0.0, zy, signal_width, zy);
        canvas.clear_line_dash();
    }

    // ── Per-pixel column envelope (aggregated in Rust) ─────────────────
    let n_c: usize = signal_width.round() as usize;
    let mut c_min = vec![f64::INFINITY; n_c];
    let mut c_max = vec![f64::NEG_INFINITY; n_c];

    for b in &buckets {
        let min_p = trace.to_physical(b.min).clamp(p_lo, p_hi);
        let max_p = trace.to_physical(b.max).clamp(p_lo, p_hi);
        let bx = ((b.start as f64 - range_start as f64) / range_len) * signal_width;
        let nx = if let Some(next) = buckets.get(buckets.iter().position(|x| x.start == b.start).map(|i| i + 1).unwrap_or(buckets.len())) {
            ((next.start as f64 - range_start as f64) / range_len) * signal_width
        } else {
            signal_width + 1.0
        };
        // Handle end of list: use the next bucket or signal_width+1
        let c0 = (bx.floor() as usize).min(n_c.saturating_sub(1));
        let c1 = (nx.ceil() as usize).min(n_c);
        for c in c0..c1 {
            c_min[c] = c_min[c].min(min_p);
            c_max[c] = c_max[c].max(max_p);
        }
    }

    // redo the loop properly: iterate adjacent bucket pairs
    let mut c_min2 = vec![f64::INFINITY; n_c];
    let mut c_max2 = vec![f64::NEG_INFINITY; n_c];
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
            c_min2[c] = c_min2[c].min(min_p);
            c_max2[c] = c_max2[c].max(max_p);
        }
    }

    canvas.set_fill_style(color);
    canvas.set_global_alpha(0.8);
    for c in 0..n_c {
        if c_min2[c] <= c_max2[c] {
            let y0 = (sig_h - ((c_max2[c] - p_lo) / p_span * sig_h)).round();
            let y1 = (sig_h - ((c_min2[c] - p_lo) / p_span * sig_h)).round();
            let h = (y1 - y0).max(1.0);
            canvas.fill_rect(c as f64, y0, 1.0, h);
        }
    }
    canvas.set_global_alpha(1.0);
}

fn build_digital_signal(
    canvas: &mut dyn Canvas, dt: &rb_model::DigitalTrace, bit: usize,
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor,
    signal_width: f64,
) {
    let sig_h = row.signal_height_px;
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
    canvas.set_fill_style(&BG_COLOR_RGBA);
    canvas.clear();

    // Mid line
    canvas.set_stroke_style(&GRID_COLOR_RGBA);
    canvas.set_line_width(0.5);
    canvas.clear_line_dash();
    canvas.stroke_line(0.0, mid_y, signal_width, mid_y);

    canvas.set_fill_style(&DIGITAL_COLOR_RGBA);

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
                canvas.fill_rect(px, high_row, pw, full_h);
            } else {
                canvas.fill_rect(px, cur, pw, 1.0);
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
                canvas.fill_rect(xp(px), cur, xp(ei) - xp(px), 1.0);
                let next: f64 = if (cur - high_row).abs() < 0.5 { low_row } else { high_row };
                let top = cur.min(next);
                let h = (next - cur).abs() + 1.0;
                canvas.fill_rect(xp(ei), top, 1.0, h);
                cur = next;
                px = ei;
            }
            canvas.fill_rect(xp(px), cur, xp(range_end as u64) - xp(px), 1.0);
        }
    }
}

fn build_decoder_signal(
    canvas: &mut dyn Canvas, annotations: &[rb_decode::Annotation],
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor,
) {
    let sig_h = row.signal_height_px;
    canvas.set_fill_style(&DECODER_BG_RGBA);
    canvas.clear();

    let data_color = RgbaColor { r: 0x1f, g: 0x3a, b: 0x6b, a: 255 };
    let addr_color = RgbaColor { r: 0x6b, g: 0x3d, b: 0x1f, a: 255 };
    let frame_color = RgbaColor { r: 0x2d, g: 0x2d, b: 0x2d, a: 255 };
    let error_color = RgbaColor { r: 0x6b, g: 0x1f, b: 0x1f, a: 255 };

    for ann in annotations {
        if ann.range.end <= range_start || ann.range.start >= range_end { continue; }
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
        canvas.set_fill_style(color);
        canvas.fill_rect(x0, 1.0, ww, sig_h - 2.0);
        if ww > 0.03 {
            canvas.set_fill_style(&DECODER_TEXT_RGBA);
            canvas.fill_text(&ann.label, x0 + 2.0, sig_h * 0.5 + 3.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_canvas::PixelCanvas;
    use rb_model::{
        AnalogChannel, AnalogFormat, AnalogTrace,
        DigitalChannel, DigitalTrace,
        ChannelId, Timebase,
    };
    use crate::waveform_state::{RowDescriptor, RowKind};

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_analog_row(sig_h: f64) -> RowDescriptor {
        RowDescriptor {
            kind: RowKind::Analog,
            signal_height_px: sig_h,
            channel_index: 0,
            visible: true,
            decoder_kind: None,
        }
    }

    fn make_digital_row(sig_h: f64, ch_idx: usize) -> RowDescriptor {
        RowDescriptor {
            kind: RowKind::Digital,
            signal_height_px: sig_h,
            channel_index: ch_idx,
            visible: true,
            decoder_kind: None,
        }
    }

    /// Count how many pixels in the canvas match `color` exactly.
    fn count_pixels(canvas: &PixelCanvas, color: RgbaColor) -> usize {
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
    fn assert_grid_line_at(canvas: &PixelCanvas, y: u32, min_len: u32) {
        let mut run = 0u32;
        for x in 0..canvas.width() {
            if canvas.pixel(x, y) == GRID_COLOR_RGBA {
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
    fn any_pixel_is(canvas: &PixelCanvas, color: RgbaColor) -> bool {
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                if canvas.pixel(x, y) == color {
                    return true;
                }
            }
        }
        false
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Analog – per-sample mode
    // ═══════════════════════════════════════════════════════════════════════

    /// A 50-sample sine drawn at 400 px width → 0.125 samples/px
    /// triggers per-sample mode without dots.
    #[test]
    fn analog_per_sample_sine_no_dots() {
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

        build_analog_signal(&mut canvas, &trace, 0, 50, 50.0, &row, &color, 400.0);

        // Top grid line at y=0 is safe from signal overlap (margin keeps
        // signal away from edges).
        assert_grid_line_at(&canvas, 0, 200);

        // The polyline should paint signal-colored pixels.
        let signal_px = count_pixels(&canvas, color);
        assert!(signal_px > 20, "expected signal polyline, found {signal_px} pixels");

        // At x=200 (middle), the ramp value 25 maps to ~mid-height.
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

    /// Very zoomed in: 10-sample sine over 200 px → 0.05 samples/px → dots.
    #[test]
    fn analog_per_sample_with_dots() {
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

        // samples_per_px = 10/200 = 0.05 → per-sample with dots
        build_analog_signal(&mut canvas, &trace, 0, 10, 10.0, &row, &color, 200.0);

        // Dots are filled circles of radius 2.5. Each circle has ~20 pixels.
        let signal_px = count_pixels(&canvas, color);
        assert!(signal_px > 50, "expected dots with many signal pixels, got {signal_px}");

        // Top grid line safe from signal overlap.
        assert_grid_line_at(&canvas, 0, 100);

        save_canvas_png(&canvas, "analog_per_sample_dots.png");
    }

    /// Empty data range → only background drawn.
    #[test]
    fn analog_per_sample_empty() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));

        let row = make_analog_row(80.0);
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(200, 80);

        build_analog_signal(&mut canvas, &trace, 0, 0, 1.0, &row, &color, 200.0);

        let signal_px = count_pixels(&canvas, color);
        assert_eq!(signal_px, 0);
        assert_eq!(canvas.pixel(50, 40), RgbaColor::TRANSPARENT);

        save_canvas_png(&canvas, "analog_per_sample_empty.png");
    }

    /// Data crossing zero → dashed zero line drawn.
    #[test]
    fn analog_per_sample_zero_line() {
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
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(200, 80);

        build_analog_signal(&mut canvas, &trace, 0, 20, 20.0, &row, &color, 200.0);

        // Zero line is dashed ZERO_LINE_COLOR on a background of BG_COLOR.
        assert!(any_pixel_is(&canvas, ZERO_LINE_COLOR_RGBA),
            "dashed zero line should be visible");

        save_canvas_png(&canvas, "analog_per_sample_zero_line.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Analog – envelope mode
    // ═══════════════════════════════════════════════════════════════════════

    /// 1000-sample sine wave at 800 px → 1.25 samples/px → envelope mode.
    #[test]
    fn analog_envelope_sine() {
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
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(800, 80);

        build_analog_signal(&mut canvas, &trace, 0, 1000, 1000.0, &row, &color, 800.0);

        // Top grid line at y=0 — safe from signal overlap.
        assert_grid_line_at(&canvas, 0, 400);

        // Envelope mode uses globalAlpha=0.8 → blended pixels.
        let bg_count = count_pixels(&canvas, BG_COLOR_RGBA);
        assert!(bg_count < canvas.width() as usize * canvas.height() as usize,
            "envelope should draw over background, but all pixels are BG");

        // The envelope should span most of the vertical range.
        let mut min_y = canvas.height();
        let mut max_y = 0u32;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let p = canvas.pixel(x, y);
                if p != BG_COLOR_RGBA && p != GRID_COLOR_RGBA && p != RgbaColor::TRANSPARENT {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let span = max_y - min_y;
        assert!(span > 30, "envelope should span >30 px vertically, got {span}");

        // Zero line present (data crosses zero).
        assert!(any_pixel_is(&canvas, ZERO_LINE_COLOR_RGBA),
            "zero line should appear for AC signal");

        save_canvas_png(&canvas, "analog_envelope_sine.png");
    }

    /// Empty envelope range → only background.
    #[test]
    fn analog_envelope_empty() {
        let ch = AnalogChannel::new(ChannelId(0), "CH0", AnalogFormat::identity());
        let trace = AnalogTrace::new(ch, Timebase::new(1_000_000.0, 0.0));

        let row = make_analog_row(80.0);
        let color = ANALOG_COLORS_RGBA[0];
        let mut canvas = PixelCanvas::new(800, 80);

        build_analog_signal(&mut canvas, &trace, 0, 0, 1.0, &row, &color, 800.0);

        assert_eq!(canvas.pixel(400, 40), RgbaColor::TRANSPARENT);

        save_canvas_png(&canvas, "analog_envelope_empty.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Digital – sparse mode
    // ═══════════════════════════════════════════════════════════════════════

    /// A few transitions on a wide canvas → sparse rendering.
    #[test]
    fn digital_sparse_few_edges() {
        let channels = vec![DigitalChannel::new(ChannelId(0), "D0", 0)];
        let mut trace = DigitalTrace::new(channels, Timebase::new(1_000_000.0, 0.0));
        // Pattern: 0,0,0,1,1,1,0,0,1,1 → edges at sample 3, 6, 8
        let words: Vec<u64> = vec![0b0, 0b0, 0b0, 0b1, 0b1, 0b1, 0b0, 0b0, 0b1, 0b1];
        trace.push_words(&words[..5]);
        trace.push_words(&words[5..]);

        let row = make_digital_row(40.0, 0);
        let mut canvas = PixelCanvas::new(400, 40);

        build_digital_signal(&mut canvas, &trace, 0, 0, 10, 10.0, &row, 400.0);

        // Signal pixels present.
        let sig = count_pixels(&canvas, DIGITAL_COLOR_RGBA);
        assert!(sig > 0, "no digital signal pixels drawn, got {sig}");

        // At x=120 (edge at sample 3), there should be a vertical edge
        // from high_row=10 to low_row=30.
        let edge_x = 120u32;
        let mut edge_pixels = 0;
        for y in 10..=30 {
            if canvas.pixel(edge_x, y) == DIGITAL_COLOR_RGBA {
                edge_pixels += 1;
            }
        }
        assert!(edge_pixels > 0,
            "vertical edge expected at x={edge_x}, found {edge_pixels} signal pixels in [10..30]");

        save_canvas_png(&canvas, "digital_sparse.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Digital – dense mode
    // ═══════════════════════════════════════════════════════════════════════

    /// Rapid toggling on a narrow canvas → dense rendering via MipMap.
    #[test]
    fn digital_dense_many_edges() {
        let channels = vec![DigitalChannel::new(ChannelId(0), "D0", 0)];
        let mut trace = DigitalTrace::new(channels, Timebase::new(1_000_000.0, 0.0));
        let words: Vec<u64> = (0..50).map(|i| if i % 2 == 0 { 0b0 } else { 0b1 }).collect();
        trace.push_words(&words[..25]);
        trace.push_words(&words[25..]);

        let row = make_digital_row(40.0, 0);
        let mut canvas = PixelCanvas::new(20, 40);

        // edge_count=49, num_pixels=20 → dense mode
        build_digital_signal(&mut canvas, &trace, 0, 0, 50, 50.0, &row, 20.0);

        // Background present.
        assert!(count_pixels(&canvas, BG_COLOR_RGBA) > 0);

        // Signal pixels present.
        let sig = count_pixels(&canvas, DIGITAL_COLOR_RGBA);
        assert!(sig > 0, "no digital signal pixels in dense mode, got {sig}");

        save_canvas_png(&canvas, "digital_dense.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  End-to-end: MipMap pipeline via envelope
    // ═══════════════════════════════════════════════════════════════════════

    /// Verifies the full AnalogTrace → AnalogMipMap → envelope_buckets →
    /// PixelCanvas pipeline with incremental pushes.
    #[test]
    fn analog_mipmap_pipeline_incremental() {
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
        let color = ANALOG_COLORS_RGBA[2];
        let mut canvas = PixelCanvas::new(600, 80);

        build_analog_signal(&mut canvas, &trace, 0, total, total as f64,
                            &row, &color, 600.0);

        // Top grid line safe from signal.
        assert_grid_line_at(&canvas, 0, 300);

        // The mip-map should faithfully preserve amplitude.
        let mut min_y = canvas.height();
        let mut max_y = 0u32;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let p = canvas.pixel(x, y);
                if p != BG_COLOR_RGBA && p != GRID_COLOR_RGBA && p != RgbaColor::TRANSPARENT {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let span = max_y.saturating_sub(min_y);
        assert!(span > 20,
            "mip-map envelope should span >20 px, got {span} (y {min_y}..{max_y})");

        save_canvas_png(&canvas, "analog_mipmap_pipeline.png");
    }

    // ═══════════════════════════════════════════════════════════════════════
    //  Screenshot export — run with `cargo test screenshot -- --nocapture`
    // ═══════════════════════════════════════════════════════════════════════

    /// Save a `PixelCanvas` as a PNG file in `target/test-screenshots/`.
    fn save_canvas_png(canvas: &PixelCanvas, name: &str) {
        let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap()
            .join("target").join("test-screenshots");
        std::fs::create_dir_all(&out_dir).expect("create screenshot dir");
        canvas.save_png(out_dir.join(name)).expect("save png");
    }

}
