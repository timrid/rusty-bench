//! Central panel: tab bar + waveform view for the selected device.

use dioxus::prelude::*;

use super::app::AppStateRef;

/// Central area showing a tab bar of connected devices and the waveform view.
/// Reads [`AppStateRef`] from context.
#[component]
pub fn MainPanel(data_version: Signal<u64>) -> Element {
    let _version = data_version();

    let state: AppStateRef = use_context();

    let device_ids = state.borrow().connected_device_ids();
    let mut selected_id = state.borrow().selected_device.clone();

    if selected_id.as_ref().is_none_or(|sid| !device_ids.contains(sid)) {
        selected_id = device_ids.first().cloned();
        state.borrow_mut().selected_device = selected_id.clone();
    }

    if device_ids.is_empty() {
        return rsx! {
            div { class: "flex-1 flex items-center justify-center",
                p { class: "text-zinc-500 text-sm text-center",
                    "No devices connected.\n\nUse the sidebar to scan and connect a device."
                }
            }
        };
    }

    // Pre-compute labels and active status outside rsx!
    let tab_infos: Vec<_> = device_ids.iter().map(|id| {
        let label = state.borrow().device_label(id);
        let is_active = selected_id.as_ref() == Some(id);
        (id.clone(), label, is_active)
    }).collect();

    rsx! {
        div { class: "flex-1 flex flex-col overflow-hidden",
            div { class: "flex border-b border-zinc-800 px-1 pt-1 gap-0.5",
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

            if let Some(ref id) = selected_id {
                div { class: "flex-1 overflow-hidden",
                    super::waveform_canvas::WaveformCanvas {
                        device_id: id.clone(),
                        data_version,
                    }
                }
            }
        }
    }
}
