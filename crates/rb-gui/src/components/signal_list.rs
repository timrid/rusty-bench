//! Signal list panel: channel names, colors, visibility toggles, and decoder
//! configuration. Lives left of the waveform canvas.

use dioxus::prelude::*;
use crate::waveform_state::WaveformView;

use super::app::AppStateRef;
use crate::components::decoder_config::DecoderConfig;

/// Left panel showing channel labels and decoder configuration.
#[component]
pub fn SignalList(
    device_id: rb_device::DeviceId,
    view: Signal<WaveformView>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (analog, digital, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (
                acq.analog_traces().to_vec(),
                acq.digital_trace().cloned(),
                acq.sample_count(),
            )
        } else if let Some(handle) = s.session.device(&device_id) {
            (
                handle.analog_traces().to_vec(),
                handle.digital_trace().cloned(),
                handle.sample_count(),
            )
        } else {
            (Vec::new(), None, 0)
        }
    };

    let has_analog = !analog.is_empty();
    let has_digital = digital.is_some();

    // Pre-compute channel infos for rendering.
    let analog_infos: Vec<_> = analog
        .iter()
        .enumerate()
        .map(|(i, trace)| {
            let name = trace.channel().name.clone();
            (i, name, true) // (index, name, enabled)
        })
        .collect();

    let digital_infos: Vec<_> = digital
        .as_ref()
        .map(|dt| {
            (0..dt.channels().len())
                .map(|i| (i, format!("D{i}")))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    rsx! {
        div { class: "w-36 flex-shrink-0 border-r border-zinc-800 bg-zinc-900/50 flex flex-col h-full overflow-y-auto select-none",

            // Analog channel section
            if has_analog {
                div { class: "px-2 pt-2 pb-0.5",
                    span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider",
                        "Analog"
                    }
                }
                for (i, name, enabled) in &analog_infos {
                    div {
                        class: if *enabled {
                            "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                        } else {
                            "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-600 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                        },
                        span {
                            class: "w-2.5 h-2.5 rounded-full flex-shrink-0",
                            style: "background-color: {channel_color(*i, true)}"
                        }
                        span { class: "truncate flex-1", "{name}" }
                    }
                }
            }

            // Digital channel section
            if has_digital {
                if has_analog {
                    div { class: "border-t border-zinc-800 mx-2 my-1" }
                }
                div { class: "px-2 pt-1 pb-0.5",
                    span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider",
                        "Digital"
                    }
                }
                for (i, label) in &digital_infos {
                    div { class: "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 hover:bg-zinc-800/30 rounded mx-1",
                        span {
                            class: "w-2.5 h-2.5 rounded flex-shrink-0",
                            style: "background-color: {digital_channel_color(*i)}"
                        }
                        span { class: "truncate flex-1", "{label}" }
                    }
                }
            }

            // Divider
            if has_analog || has_digital {
                div { class: "border-t border-zinc-800 mx-2 my-1" }
            }

            // Decoder section
            div { class: "px-2 pt-1 pb-0.5",
                span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider",
                    "Decoder"
                }
            }
            div { class: "px-2 pb-2",
                DecoderConfig { view }
            }

            // Spacer
            div { class: "flex-1" }

            // Sample info at bottom
            if sample_count > 0 {
                div { class: "border-t border-zinc-800 px-2 py-1.5",
                    p { class: "text-[10px] text-zinc-600",
                        "{sample_count} samples"
                    }
                }
            }
        }
    }
}

/// Analog channel color palette.
fn channel_color(index: usize, _analog: bool) -> &'static str {
    const COLORS: &[&str] = &[
        "#facc15", // yellow
        "#60a5fa", // blue
        "#f87171", // red
        "#34d399", // green
        "#c084fc", // purple
        "#fb923c", // orange
        "#2dd4bf", // teal
        "#f472b6", // pink
    ];
    COLORS[index % COLORS.len()]
}

/// Digital channel color palette (greens/cyans).
fn digital_channel_color(index: usize) -> &'static str {
    const COLORS: &[&str] = &[
        "#34d399", "#2dd4bf", "#22d3ee", "#38bdf8",
        "#818cf8", "#a78bfa", "#f472b6", "#fb923c",
    ];
    COLORS[index % COLORS.len()]
}
