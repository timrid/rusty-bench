//! Waveform display: multi-canvas architecture with HTML overlays.
//!
//! Each signal row has its own `<canvas>` (drawing only the waveform).
//! Labels, markers, cursor line, and time ruler are HTML/CSS elements.
//! A single `use_effect` draws all canvases on each `data_version` change.

use dioxus::prelude::*;
use rb_core::AcquisitionState;
use rb_decode::AnnotationKind;
use rb_model::AnalogTrace;

use crate::waveform_state::{
    RowKind, TimeMarker, WaveformView, DIVIDER_H, LABEL_W,
    MARKER_BAR_H, MEASUREMENT_ZONE_H, TIME_RULER_H,
};

use super::app::AppStateRef;

// ── Colors ───────────────────────────────────────────────────────────────────

const ANALOG_COLORS: &[&str] = &[
    "#facc15", "#60a5fa", "#f87171", "#34d399",
    "#c084fc", "#fb923c", "#2dd4bf", "#f472b6",
];

const DIGITAL_COLOR: &str = "#58a6ff";
const BG_COLOR: &str = "#0d1117";
const GRID_COLOR: &str = "#1a1a2e";
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

    // ── Signal rows ───────────────────────────────────────────────────────
    for (row_idx, row) in rows.iter().enumerate() {
        if !row.visible { continue; }
        let cid = format!("sig-{short_id}-{row_idx}");
        let row_js = match row.kind {
            RowKind::Analog => {
                if let Some(trace) = analog.get(row.channel_index) {
                    let color = ANALOG_COLORS[row.channel_index % ANALOG_COLORS.len()];
                    build_analog_signal_js(&cid, trace, range_start, range_end, range_len, row, color)
                } else { String::new() }
            }
            RowKind::Digital => {
                if let Some(dt) = digital {
                    if let Some(ch) = dt.channels().get(row.channel_index) {
                        build_digital_signal_js(&cid, dt, ch.bit as usize, range_start, range_end, range_len, row, signal_width)
                    } else { String::new() }
                } else { String::new() }
            }
            RowKind::Decoder => {
                build_decoder_signal_js(&cid, annotations, range_start, range_end, range_len, row)
            }
        };
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
    // Replace `var c=` with `let c=` so each canvas block has its own scope.
    // `var` is function-scoped and would cause all blocks to share one `c`.
    let js = js.replace("var c=", "let c=");
    let inner = &js[1..js.len()-1];

    // Find the "if(!c)" guard and split after it.
    // Handles both old-style `if(!c)return;` and new-style `if(!c){...;return;}`.
    let guard_start = inner.find("if(!c)").unwrap_or(0);
    let after_guard = if inner[guard_start..].starts_with("if(!c){") {
        // New style with brace block: find the closing `}`
        if let Some(brace_pos) = inner[guard_start..].find('}') {
            guard_start + brace_pos + 1
        } else {
            guard_start
        }
    } else {
        // Old style without braces: `if(!c)return;`
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

fn build_analog_signal_js(
    canvas_id: &str, trace: &AnalogTrace,
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor, color: &str,
) -> String {
    let sig_h = row.signal_height_px;
    let range = range_start..range_end;
    let buckets = trace.buckets(range.clone(), 1200usize.max(1));
    if buckets.is_empty() {
        log::warn!("Analog canvas {canvas_id}: no buckets for range {range_start}..{range_end}, trace len={}", trace.len());
        return String::new();
    }
    let (raw_lo, raw_hi) = buckets.iter().fold((i32::MAX, i32::MIN), |(lo, hi), b| (lo.min(b.min), hi.max(b.max)));
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let mut js = format!(
        "{{var c=document.getElementById('{canvas_id}');if(!c){{console.warn('Canvas not found: {canvas_id}');return;}}var dpr=window.devicePixelRatio||1;var w=c.clientWidth;var h={sig_h};c.width=w*dpr;c.height=h*dpr;var ctx=c.getContext('2d');ctx.scale(dpr,dpr);ctx.fillStyle='{BG_COLOR}';ctx.fillRect(0,0,w,h);"
    );
    // Grid
    js.push_str(&format!("ctx.strokeStyle='{GRID_COLOR}';ctx.lineWidth=0.5;"));
    for i in 0..=5 { let gy = i as f64 / 5.0 * sig_h; js.push_str(&format!("ctx.beginPath();ctx.moveTo(0,{gy:.1});ctx.lineTo(w,{gy:.1});ctx.stroke();")); }
    // Zero line
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = sig_h - ((0.0 - p_lo) / p_span * sig_h);
        js.push_str(&format!("ctx.strokeStyle='#30363d';ctx.lineWidth=1;ctx.setLineDash([4,4]);ctx.beginPath();ctx.moveTo(0,{zy:.1});ctx.lineTo(w,{zy:.1});ctx.stroke();ctx.setLineDash([]);"));
    }
    // Data arrays
    let mut xv = String::from("[");
    let mut mxv = String::from("[");
    let mut mnv = String::from("[");
    for b in &buckets {
        xv.push_str(&format!("{},", b.start));
        mxv.push_str(&format!("{:.1},", trace.to_physical(b.max).clamp(p_lo, p_hi)));
        mnv.push_str(&format!("{:.1},", trace.to_physical(b.min).clamp(p_lo, p_hi)));
    }
    xv.push(']'); mxv.push(']'); mnv.push(']');
    // JS: scale sample index to pixel
    js.push_str(&format!("var xv={xv},mv={mxv},lv={mnv},n=xv.length;"));
    js.push_str(&format!("function ty(v){{return {sig_h}-((v-({p_lo}))/{p_span}*{sig_h});}}"));
    js.push_str(&format!("function xs(s){{return((s-{range_start})/{range_len})*w;}}"));
    // Fill
    js.push_str(&format!("ctx.fillStyle='{color}1a';ctx.beginPath();ctx.moveTo(xs(xv[0]),ty(mv[0]));"));
    js.push_str("for(var i=1;i<n;i++)ctx.lineTo(xs(xv[i]),ty(mv[i]));for(var i=n-1;i>=0;i--)ctx.lineTo(xs(xv[i]),ty(lv[i]));ctx.closePath();ctx.fill();");
    // Center line
    js.push_str(&format!("ctx.strokeStyle='{color}';ctx.lineWidth=1.5;ctx.beginPath();ctx.moveTo(xs(xv[0]),ty((mv[0]+lv[0])/2));"));
    js.push_str("for(var i=1;i<n;i++)ctx.lineTo(xs(xv[i]),ty((mv[i]+lv[i])/2));ctx.stroke();");
    js.push('}');
    js
}

fn build_digital_signal_js(
    canvas_id: &str, dt: &rb_model::DigitalTrace, bit: usize,
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor,
    signal_width: f64,
) -> String {
    let sig_h = row.signal_height_px;
    let mip = dt.transitions();
    let rs_u64 = range_start as u64;
    let initial = mip.value_at(bit, rs_u64);
    let edges: Vec<u64> = mip.edges_in(bit, rs_u64..range_end as u64).to_vec();
    // Integer pixel rows for fillRect (no half-pixel offsets needed).
    let high_row = (sig_h * 0.25).round();
    let low_row = (sig_h * 0.75).round();
    let mid_y = (sig_h * 0.5).round();
    let full_h = low_row - high_row + 1.0; // full-height block when both levels seen

    let num_pixels = signal_width.ceil() as usize;
    let dense = edges.len() > num_pixels;

    let mut js = format!(
        "{{var c=document.getElementById('{canvas_id}');if(!c){{console.warn('Canvas not found: {canvas_id}');return;}}var dpr=window.devicePixelRatio||1;var w=c.clientWidth;var h={sig_h};c.width=w*dpr;c.height=h*dpr;var ctx=c.getContext('2d');ctx.scale(dpr,dpr);ctx.imageSmoothingEnabled=false;ctx.fillStyle='{BG_COLOR}';ctx.fillRect(0,0,w,h);"
    );
    js.push_str(&format!(
        "ctx.strokeStyle='{GRID_COLOR}';ctx.lineWidth=0.5;ctx.beginPath();ctx.moveTo(0,{mid_y:.1});ctx.lineTo(w,{mid_y:.1});ctx.stroke();"
    ));
    js.push_str(&format!("ctx.fillStyle='{DIGITAL_COLOR}';"));

    if dense {
        // ── Dense mode: bucket edges per pixel column ─────────────────
        // When more edges than pixels, aggregate into min/max per column.
        // Columns with ≥1 transition draw a full-height block; quiet
        // columns use value_at() for the correct single-row level.
        let mut col_has_edge = vec![false; num_pixels];
        for &e in &edges {
            let px = (((e as f64 - range_start as f64) / range_len) * signal_width)
                .round() as usize;
            if px < num_pixels {
                col_has_edge[px] = true;
            }
        }
        // Pre-compute correct level per column via value_at (O(log n) each).
        let samples_per_col = range_len / num_pixels as f64;
        let mut col_level = vec![false; num_pixels];
        for col in 0..num_pixels {
            let s = rs_u64 + ((col as f64 + 0.5) * samples_per_col) as u64;
            col_level[col] = mip.value_at(bit, s);
        }
        // Encode as compact '0'/'1' strings.
        let edge_str: String = col_has_edge.iter()
            .map(|&b| if b { '1' } else { '0' }).collect();
        let lvl_str: String = col_level.iter()
            .map(|&b| if b { '1' } else { '0' }).collect();
        js.push_str(&format!(
            "var E='{edge_str}',L='{lvl_str}',hH={high_row:.0},lL={low_row:.0},fH={full_h:.0};\
             for(var x=0;x<E.length;x++){{\
               if(E[x]=='1'){{ctx.fillRect(x,hH,1,fH);}}\
               else{{ctx.fillRect(x,L[x]=='1'?hH:lL,1,1);}}\
             }}"
        ));
    } else {
        // ── Sparse mode: draw individual edges ────────────────────────
        js.push_str(&format!(
            "function xp(s){{return Math.round(((s-{range_start})/{range_len})*w);}}"
        ));
        if !edges.is_empty() || initial {
            let mut cur: f64 = if initial { high_row } else { low_row };
            let mut px = rs_u64;
            for &ei in &edges {
                js.push_str(&format!(
                    "ctx.fillRect(xp({px}),{cur:.0},xp({ei})-xp({px}),1);"
                ));
                let next: f64 = if (cur - high_row).abs() < 0.5 { low_row } else { high_row };
                let top = cur.min(next);
                let h = (next - cur).abs() + 1.0;
                js.push_str(&format!(
                    "ctx.fillRect(xp({ei}),{top:.0},1,{h:.0});"
                ));
                cur = next;
                px = ei;
            }
            js.push_str(&format!(
                "ctx.fillRect(xp({px}),{cur:.0},xp({range_end})-xp({px}),1);"
            ));
        }
    }
    js.push('}');
    js
}

fn build_decoder_signal_js(
    canvas_id: &str, annotations: &[rb_decode::Annotation],
    range_start: usize, range_end: usize, range_len: f64,
    row: &crate::waveform_state::RowDescriptor,
) -> String {
    let sig_h = row.signal_height_px;
    let mut js = format!(
        "{{var c=document.getElementById('{canvas_id}');if(!c){{console.warn('Canvas not found: {canvas_id}');return;}}var dpr=window.devicePixelRatio||1;var w=c.clientWidth;var h={sig_h};c.width=w*dpr;c.height=h*dpr;var ctx=c.getContext('2d');ctx.scale(dpr,dpr);ctx.fillStyle='#161b22';ctx.fillRect(0,0,w,h);"
    );
    for ann in annotations {
        if ann.range.end <= range_start || ann.range.start >= range_end { continue; }
        let rs = range_start;
        let x0 = ann.range.start.saturating_sub(rs) as f64 / range_len;
        let x1 = ann.range.end.min(range_end).saturating_sub(rs) as f64 / range_len;
        let ww = (x1 - x0).max(0.001);
        let color = match ann.kind {
            AnnotationKind::Data => "#1f3a6b",
            AnnotationKind::Address => "#6b3d1f",
            AnnotationKind::Frame => "#2d2d2d",
            AnnotationKind::Error => "#6b1f1f",
        };
        js.push_str(&format!("ctx.fillStyle='{color}';ctx.fillRect({x0}*w,1,{ww}*w,{:.1});", sig_h - 2.0));
        if ww > 0.03 {
            let label = ann.label.replace('\'', "\\'").replace('\\', "\\\\");
            js.push_str(&format!("ctx.fillStyle='#c9d1d9';ctx.font='9px monospace';ctx.fillText('{label}',{x0}*w+2,{:.1});", sig_h * 0.5 + 3.0));
        }
    }
    js.push('}');
    js
}
