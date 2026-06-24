//! Waveform canvas component: renders analog/digital traces via HTML5 `<canvas>`
//! with pan/zoom support and decoder annotation display.

use dioxus::prelude::*;
use rb_core::AcquisitionState;
use rb_model::AnalogTrace;

use crate::waveform_state::WaveformView;

use super::app::AppStateRef;
use super::decoder_config::DecoderConfig;

const ANALOG_ROW_H: f64 = 80.0;
const DIGITAL_ROW_H: f64 = 22.0;
const ANNOTATION_ROW_H: f64 = 16.0;
const LABEL_W: f64 = 36.0;

/// Waveform canvas component for one device.
#[component]
pub fn WaveformCanvas(
    device_id: rb_device::DeviceId,
    data_version: Signal<u64>,
) -> Element {
    let _version = data_version();

    let state: AppStateRef = use_context();

    // Per-device view state.
    let view = {
        let s = state.borrow();
        s.views.get(&device_id).cloned().unwrap_or_default()
    };
    let mut view_signal = use_signal(move || view);

    // Gather acquisition data.
    let (acq_state, analog, digital, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (
                acq.state().clone(),
                acq.analog_traces().to_vec(),
                acq.digital_trace().cloned(),
                acq.sample_count(),
            )
        } else if let Some(handle) = s.session.device(&device_id) {
            (
                handle.state().clone(),
                handle.analog_traces().to_vec(),
                handle.digital_trace().cloned(),
                handle.sample_count(),
            )
        } else {
            (AcquisitionState::Idle, Vec::new(), None, 0)
        }
    };

    // Update view: clamp window, feed decoder.
    {
        let mut v = view_signal.write();
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
        s.views.insert(device_id.clone(), view_signal.read().clone());
    }

    // Build canvas drawing JavaScript.
    let canvas_id = format!("waveform-{}", device_id.as_str().replace(':', "-"));
    let draw_js = build_draw_script(
        &canvas_id,
        &view_signal.read(),
        &analog,
        &digital,
        sample_count,
    );

    // Execute drawing after render.
    use_effect(move || {
        let _ = dioxus::document::eval(&draw_js);
    });

    // Pan state for drag handling.
    let mut drag_active = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0.0f64);

    // Unicode symbols
    let bullet = "\u{25CF}";
    let circle = "\u{25CB}";
    let warn = "\u{26A0}";

    rsx! {
        div { class: "flex flex-col h-full",
            // Status bar
            div { class: "flex items-center gap-2 px-2 py-1 border-b border-zinc-800 text-xs",
                match &acq_state {
                    AcquisitionState::Running => rsx! {
                        span { class: "text-green-400", "{bullet}" }
                        span { class: "text-green-400", "Running" }
                    },
                    AcquisitionState::Idle => rsx! {
                        span { class: "text-zinc-500", "{circle}" }
                        span { class: "text-zinc-500", "Idle" }
                    },
                    AcquisitionState::Stopped => rsx! {
                        span { class: "text-zinc-500", "{circle}" }
                        span { class: "text-zinc-500", "Stopped" }
                    },
                    AcquisitionState::Error(msg) => rsx! {
                        span { class: "text-red-400", "{warn}" }
                        span { class: "text-red-400", "Error: {msg}" }
                    },
                }
                span { class: "text-zinc-600 mx-1", "|" }
                span { class: "text-zinc-400", "{sample_count} samples" }
                span { class: "text-zinc-600 mx-1", "|" }

                label { class: "flex items-center gap-1 cursor-pointer text-zinc-400 hover:text-zinc-200",
                    input {
                        r#type: "checkbox",
                        class: "accent-blue-600",
                        checked: view_signal.read().auto_scroll,
                        onchange: move |evt| view_signal.write().auto_scroll = evt.checked(),
                    }
                    "Follow"
                }

                div { class: "flex-1" }

                DecoderConfig { view: view_signal }
            }

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
                        view_signal.write().zoom(factor, sample_count);
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
                            view_signal.write().pan(dx, 800.0, sample_count);
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

fn build_draw_script(
    canvas_id: &str,
    view: &WaveformView,
    analog: &[AnalogTrace],
    digital: &Option<rb_model::DigitalTrace>,
    sample_count: usize,
) -> String {
    if sample_count == 0 {
        return format!(
            "var c=document.getElementById('{}');if(c){{c.width=c.clientWidth;c.height=c.clientHeight;var ctx=c.getContext('2d');ctx.fillStyle='#12121c';ctx.fillRect(0,0,c.width,c.height);ctx.fillStyle='#666';ctx.font='12px monospace';ctx.fillText('No samples yet.',10,30);}}",
            canvas_id
        );
    }

    let range = {
        let start = view.view_start;
        let end = (view.view_start + view.view_samples).min(sample_count);
        start..end
    };

    let mut js = format!(
        "var c=document.getElementById('{}');if(!c)return;c.width=c.clientWidth;c.height=c.clientHeight;var ctx=c.getContext('2d');var w=c.width,h=c.height;ctx.fillStyle='#12121c';ctx.fillRect(0,0,w,h);",
        canvas_id
    );

    let range_len = (range.end - range.start).max(1) as f64;

    let mut y_offset: f64 = 0.0;

    // analog traces
    for trace in analog {
        draw_analog_js(&mut js, trace, range.clone(), &mut y_offset, range_len);
        y_offset += ANALOG_ROW_H + 2.0;
    }

    // digital traces
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

                let label = ch.name.replace('\'', "\\'");
                js.push_str(&format!(
                    "ctx.fillStyle='#b4b4b8';ctx.font='10px monospace';ctx.fillText('{}',4,{:.1});",
                    label,
                    row_top + DIGITAL_ROW_H * 0.5 + 4.0
                ));

                let high_y = row_top + 3.0;
                let low_y = row_top + DIGITAL_ROW_H - 4.0;
                let signal_left = LABEL_W;
                let signal_width = 800.0 - signal_left;

                let mut current_y = if initial { high_y } else { low_y };
                let mut prev_x = signal_left;

                js.push_str("ctx.strokeStyle='#64c8ff';ctx.lineWidth=1.5;ctx.beginPath();");

                for &edge_idx in &edges {
                    let edge_x = signal_left
                        + (edge_idx as f64 - range.start as f64) / range_len * signal_width;
                    js.push_str(&format!(
                        "ctx.moveTo({:.1},{:.1});ctx.lineTo({:.1},{:.1});",
                        prev_x, current_y, edge_x, current_y
                    ));
                    let next_y = if (current_y - high_y).abs() < 0.5 { low_y } else { high_y };
                    js.push_str(&format!(
                        "ctx.moveTo({:.1},{:.1});ctx.lineTo({:.1},{:.1});",
                        edge_x, current_y, edge_x, next_y
                    ));
                    current_y = next_y;
                    prev_x = edge_x;
                }

                let end_x = signal_left
                    + (range.end - range.start) as f64 / range_len * signal_width;
                js.push_str(&format!(
                    "ctx.moveTo({:.1},{:.1});ctx.lineTo({:.1},{:.1});",
                    prev_x, current_y, end_x, current_y
                ));
                js.push_str("ctx.stroke();");
            }

            y_offset += channels.len() as f64 * DIGITAL_ROW_H + 4.0;
        }
    }

    // annotations
    if !view.annotations.is_empty() {
        let ann_top = y_offset;
        js.push_str(&format!(
            "ctx.fillStyle='#121223';ctx.fillRect(0,{:.1},w,{:.1});",
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
                rb_decode::AnnotationKind::Data => "'#2850a0'",
                rb_decode::AnnotationKind::Address => "'#8c5014'",
                rb_decode::AnnotationKind::Frame => "'#3c3c3c'",
                rb_decode::AnnotationKind::Error => "'#a01e1e'",
            };
            js.push_str(&format!(
                "ctx.fillStyle={};var ax={};var aw={};ctx.fillRect(ax,{:.1},aw,{:.1});",
                color,
                x0,
                (x1 - x0).max(1.0),
                ann_top + 1.0,
                ANNOTATION_ROW_H - 2.0
            ));

            if x1 - x0 > 24.0 {
                let label = ann.label.replace('\'', "\\'");
                js.push_str(&format!(
                    "ctx.fillStyle='#fff';ctx.font='9px monospace';ctx.fillText('{}',{:.1},{:.1});",
                    label, x0 + 2.0, ann_top + 12.0
                ));
            }
        }
    }

    js.push('}');
    js
}

