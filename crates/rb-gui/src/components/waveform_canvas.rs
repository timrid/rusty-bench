//! Waveform canvas component: renders analog/digital traces via HTML5 `<canvas>`
//! with pan/zoom support and decoder annotation display.

use dioxus::prelude::*;
use rb_core::AcquisitionState;
use rb_model::AnalogTrace;

use crate::waveform_state::WaveformView;

use super::app::AppStateRef;

const ANALOG_ROW_H: f64 = 80.0;
const DIGITAL_ROW_H: f64 = 22.0;
const ANNOTATION_ROW_H: f64 = 16.0;
const LABEL_W: f64 = 36.0;

/// Waveform canvas component for one device.
#[component]
pub fn WaveformCanvas(
    device_id: rb_device::DeviceId,
    data_version: Signal<u64>,
    view: Signal<WaveformView>,
) -> Element {
    let _version = data_version();

    let state: AppStateRef = use_context();

    // Gather acquisition data for view update.
    let (acq_state, digital, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (
                acq.state().clone(),
                acq.digital_trace().cloned(),
                acq.sample_count(),
            )
        } else if let Some(handle) = s.session.device(&device_id) {
            (
                handle.state().clone(),
                handle.digital_trace().cloned(),
                handle.sample_count(),
            )
        } else {
            (AcquisitionState::Idle, None, 0)
        }
    };

    // Update view: clamp window, feed decoder.
    {
        let mut v = view.write();
        let is_running = matches!(acq_state, AcquisitionState::Running);
        if sample_count > 0 {
            v.clamp_view(sample_count, is_running);
        }

        if let Some(dt) = &digital {
            v.feed_decoder(dt);
        }
    }

    // Write view back to app state.
    {
        let mut s = state.borrow_mut();
        s.views.insert(device_id.clone(), view.read().clone());
    }

    // Canvas drawing effect: re-runs when data_version changes.
    let canvas_id = format!("waveform-{}", device_id.as_str().replace(':', "-"));
    let canvas_id_for_effect = canvas_id.clone();
    let device_id_for_effect = device_id.clone();
    let state_for_effect = state.clone();
    use_effect(move || {
        // Subscribe to data_version so effect re-runs on changes.
        data_version();

        // Read current state.
        let s = state_for_effect.borrow();
        let (acq_state, analog, digital, sample_count) =
            if let Some(acq) = s.acquisitions.get(&device_id_for_effect) {
                (
                    acq.state().clone(),
                    acq.analog_traces().to_vec(),
                    acq.digital_trace().cloned(),
                    acq.sample_count(),
                )
            } else if let Some(handle) = s.session.device(&device_id_for_effect) {
                (
                    handle.state().clone(),
                    handle.analog_traces().to_vec(),
                    handle.digital_trace().cloned(),
                    handle.sample_count(),
                )
            } else {
                (AcquisitionState::Idle, Vec::new(), None, 0)
            };
        drop(s);

        let is_running = matches!(acq_state, AcquisitionState::Running);
        let view_state = view.read();
        let draw_js = build_draw_script(
            &canvas_id_for_effect,
            &view_state,
            &analog,
            &digital,
            sample_count,
            is_running,
        );

        log::info!("Canvas eval: id={canvas_id_for_effect}, js_len={}", draw_js.len());
        dioxus::document::eval(&draw_js);
    });

    // Pan state for drag handling.
    let mut drag_active = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0.0f64);

    rsx! {
        div { class: "flex flex-col h-full",
            // Canvas
            div { class: "flex-1 relative",
                canvas {
                    id: "{canvas_id}",
                    class: "absolute inset-0 w-full h-full",
                    width: "100%",
                    height: "100%",
                    onwheel: move |evt| {
                        let dy = match evt.data().delta() {
                            dioxus::html::geometry::WheelDelta::Pixels(v) => v.y,
                            dioxus::html::geometry::WheelDelta::Lines(v) => v.y * 20.0,
                            dioxus::html::geometry::WheelDelta::Pages(v) => v.y * 200.0,
                        };
                        let factor: f64 = if dy < 0.0 { 0.8 } else { 1.25 };
                        view.write().zoom(factor, sample_count);
                    },
                    onmousedown: move |evt| {
                        drag_active.set(true);
                        drag_start_x.set(evt.data().coordinates().client().x);
                    },
                    onmousemove: move |evt| {
                        if drag_active() {
                            let cx = evt.data().coordinates().client().x;
                            let dx = (drag_start_x() - cx) as f32;
                            drag_start_x.set(cx);
                            view.write().pan(dx, 800.0, sample_count);
                        }
                    },
                    onmouseup: move |_| drag_active.set(false),
                    onmouseleave: move |_| drag_active.set(false),
                }
            }
        }
    }
}

