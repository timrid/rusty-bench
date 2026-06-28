//! Signal list panel: channel names, colors, visibility toggles (right-click),
//! and sample rate input. Lives left of the waveform canvas.

use dioxus::prelude::*;
use crate::waveform_state::{RowKind, WaveformView};

use super::app::AppStateRef;

/// Left panel showing channel labels and sample rate configuration.
#[component]
pub fn SignalList(
    session_id: crate::state::SessionId,
    mut view: Signal<WaveformView>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (analog_names, digital_names, sample_count, analog_enabled, digital_enabled, sample_rate_hz) = {
        let s = state.borrow();
        if let Some(acq) = s.acq_for_session(session_id) {
            let anames: Vec<String> = acq.analog_traces().iter().map(|t| t.channel().name.clone()).collect();
            let dnames: Vec<String> = acq.digital_trace()
                .map(|dt| dt.channels().iter().map(|ch| ch.name.clone()).collect())
                .unwrap_or_default();
            let aen: Vec<bool> = acq.config.analog_enabled.clone();
            let den: Vec<bool> = acq.config.digital_enabled.clone();
            (anames, dnames, acq.sample_count(), aen, den, acq.config.sample_rate_hz)
        } else if let Some(handle) = s.handle_for_session(session_id) {
            let anames: Vec<String> = handle.analog_traces().iter().map(|t| t.channel().name.clone()).collect();
            let dnames: Vec<String> = handle.digital_trace()
                .map(|dt| dt.channels().iter().map(|ch| ch.name.clone()).collect())
                .unwrap_or_default();
            let aen = vec![true; anames.len()];
            let den = vec![true; dnames.len()];
            (anames, dnames, handle.sample_count(), aen, den, 1_000_000.0)
        } else {
            (Vec::new(), Vec::new(), 0, Vec::new(), Vec::new(), 1_000_000.0)
        }
    };

    let has_analog = !analog_names.is_empty();
    let has_digital = !digital_names.is_empty();

    rsx! {
        div { class: "w-36 flex-shrink-0 border-r border-zinc-800 bg-zinc-900/50 flex flex-col h-full overflow-y-auto select-none",

            // Sample rate
            div { class: "px-2 pt-2 pb-0.5",
                span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider", "Rate" }
            }
            div { class: "px-2 pb-1.5",
                input {
                    r#type: "text",
                    class: "w-full px-1.5 py-0.5 bg-zinc-800 border border-zinc-700 rounded text-xs text-zinc-300 font-mono",
                    value: "{fmt_rate(sample_rate_hz)}",
                    oninput: {
                        let state = state.clone();
                        let sid = session_id;
                        move |evt| {
                            if let Some(hz) = parse_rate(&evt.value()) {
                                let mut s = state.borrow_mut();
                                if let Some(acq) = s.acq_for_session_mut(sid) {
                                    acq.config.sample_rate_hz = hz;
                                }
                            }
                        }
                    },
                }
            }

            div { class: "border-t border-zinc-800 mx-2" }

            // Analog channel section
            if has_analog {
                div { class: "px-2 pt-2 pb-0.5",
                    span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider", "Analog" }
                }
                for (i, name) in analog_names.iter().enumerate() {
                    {
                        let is_enabled = analog_enabled.get(i).copied().unwrap_or(true);
                        let mut view = view;
                        let state = state.clone();
                        let sid = session_id;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_enabled {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-600 cursor-pointer hover:bg-zinc-800/30 rounded mx-1 line-through"
                                },
                                oncontextmenu: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    // Toggle display visibility.
                                    let mut v = view.write();
                                    if let Some(row) = v.rows.iter_mut()
                                        .find(|r| matches!(r.kind, RowKind::Analog) && r.channel_index == i)
                                    {
                                        row.visible = !row.visible;
                                    }
                                    // Toggle acquisition config.
                                    let mut s = state.borrow_mut();
                                    if let Some(acq) = s.acq_for_session_mut(sid) {
                                        if let Some(en) = acq.config.analog_enabled.get_mut(i) {
                                            *en = !*en;
                                        }
                                    }
                                },
                                span {
                                    class: "w-2.5 h-2.5 rounded-full flex-shrink-0",
                                    style: "background-color: {channel_color(i, true)}"
                                }
                                span { class: "truncate flex-1", "{name}" }
                            }
                        }
                    }
                }
            }

            // Digital channel section
            if has_digital {
                if has_analog {
                    div { class: "border-t border-zinc-800 mx-2 my-1" }
                }
                div { class: "px-2 pt-1 pb-0.5",
                    span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider", "Digital" }
                }
                for (i, name) in digital_names.iter().enumerate() {
                    {
                        let is_enabled = digital_enabled.get(i).copied().unwrap_or(true);
                        let mut view = view;
                        let state = state.clone();
                        let sid = session_id;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_enabled {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-600 cursor-pointer hover:bg-zinc-800/30 rounded mx-1 line-through"
                                },
                                oncontextmenu: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    // Toggle display visibility.
                                    let mut v = view.write();
                                    if let Some(row) = v.rows.iter_mut()
                                        .find(|r| matches!(r.kind, RowKind::Digital) && r.channel_index == i)
                                    {
                                        row.visible = !row.visible;
                                    }
                                    // Toggle acquisition config.
                                    let mut s = state.borrow_mut();
                                    if let Some(acq) = s.acq_for_session_mut(sid) {
                                        if let Some(en) = acq.config.digital_enabled.get_mut(i) {
                                            *en = !*en;
                                        }
                                    }
                                },
                                span {
                                    class: "w-2.5 h-2.5 rounded flex-shrink-0",
                                    style: "background-color: {digital_channel_color(i)}"
                                }
                                span { class: "truncate flex-1", "{name}" }
                            }
                        }
                    }
                }
            }

            // Bottom spacer
            div { class: "flex-1" }

            if sample_count > 0 {
                div { class: "border-t border-zinc-800 px-2 py-1.5",
                    p { class: "text-[10px] text-zinc-600", "{sample_count} samples" }
                }
            }
        }
    }
}

fn channel_color(index: usize, _analog: bool) -> &'static str {
    const COLORS: &[&str] = &[
        "#facc15", "#60a5fa", "#f87171", "#34d399",
        "#c084fc", "#fb923c", "#2dd4bf", "#f472b6",
    ];
    COLORS[index % COLORS.len()]
}

fn digital_channel_color(index: usize) -> &'static str {
    const COLORS: &[&str] = &[
        "#34d399", "#2dd4bf", "#22d3ee", "#38bdf8",
        "#818cf8", "#a78bfa", "#f472b6", "#fb923c",
    ];
    COLORS[index % COLORS.len()]
}

// ── Sample-rate helpers ──────────────────────────────────────────────────────

fn fmt_rate(hz: f64) -> String {
    if hz >= 1_000_000.0 {
        format!("{:.3} MHz", hz / 1_000_000.0)
    } else if hz >= 1_000.0 {
        format!("{:.1} kHz", hz / 1_000.0)
    } else {
        format!("{:.0} Hz", hz)
    }
}

fn parse_rate(input: &str) -> Option<f64> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    let (num, mul): (&str, f64) = if let Some(n) = s.strip_suffix("mhz") {
        (n.trim(), 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("khz") {
        (n.trim(), 1_000.0)
    } else if let Some(n) = s.strip_suffix("hz") {
        (n.trim(), 1.0)
    } else {
        (&s[..], 1.0)
    };
    num.parse::<f64>().ok().map(|v| v * mul).filter(|&v| v > 0.0)
}