/// Generates JS for one analog trace row via Canvas 2D API.
fn draw_analog_js(
    js: &mut String,
    trace: &AnalogTrace,
    range: std::ops::Range<usize>,
    y_offset: &mut f64,
    range_len: f64,
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

    // Zero line
    if p_lo < 0.0 && p_hi > 0.0 {
        let zy = top + ANALOG_ROW_H - ((0.0 - p_lo) / p_span * ANALOG_ROW_H);
        js.push_str(&format!(
            "ctx.strokeStyle='#323232';ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(0,{:.1});ctx.lineTo(w,{:.1});ctx.stroke();",
            zy, zy
        ));
    }

    // Bucket bars
    js.push_str("ctx.strokeStyle='#3cd278';ctx.lineWidth=1;ctx.beginPath();");
    for b in &buckets {
        let x = (b.start as f64 - range.start as f64) / range_len * 800.0;
        let y_top_val =
            top + ANALOG_ROW_H - ((trace.to_physical(b.max) - p_lo) / p_span * ANALOG_ROW_H);
        let y_bot_val =
            top + ANALOG_ROW_H - ((trace.to_physical(b.min) - p_lo) / p_span * ANALOG_ROW_H);
        let (y0, y1) = if (y_bot_val - y_top_val).abs() < 1.0 {
            (y_bot_val - 0.5, y_bot_val + 0.5)
        } else {
            (y_top_val, y_bot_val)
        };
        js.push_str(&format!(
            "ctx.moveTo({:.1},{:.1});ctx.lineTo({:.1},{:.1});",
            x, y0, x, y1
        ));
    }
    js.push_str("ctx.stroke();");

    // Channel label
    let unit = trace.channel().unit.as_deref().unwrap_or("");
    let label = if unit.is_empty() {
        trace.channel().name.clone()
    } else {
        format!("{} [{}]", trace.channel().name, unit)
    };
    let label_safe = label.replace('\'', "\\'");
    js.push_str(&format!(
        "ctx.fillStyle='#c8c8c8';ctx.font='11px monospace';ctx.fillText('{}',4,{:.1});",
        label_safe,
        top + 15.0
    ));
}