// ── Canvas drawing script builder ─────────────────────────────────────────────

/// Color palette for analog channels (CSS colors).
const ANALOG_COLORS: &[&str] = &[
    "#facc15", // yellow
    "#60a5fa", // blue
    "#f87171", // red
    "#34d399", // green
    "#c084fc", // purple
    "#fb923c", // orange
    "#2dd4bf", // teal
    "#f472b6", // pink
];

fn build_draw_script(
    canvas_id: &str,
    view: &WaveformView,
    analog: &[AnalogTrace],
    digital: &Option<rb_model::DigitalTrace>,
    sample_count: usize,
    is_running: bool,
) -> String {
    if sample_count == 0 {
        let msg = if is_running {
            "Waiting for samples..."
        } else {
            "Click \u{25B6} Run to start"
        };
        return format!(
            "var c=document.getElementById('{}');if(c){{c.width=c.clientWidth;c.height=c.clientHeight;var ctx=c.getContext('2d');ctx.fillStyle='#0d1117';ctx.fillRect(0,0,c.width,c.height);ctx.fillStyle='#666';ctx.font='12px monospace';ctx.fillText('{}',10,30);}}",
            canvas_id, msg,
        );
    }

    let range = {
        let start = view.view_start;
        let end = (view.view_start + view.view_samples).min(sample_count);
        start..end
    };

    let mut js = format!(
        "var c=document.getElementById('{}');if(!c)return;c.width=c.clientWidth;c.height=c.clientHeight;var ctx=c.getContext('2d');var w=c.width,h=c.height;var sx=w/800;ctx.fillStyle='#0d1117';ctx.fillRect(0,0,w,h);",
        canvas_id
    );

    let range_len = (range.end - range.start).max(1) as f64;

    // ═══ Scaled drawing: all X coordinates use *sx multiplier ═══════════
    // (no ctx.scale — we multiply X by sx manually to keep line widths consistent)

    // ── Time grid ─────────────────────────────────────────────────────────
    let grid_count = 10;
    for i in 0..=grid_count {
        let gx = (i as f64 / grid_count as f64) * 800.0;
        js.push_str(&format!(
            "ctx.strokeStyle='#1a1a2e';ctx.lineWidth=0.5;ctx.beginPath();ctx.moveTo({gx:.1}*sx,0);ctx.lineTo({gx:.1}*sx,h);ctx.stroke();"
        ));
    }

    let mut y_offset: f64 = 0.0;

    // ── Analog traces (scaled drawing only) ───────────────────────────────
    for (ch_idx, trace) in analog.iter().enumerate() {
        let color = ANALOG_COLORS[ch_idx % ANALOG_COLORS.len()];
        draw_analog_scaled(&mut js, trace, range.clone(), &mut y_offset, range_len, color);
        y_offset += ANALOG_ROW_H + 2.0;
    }

    // ── Digital traces ────────────────────────────────────────────────────
    if let Some(dt) = digital {
        let channels = dt.channels();
        if !channels.is_empty() {
            let mip = dt.transitions();

            for (ch_idx, ch) in channels.iter().enumerate() {
                let row_top = y_offset + ch_idx as f64 * DIGITAL_ROW_H;
                let bit = ch.bit as usize;
                let initial = mip.value_at(bit, range.start as u64);
                let edges: Vec<u64> = mip
                    .edges_in(bit, range.start as u64..range.end as u64)
                    .to_vec();

                let high_y = row_top + 3.0;
                let low_y = row_top + DIGITAL_ROW_H - 4.0;
                let signal_left = LABEL_W;
                let signal_width = 800.0 - signal_left;

                let mut current_y = if initial { high_y } else { low_y };
                let mut prev_x = signal_left;

                js.push_str("ctx.strokeStyle='#58a6ff';ctx.lineWidth=1.5;ctx.beginPath();");

                for &edge_idx in &edges {
                    let edge_x = signal_left
                        + (edge_idx as f64 - range.start as f64) / range_len * signal_width;
                    js.push_str(&format!(
                        "ctx.moveTo({:.1}*sx,{:.1});ctx.lineTo({:.1}*sx,{:.1});",
                        prev_x, current_y, edge_x, current_y
                    ));
                    let next_y = if (current_y - high_y).abs() < 0.5 { low_y } else { high_y };
                    js.push_str(&format!(
                        "ctx.moveTo({:.1}*sx,{:.1});ctx.lineTo({:.1}*sx,{:.1});",
                        edge_x, current_y, edge_x, next_y
                    ));
                    current_y = next_y;
                    prev_x = edge_x;
                }

                let end_x = signal_left
                    + (range.end - range.start) as f64 / range_len * signal_width;
                js.push_str(&format!(
                    "ctx.moveTo({:.1}*sx,{:.1});ctx.lineTo({:.1}*sx,{:.1});",
                    prev_x, current_y, end_x, current_y
                ));
                js.push_str("ctx.stroke();");
            }

            y_offset += channels.len() as f64 * DIGITAL_ROW_H + 4.0;
        }
    }

    // ── Annotations ───────────────────────────────────────────────────────
    if !view.annotations.is_empty() {
        let ann_top = y_offset;
        js.push_str(&format!(
            "ctx.fillStyle='#161b22';ctx.fillRect(0,{:.1},800*sx,{:.1});",
            ann_top, ANNOTATION_ROW_H
        ));

        for ann in &view.annotations {
            if ann.range.end <= range.start || ann.range.start >= range.end {
                continue;
            }
            let x0 = (ann.range.start.max(range.start)).saturating_sub(range.start) as f64
                / range_len * 800.0;
            let x1 = (ann.range.end.min(range.end)).saturating_sub(range.start) as f64
                / range_len * 800.0;
            if x1 - x0 < 1.0 {
                continue;
            }

            let color = match ann.kind {
                rb_decode::AnnotationKind::Data => "'#1f3a6b'",
                rb_decode::AnnotationKind::Address => "'#6b3d1f'",
                rb_decode::AnnotationKind::Frame => "'#2d2d2d'",
                rb_decode::AnnotationKind::Error => "'#6b1f1f'",
            };
            js.push_str(&format!(
                "ctx.fillStyle={};ctx.fillRect({:.1}*sx,{:.1},{:.1}*sx,{:.1});",
                color,
                x0,
                (x1 - x0).max(1.0),
                ann_top + 1.0,
                ANNOTATION_ROW_H - 2.0
            ));

            if x1 - x0 > 24.0 {
                let label = ann.label.replace('\'', "\\'");
                js.push_str(&format!(
                    "ctx.fillStyle='#c9d1d9';ctx.font='9px monospace';ctx.fillText('{}',{:.1}*sx,{:.1});",
                    label, x0 + 2.0, ann_top + 12.0
                ));
            }
        }
    }

    // ═══ Unscaled section: text labels (real pixel coordinates) ══════════
    let mut label_y: f64 = 0.0;

    // Analog channel labels
    for (ch_idx, trace) in analog.iter().enumerate() {
        draw_analog_labels(&mut js, trace, label_y, ch_idx);
        label_y += ANALOG_ROW_H + 2.0;
    }

    // Digital channel labels
    if let Some(dt) = digital {
        for (ch_idx, ch) in dt.channels().iter().enumerate() {
            let row_top = label_y + ch_idx as f64 * DIGITAL_ROW_H;
            let label = ch.name.replace('\'', "\\'");
            js.push_str(&format!(
                "ctx.fillStyle='#8b949e';ctx.font='10px monospace';ctx.fillText('{}',4*sx,{:.1});",
                label,
                row_top + DIGITAL_ROW_H * 0.5 + 4.0
            ));
        }
    }

    js
}

