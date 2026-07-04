//! Channel-config panel: shows device channels with enable toggles and
//! sample-rate input. Lives left of the waveform canvas.
//!
//! Receives its data as props (Signals) — no global state lookup.

use dioxus::prelude::*;

use crate::logic_analyzer::acquisition::AcquisitionConfig;
use crate::logic_analyzer::view::{RowKind, WaveformView};

/// Left panel showing channel labels, enable toggles, and sample rate.
#[component]
pub fn ChannelConfig(
    mut config: Signal<AcquisitionConfig>,
    mut view: Signal<WaveformView>,
    sample_count: Signal<u64>,
    on_sample_rate_change: Callback<f64>,
) -> Element {
    let cfg = config.read();
    let analog_channels = cfg.analog_channels.clone();
    let digital_channels = cfg.digital_channels.clone();
    let analog_enabled = cfg.analog_enabled.clone();
    let digital_enabled = cfg.digital_enabled.clone();
    let sample_rate_hz = cfg.sample_rate_hz;
    let supported = cfg.supported_sample_rates.clone();
    let sc = sample_count();
    drop(cfg);

    let has_analog = !analog_channels.is_empty();
    let has_digital = !digital_channels.is_empty();

    rsx! {
        div { class: "w-36 flex-shrink-0 border-r border-gray-200 bg-gray-50/50 dark:border-zinc-800 dark:bg-zinc-900/50 flex flex-col h-full overflow-y-auto select-none",

            // Sample rate
            div { class: "px-2 pt-2 pb-0.5",
                span { class: "text-[9px] font-bold text-gray-400 dark:text-zinc-500 uppercase tracking-wider", "Rate" }
            }
            div { class: "px-2 pb-1.5",
                if supported.is_empty() {
                    input {
                        r#type: "text",
                        class: "w-full px-1.5 py-0.5 bg-gray-100 border border-gray-300 dark:bg-zinc-800 dark:border-zinc-700 rounded text-xs text-gray-700 dark:text-zinc-300 font-mono",
                        value: "{fmt_rate(sample_rate_hz)}",
                        oninput: {
                            let mut config = config;
                            let on_sample_rate_change = on_sample_rate_change;
                            move |evt| {
                                if let Some(hz) = parse_rate(&evt.value()) {
                                    config.write().sample_rate_hz = hz;
                                    on_sample_rate_change.call(hz);
                                }
                            }
                        },
                    }
                } else {
                    select {
                        class: "w-full px-1 py-0.5 bg-gray-100 border border-gray-300 dark:bg-zinc-800 dark:border-zinc-700 rounded text-xs text-gray-700 dark:text-zinc-300 font-mono",
                        onchange: {
                            let mut config = config;
                            let on_sample_rate_change = on_sample_rate_change;
                            move |evt| {
                                if let Ok(hz) = evt.value().parse::<f64>() {
                                    config.write().sample_rate_hz = hz;
                                    on_sample_rate_change.call(hz);
                                }
                            }
                        },
                        for &rate in &supported {
                            option {
                                value: "{rate}",
                                selected: (rate - sample_rate_hz).abs() < 1.0,
                                "{fmt_rate(rate)}"
                            }
                        }
                    }
                }
            }

            div { class: "border-t border-gray-200 dark:border-zinc-800 mx-2" }

            // Analog channel section
            if has_analog {
                div { class: "px-2 pt-2 pb-0.5",
                    span { class: "text-[9px] font-bold text-gray-400 dark:text-zinc-500 uppercase tracking-wider", "Analog" }
                }
                for (i, name) in analog_channels.iter().map(|ch| &ch.name).enumerate() {
                    {
                        let is_enabled = analog_enabled.get(i).copied().unwrap_or(true);
                        let mut view = view;
                        let mut config = config;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_enabled {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-gray-700 hover:bg-gray-100 dark:text-zinc-300 dark:hover:bg-zinc-800/30 cursor-pointer rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-gray-400 hover:bg-gray-100 dark:text-zinc-600 dark:hover:bg-zinc-800/30 cursor-pointer rounded mx-1 line-through"
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
                                    if let Some(en) = config.write().analog_enabled.get_mut(i) {
                                        *en = !*en;
                                    }
                                },
                                span {
                                    class: "w-2.5 h-2.5 rounded-full flex-shrink-0",
                                    style: "background-color: {channel_color(i)}"
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
                    div { class: "border-t border-gray-200 dark:border-zinc-800 mx-2 my-1" }
                }
                div { class: "px-2 pt-1 pb-0.5",
                    span { class: "text-[9px] font-bold text-gray-400 dark:text-zinc-500 uppercase tracking-wider", "Digital" }
                }
                for (i, name) in digital_channels.iter().map(|ch| &ch.name).enumerate() {
                    {
                        let is_enabled = digital_enabled.get(i).copied().unwrap_or(true);
                        let mut view = view;
                        let mut config = config;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_enabled {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-gray-700 hover:bg-gray-100 dark:text-zinc-300 dark:hover:bg-zinc-800/30 cursor-pointer rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-gray-400 hover:bg-gray-100 dark:text-zinc-600 dark:hover:bg-zinc-800/30 cursor-pointer rounded mx-1 line-through"
                                },
                                oncontextmenu: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    let mut v = view.write();
                                    if let Some(row) = v.rows.iter_mut()
                                        .find(|r| matches!(r.kind, RowKind::Digital) && r.channel_index == i)
                                    {
                                        row.visible = !row.visible;
                                    }
                                    if let Some(en) = config.write().digital_enabled.get_mut(i) {
                                        *en = !*en;
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

            if sc > 0 {
                div { class: "border-t border-gray-200 dark:border-zinc-800 px-2 py-1.5",
                    p { class: "text-[10px] text-gray-400 dark:text-zinc-600", "{sc} samples" }
                }
            }
        }
    }
}

fn channel_color(index: usize) -> &'static str {
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
    num.parse::<f64>().ok().map(|n| n * mul)
}
