//! Device view: the main content area for a single connected device.
//!
//! Layout:
//! ┌──────────────────────────────┐
//! │ Device Tabs (if multiple)    │
//! ├──────────────────────────────┤
//! │ Canvas Toolbar               │
//! ├────────┬─────────────────────┤
//! │ Signal │                     │
//! │ List   │    Canvas           │
//! │        │                     │
//! ├────────┴─────────────────────┤
//! │ Control Cards                │
//! └──────────────────────────────┘

use dioxus::prelude::*;

use super::app::AppStateRef;
use super::canvas_toolbar::CanvasToolbar;
use super::control_cards::ControlCards;
use super::signal_list::SignalList;
use super::waveform_canvas::WaveformCanvas;

/// The device view for the currently selected device.
#[component]
pub fn DeviceView(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    // Determine which device to show.
    // If selected_device is set and connected, use it. Otherwise pick first connected.
    let s = state.borrow();
    let device_ids = s.connected_device_ids();

    let selected_id = {
        let sel = s.selected_device.clone();
        if sel.as_ref().is_some_and(|sid| device_ids.contains(sid)) {
            sel
        } else {
            device_ids.first().cloned()
        }
    };
    drop(s);

    // Persist the computed selection so sidebar stays in sync.
    {
        let mut s = state.borrow_mut();
        s.selected_device = selected_id.clone();
    }

    let Some(ref device_id) = selected_id else {
        return rsx! {
            div { class: "flex-1 flex flex-col items-center justify-center",
                div { class: "text-center space-y-3",
                    div { class: "text-5xl mb-2", "\u{1F50C}" }
                    h2 { class: "text-lg font-bold text-zinc-400", "No Device Connected" }
                    p { class: "text-xs text-zinc-600 max-w-sm",
                        "Use the sidebar to scan for devices and connect one, or enable demo devices to get started without hardware."
                    }
                    button {
                        class: "bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded text-sm font-medium transition-colors mt-2",
                        onclick: {
                            let state = state.clone();
                            move |_| { state.borrow_mut().trigger_scan(); }
                        },
                        "\u{27F3}  Scan for Devices"
                    }
                }
            }
        };
    };

    // Tab bar (only show if multiple devices)
    let tab_elements = if device_ids.len() > 1 {
        let tab_infos: Vec<_> = {
            let s = state.borrow();
            device_ids
                .iter()
                .map(|id| {
                    let label = s.device_label(id);
                    let is_active = Some(id) == selected_id.as_ref();
                    (id.clone(), label, is_active)
                })
                .collect()
        };

        rsx! {
            div { class: "flex border-b border-zinc-800 px-1 pt-1 gap-0.5 flex-shrink-0",
                for (id, label, is_active) in &tab_infos {
                    div {
                        class: if *is_active {
                            "px-3 py-1 text-xs bg-zinc-800 text-zinc-200 rounded-t cursor-pointer border-t border-x border-zinc-700"
                        } else {
                            "px-3 py-1 text-xs text-zinc-500 hover:text-zinc-300 cursor-pointer rounded-t"
                        },
                        onclick: {
                            let state = state.clone();
                            let id = id.clone();
                            move |_| {
                                state.borrow_mut().selected_device = Some(id.clone());
                                data_version += 1;
                            }
                        },
                        "{label}"
                    }
                }
                div { class: "flex-1 border-b border-zinc-800" }
            }
        }
    } else {
        rsx! {}
    };

    // Per-device view state — created once, shared with all children.
    let view = {
        let s = state.borrow();
        s.views.get(device_id).cloned().unwrap_or_default()
    };
    let view_signal = use_signal(move || view);

    rsx! {
        div { class: "flex-1 flex flex-col overflow-hidden",
            {tab_elements}

            // Canvas toolbar
            CanvasToolbar {
                device_id: device_id.clone(),
                view: view_signal,
                data_version,
            }

            // Canvas area: Signal List | Canvas
            div { class: "flex-1 flex overflow-hidden",
                SignalList {
                    device_id: device_id.clone(),
                    view: view_signal,
                    data_version,
                }
                div { class: "flex-1 overflow-hidden",
                    WaveformCanvas {
                        device_id: device_id.clone(),
                        data_version,
                        view: view_signal,
                    }
                }
            }

            // Control cards for non-timeline capabilities
            ControlCards {
                device_id: device_id.clone(),
                data_version,
            }
        }
    }
}