/// Draws the scaled portion of one analog trace row: background, grid, zero-line, fill, center line.
fn draw_analog_scaled(
    js: &mut String,
    trace: &AnalogTrace,
    range: std::ops::Range<usize>,
    y_offset: &mut f64,
    range_len: f64,
    color: &str,
) {
    let pixel_width: usize = 800;
    let buckets = trace.buckets(range.clone(), pixel_width.max(1));
    if buckets.is_empty() {
        return;
    }

    let (raw_lo, raw_hi) = buckets.iter().fold((i32::MAX, i32::MIN), |(lo, hi), b| {
        (lo.min(b.min), hi.max(b.max))
    });
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let top = *y_offset;

    let to_y = |phys: f64| -> f64 {
        top + ANALOG_ROW_H - ((phys - p_lo) / p_span * ANALOG_ROW_H)
    };

    // Row background
    js.push_str(&format!(
        "ctx.fillStyle='#0d1117';ctx.fillRect(0,{top:.1},800*sx,{ANALOG_ROW_H:.1});"
    ));

    // Voltage grid (horizontal lines)
    let grid_steps = 5;
    for i in 0..=grid_steps {
        let frac = i as f64 / grid_steps as f64;
        let gy = top + frac * ANALOG_ROW_H;
        js.push_str(&format!(
            "ctx.strokeStyle='#1a1a2e';ctx.lineWidth=0.5;ctx.beginPath();ctx.moveTo({LABEL_W:.1}*sx,{gy:.1});ctx.lineTo(800*sx,{gy:.1});ctx.stroke();"
        ));
    }

    // Zero line (dashed)
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = to_y(0.0);
        js.push_str(&format!(
            "ctx.strokeStyle='#30363d';ctx.lineWidth=1;ctx.setLineDash([4,4]);ctx.beginPath();ctx.moveTo(0,{zy:.1});ctx.lineTo(800*sx,{zy:.1});ctx.stroke();ctx.setLineDash([]);"
        ));
    }

    // Filled area (min → max polygon)
    js.push_str(&format!("ctx.fillStyle='{color}1a';ctx.beginPath();"));

    let first_x = (buckets[0].start as f64 - range.start as f64) / range_len * 800.0;
    js.push_str(&format!("ctx.moveTo({first_x:.1}*sx,{:.1});", to_y(trace.to_physical(buckets[0].max))));
    for b in &buckets[1..] {
        let x = (b.start as f64 - range.start as f64) / range_len * 800.0;
        js.push_str(&format!("ctx.lineTo({x:.1}*sx,{:.1});", to_y(trace.to_physical(b.max))));
    }
    for b in buckets.iter().rev() {
        let x = (b.start as f64 - range.start as f64) / range_len * 800.0;
        js.push_str(&format!("ctx.lineTo({x:.1}*sx,{:.1});", to_y(trace.to_physical(b.min))));
    }
    js.push_str("ctx.closePath();ctx.fill();");

    // Center line
    js.push_str(&format!("ctx.strokeStyle='{color}';ctx.lineWidth=1.5;ctx.beginPath();"));
    let first_avg = (trace.to_physical(buckets[0].min) + trace.to_physical(buckets[0].max)) / 2.0;
    js.push_str(&format!("ctx.moveTo({first_x:.1}*sx,{:.1});", to_y(first_avg)));
    for b in &buckets[1..] {
        let x = (b.start as f64 - range.start as f64) / range_len * 800.0;
        let avg = (trace.to_physical(b.min) + trace.to_physical(b.max)) / 2.0;
        js.push_str(&format!("ctx.lineTo({x:.1}*sx,{:.1});", to_y(avg)));
    }
    js.push_str("ctx.stroke();");
}

