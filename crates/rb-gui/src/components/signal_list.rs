//! Signal list panel: channel names, colors, visibility toggles (right-click),
//! and decoder configuration. Lives left of the waveform canvas.

use dioxus::prelude::*;
use crate::waveform_state::{RowKind, WaveformView};

use super::app::AppStateRef;
use crate::components::decoder_config::DecoderConfig;

/// Left panel showing channel labels and decoder configuration.
#[component]
pub fn SignalList(
    device_id: rb_device::DeviceId,
    mut view: Signal<WaveformView>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (analog_names, digital_names, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            let anames: Vec<String> = acq.analog_traces().iter().map(|t| t.channel().name.clone()).collect();
            let dnames: Vec<String> = acq.digital_trace()
                .map(|dt| dt.channels().iter().map(|ch| ch.name.clone()).collect())
                .unwrap_or_default();
            (anames, dnames, acq.sample_count())
        } else if let Some(handle) = s.session.device(&device_id) {
            let anames: Vec<String> = handle.analog_traces().iter().map(|t| t.channel().name.clone()).collect();
            let dnames: Vec<String> = handle.digital_trace()
                .map(|dt| dt.channels().iter().map(|ch| ch.name.clone()).collect())
                .unwrap_or_default();
            (anames, dnames, handle.sample_count())
        } else {
            (Vec::new(), Vec::new(), 0)
        }
    };

    let has_analog = !analog_names.is_empty();
    let has_digital = !digital_names.is_empty();

    // Read current rows to get visibility state.
    let rows = view.read().rows.clone();

    rsx! {
        div { class: "w-36 flex-shrink-0 border-r border-zinc-800 bg-zinc-900/50 flex flex-col h-full overflow-y-auto select-none",

            // Analog channel section
            if has_analog {
                div { class: "px-2 pt-2 pb-0.5",
                    span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider", "Analog" }
                }
                for (i, name) in analog_names.iter().enumerate() {
                    {
                        let is_visible = rows.iter().any(|r| {
                            match r.kind { RowKind::Analog => r.channel_index == i && r.visible, _ => false }
                        });
                        let mut view = view;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_visible {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-600 cursor-pointer hover:bg-zinc-800/30 rounded mx-1 line-through"
                                },
                                oncontextmenu: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    let mut v = view.write();
                                    if let Some(row) = v.rows.iter_mut()
                                        .find(|r| matches!(r.kind, RowKind::Analog) && r.channel_index == i)
                                    {
                                        row.visible = !row.visible;
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
                        let is_visible = rows.iter().any(|r| {
                            match r.kind { RowKind::Digital => r.channel_index == i && r.visible, _ => false }
                        });
                        let mut view = view;
                        let i = i;
                        rsx! {
                            div {
                                class: if is_visible {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded mx-1"
                                } else {
                                    "flex items-center gap-1.5 px-2 py-1 text-xs text-zinc-600 cursor-pointer hover:bg-zinc-800/30 rounded mx-1 line-through"
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

            // Divider
            if has_analog || has_digital {
                div { class: "border-t border-zinc-800 mx-2 my-1" }
            }

            // Decoder section
            div { class: "px-2 pt-1 pb-0.5",
                span { class: "text-[9px] font-bold text-zinc-500 uppercase tracking-wider", "Decoder" }
            }
            div { class: "px-2 pb-2",
                DecoderConfig { view }
            }

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
