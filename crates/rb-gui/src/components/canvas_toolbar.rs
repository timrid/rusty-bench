//! Canvas toolbar: Run/Stop, marker controls.
//! Sits directly above the waveform canvas.

use dioxus::prelude::*;
use rb_core::AcquisitionState;
use crate::waveform_state::WaveformView;

use super::app::AppStateRef;

/// Toolbar above the canvas with acquisition controls and marker buttons.
#[component]
pub fn CanvasToolbar(
    device_id: rb_device::DeviceId,
    view: Signal<WaveformView>,
    cursor_sample_pos: Signal<Option<u64>>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (acq_state, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (acq.state().clone(), acq.sample_count())
        } else if let Some(handle) = s.session.device(&device_id) {
            (handle.state().clone(), handle.sample_count())
        } else {
            (AcquisitionState::Idle, 0)
        }
    };

    let is_running = matches!(acq_state, AcquisitionState::Running);

    rsx! {
        div { class: "flex items-center gap-2 px-3 py-1.5 border-b border-zinc-800 bg-zinc-900/80 flex-shrink-0",
            // Run / Stop
            if is_running {
                button {
                    class: "flex items-center gap-1 px-2 py-1 bg-red-800 hover:bg-red-700 text-red-200 rounded text-xs font-medium transition-colors",
                    title: "Stop acquisition",
                    onclick: {
                        let state = state.clone();
                        let id = device_id.clone();
                        move |_| {
                            state.borrow_mut().stop_blocking(&id);
                            data_version += 1;
                        }
                    },
                    span { class: "text-[10px]", "\u{23F9}" }
                    "Stop"
                }
            } else {
                button {
                    class: "flex items-center gap-1 px-2 py-1 bg-green-800 hover:bg-green-700 text-green-200 rounded text-xs font-medium transition-colors",
                    title: "Start acquisition",
                    onclick: {
                        let state = state.clone();
                        let id = device_id.clone();
                        move |_| {
                            state.borrow_mut().start_blocking(&id);
                            data_version.set(data_version() + 1);
                        }
                    },
                    span { class: "text-[10px]", "\u{25B6}" }
                    "Run"
                }
            }

            span { class: "text-zinc-700", "|" }

            // Marker A button
            button {
                class: "px-2 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-amber-400 rounded text-xs font-medium transition-colors",
                title: "Add Marker A at cursor position",
                onclick: {
                    let mut view = view;
                    let cursor_sample_pos = cursor_sample_pos;
                    move |_| {
                        if let Some(pos) = cursor_sample_pos() {
                            view.write().add_marker(pos);
                        }
                    }
                },
                "\u{25C6} +A"
            }

            // Marker B button
            button {
                class: "px-2 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-amber-400 rounded text-xs font-medium transition-colors",
                title: "Add Marker B at cursor position",
                onclick: {
                    let mut view = view;
                    let cursor_sample_pos = cursor_sample_pos;
                    move |_| {
                        if let Some(pos) = cursor_sample_pos() {
                            view.write().add_marker(pos);
                        }
                    }
                },
                "\u{25C6} +B"
            }

            // Create Pair button
            button {
                class: "px-2 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-emerald-400 rounded text-xs font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed",
                title: "Create Marker Pair from last two markers",
                disabled: {
                    let v = view.read();
                    v.markers.len() < 2
                },
                onclick: {
                    let mut view = view;
                    move |_| {
                        let mut v = view.write();
                        let len = v.markers.len();
                        if len >= 2 {
                            let a = v.markers[len - 2].id;
                            let b = v.markers[len - 1].id;
                            v.add_marker_pair(a, b);
                        }
                    }
                },
                "\u{2194} Pair"
            }

            span { class: "text-zinc-700", "|" }

            // Clear all markers
            button {
                class: "px-2 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-zinc-400 rounded text-xs transition-colors",
                title: "Clear all markers and pairs",
                onclick: {
                    let mut view = view;
                    move |_| {
                        let mut v = view.write();
                        v.markers.clear();
                        v.marker_pairs.clear();
                    }
                },
                "\u{2715} Clear"
            }

            div { class: "flex-1" }

            // Sample count
            if sample_count > 0 {
                span { class: "text-[10px] text-zinc-600 font-mono",
                    "{sample_count} samples"
                }
            }
        }
    }
}