/// Draws text labels for one analog trace row (unscaled — uses real pixel coords via `sx`).
fn draw_analog_labels(
    js: &mut String,
    trace: &AnalogTrace,
    y_offset: f64,
    _ch_idx: usize,
) {
    // Voltage labels for this row
    let buckets = trace.buckets(0..trace.len(), 800.max(1));
    if buckets.is_empty() {
        return;
    }
    let (raw_lo, raw_hi) = buckets.iter().fold((i32::MAX, i32::MIN), |(lo, hi), b| {
        (lo.min(b.min), hi.max(b.max))
    });
    let phys_lo = trace.to_physical(raw_lo);
    let phys_hi = trace.to_physical(raw_hi);
    let margin = (phys_hi - phys_lo).abs() * 0.1 + 1e-12;
    let p_lo = phys_lo - margin;
    let p_hi = phys_hi + margin;
    let p_span = (p_hi - p_lo).max(1e-12);

    let top = y_offset;
    let grid_steps = 5;
    for i in 0..=grid_steps {
        let frac = i as f64 / grid_steps as f64;
        let gy = top + frac * ANALOG_ROW_H;
        let volt = p_lo + frac * p_span;
        let v_label = format_voltage(volt);
        js.push_str(&format!(
            "ctx.fillStyle='#484f58';ctx.font='9px monospace';ctx.fillText('{v_label}',2*sx,{:.1});",
            gy - 2.0
        ));
    }

    // Channel name label
    let unit = trace.channel().unit.as_deref().unwrap_or("");
    let label = if unit.is_empty() {
        trace.channel().name.clone()
    } else {
        format!("{} [{}]", trace.channel().name, unit)
    };
    let label_safe = label.replace('\'', "\\'");
    js.push_str(&format!(
        "ctx.fillStyle='#c9d1d9';ctx.font='bold 11px monospace';ctx.fillText('{}',{:.1}*sx,{:.1});",
        label_safe,
        LABEL_W + 4.0,
        top + 14.0
    ));
}

/// Format a voltage value for display.
fn format_voltage(v: f64) -> String {
    let abs_v = v.abs();
    if abs_v >= 1.0 {
        format!("{:.1}V", v)
    } else if abs_v >= 1e-3 {
        format!("{:.1}mV", v * 1e3)
    } else if abs_v >= 1e-6 {
        format!("{:.1}µV", v * 1e6)
    } else if abs_v == 0.0 {
        "0V".to_string()
    } else {
        format!("{:.1}nV", v * 1e9)
    }
}
