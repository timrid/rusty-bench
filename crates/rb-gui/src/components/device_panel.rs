//! Device sidebar panel: scan, available devices, connected devices.

use dioxus::prelude::*;

use super::app::AppStateRef;

/// Sidebar component. All actions (scan, connect, start, stop, disconnect)
/// are processed directly in click handlers via `AppState`'s blocking methods.
#[component]
pub fn DevicePanel(data_version: Signal<u64>) -> Element {
    let _version = data_version();

    let state: AppStateRef = use_context();
    let s = state.borrow();

    let device_ids = s.connected_device_ids();
    let selected_id = s.selected_device.clone();

    let connected_set: std::collections::HashSet<String> = device_ids
        .iter()
        .map(|id| id.as_str().to_string())
        .collect();
    let available: Vec<rb_core::ScanResult> = s
        .scan_results
        .iter()
        .filter(|r| !connected_set.contains(&r.candidate.address))
        .cloned()
        .collect();

    let scan_error = s.scan_error.clone();
    let connect_error = s.connect_error.clone();

    let device_infos: Vec<_> = device_ids.iter().map(|id| {
        let label = state.borrow().device_label(id);
        let dev_state = state.borrow().device_state(id);
        let sample_count = state.borrow().device_sample_count(id);
        let is_selected = selected_id.as_ref() == Some(id);
        (id.clone(), label, dev_state, sample_count, is_selected)
    }).collect();

    drop(s);

    rsx! {
        div { class: "space-y-2",
            h2 { class: "text-base font-bold text-zinc-200 mb-1", "Devices" }
            hr { class: "border-zinc-700" }

            button {
                class: "w-full px-2 py-1 bg-blue-700 hover:bg-blue-600 text-white rounded text-xs",
                onclick: {
                    let state = state.clone();
                    move |_| {
                        state.borrow_mut().trigger_scan();
                        data_version += 1;
                    }
                },
                "\u{27F3} Scan"
            }

            if let Some(ref err) = scan_error {
                p { class: "text-red-400 text-xs", "{err}" }
            }
            if let Some(ref err) = connect_error {
                p { class: "text-red-400 text-xs", "{err}" }
            }

            if !available.is_empty() {
                hr { class: "border-zinc-700 mt-1" }
                p { class: "text-xs text-zinc-500 uppercase tracking-wide", "Available:" }
                for r in &available {
                    div { class: "flex items-center justify-between py-0.5",
                        span { class: "text-xs text-zinc-400 truncate flex-1", "{r.candidate.address}" }
                        button {
                            class: "ml-1 px-1.5 py-0.5 bg-green-800 hover:bg-green-700 text-green-200 rounded text-[10px]",
                            onclick: {
                                let state = state.clone();
                                let r = r.clone();
                                move |_| {
                                    state.borrow_mut().connect_blocking(&r);
                                    data_version += 1;
                                }
                            },
                            "Connect"
                        }
                    }
                }
            }

            hr { class: "border-zinc-700 mt-1" }
            p { class: "text-xs text-zinc-500 uppercase tracking-wide", "Connected:" }

            if device_ids.is_empty() {
                p { class: "text-xs text-zinc-600 italic", "(none)" }
            }

            for (id, label, dev_state, sample_count, is_selected) in &device_infos {
                div {
                    class: if *is_selected { "text-xs text-blue-300 cursor-pointer py-0.5" }
                           else { "text-xs text-zinc-400 cursor-pointer py-0.5 hover:text-zinc-200" },
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

                div { class: "flex items-center gap-1 pl-2",
                    match dev_state {
                        Some(rb_core::AcquisitionState::Running) => rsx! {
                            span { class: "text-green-400 text-xs", "\u{25CF}" }
                            button {
                                class: "px-1 py-0 bg-zinc-700 hover:bg-zinc-600 text-zinc-300 rounded text-[10px]",
                                title: "Stop",
                                onclick: {
                                    let state = state.clone();
                                    let id = id.clone();
                                    move |_| {
                                        state.borrow_mut().stop_blocking(&id);
                                        data_version += 1;
                                    }
                                },
                                "\u{23F9}"
                            }
                        },
                        Some(rb_core::AcquisitionState::Error(msg)) => {
                            let msg = msg.clone();
                            rsx! {
                                span {
                                    class: "text-red-400 text-xs cursor-help",
                                    title: "{msg}",
                                    "\u{26A0}"
                                }
                            }
                        }
                        _ => rsx! {
                            button {
                                class: "px-1 py-0 bg-zinc-700 hover:bg-zinc-600 text-zinc-300 rounded text-[10px]",
                                title: "Start",
                                onclick: {
                                    let state = state.clone();
                                    let id = id.clone();
                                    move |_| {
                                        state.borrow_mut().start_blocking(&id);
                                        data_version += 1;
                                    }
                                },
                                "\u{25B6}"
                            }
                        },
                    }
                    button {
                        class: "px-1 py-0 bg-zinc-700 hover:bg-red-800 text-zinc-300 rounded text-[10px]",
                        title: "Disconnect",
                        onclick: {
                            let state = state.clone();
                            let id = id.clone();
                            move |_| {
                                state.borrow_mut().disconnect_blocking(&id);
                                data_version += 1;
                            }
                        },
                        "\u{2716}"
                    }
                }

                p { class: "text-[10px] text-zinc-600 pl-2", "{sample_count} samples" }
            }
        }
    }
}
