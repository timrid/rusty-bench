//! Session sidebar: connected devices, scan, and persistent navigation.
//!
//! Collapses to a narrow icon rail for small screens / focus mode.

use dioxus::prelude::*;
use rb_core::AcquisitionState;

use super::app::AppStateRef;

/// Collapsible sidebar. Full width = w-48, collapsed = w-12 (icon rail).
#[component]
pub fn SessionSidebar(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let mut collapsed = use_signal(|| false);

    // Read state once before rendering.
    let _version = data_version();
    let s = state.borrow();

    let device_ids = s.connected_device_ids();
    let selected_id = s.selected_device.clone();

    // Connected device infos
    let connected: Vec<_> = device_ids
        .iter()
        .map(|id| {
            let label = s.device_label(id);
            let dev_state = s.device_state(id);
            let sample_count = s.device_sample_count(id);
            let is_selected = selected_id.as_ref() == Some(id);
            (id.clone(), label, dev_state, sample_count, is_selected)
        })
        .collect();

    // Available (scanned but not connected)
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
    drop(s);

    // Collapsed rail
    if collapsed() {
        let extra_count = connected.len().saturating_sub(6);
        return rsx! {
            div { class: "w-12 flex-shrink-0 bg-zinc-900/95 border-r border-zinc-800 flex flex-col h-full items-center py-3 gap-2 transition-all",
                button {
                    class: "text-zinc-500 hover:text-zinc-200 p-1.5 rounded hover:bg-zinc-800/50 transition-colors",
                    title: "Expand sidebar",
                    onclick: move |_| collapsed.set(false),
                    // Right-pointing chevron
                    span { class: "text-xs", "\u{276F}" }
                }

                div { class: "w-8 border-t border-zinc-800" }

                // Device icons (up to 6, then a "+N")
                for (_i, (id, label, dev_state, _, _)) in connected.iter().take(6).enumerate() {
                    DeviceRailIcon {
                        id: id.clone(),
                        label: label.clone(),
                        state: dev_state.clone(),
                        is_selected: selected_id.as_ref() == Some(id),
                    }
                }
                if extra_count > 0 {
                    span { class: "text-[10px] text-zinc-600", "+{extra_count}" }
                }

                div { class: "flex-1" }

                button {
                    class: "text-zinc-500 hover:text-zinc-200 p-1.5 rounded hover:bg-zinc-800/50 transition-colors",
                    title: "Settings",
                    "\u{2699}"
                }
            }
        };
    }

    // Expanded sidebar
    rsx! {
        div { class: "w-48 flex-shrink-0 bg-zinc-900/95 border-r border-zinc-800 flex flex-col h-full transition-all",
            // Header
            div { class: "flex items-center justify-between px-3 pt-3 pb-1",
                span { class: "text-[10px] font-bold text-zinc-500 uppercase tracking-wider select-none",
                    "Devices"
                }
                button {
                    class: "text-zinc-500 hover:text-zinc-200 p-0.5 rounded hover:bg-zinc-800/50 transition-colors",
                    title: "Collapse sidebar",
                    onclick: move |_| collapsed.set(true),
                    span { class: "text-xs", "\u{276E}" }
                }
            }

            // Scan button
            div { class: "px-3 pb-2",
                button {
                    class: "w-full px-2 py-1.5 bg-blue-700 hover:bg-blue-600 text-white rounded text-xs font-medium transition-colors",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            state.borrow_mut().trigger_scan();
                            data_version += 1;
                        }
                    },
                    "\u{27F3}  Scan"
                }
            }

            // Errors
            if let Some(ref err) = scan_error {
                div { class: "px-3 pb-1",
                    p { class: "text-red-400 text-[10px] bg-red-900/20 border border-red-800 rounded px-2 py-1 truncate",
                        title: "{err}",
                        "{err}"
                    }
                }
            }
            if let Some(ref err) = connect_error {
                div { class: "px-3 pb-1",
                    p { class: "text-red-400 text-[10px] bg-red-900/20 border border-red-800 rounded px-2 py-1 truncate",
                        title: "{err}",
                        "{err}"
                    }
                }
            }

            // Connected devices
            div { class: "flex-1 overflow-y-auto px-1.5",
                if connected.is_empty() {
                    div { class: "px-2 py-4 text-center",
                        p { class: "text-xs text-zinc-600 italic", "No devices connected" }
                    }
                }

                for (id, label, dev_state, sample_count, is_selected) in &connected {
                    div {
                        class: if *is_selected {
                            "mb-1 rounded bg-blue-900/30 border border-blue-800/50"
                        } else {
                            "mb-1 rounded hover:bg-zinc-800/30 border border-transparent"
                        },
                        // Device row
                        div {
                            class: "flex items-center gap-1.5 px-2 py-1.5 cursor-pointer",
                            onclick: {
                                let state = state.clone();
                                let id = id.clone();
                                move |_| {
                                    state.borrow_mut().selected_device = Some(id.clone());
                                    data_version += 1;
                                }
                            },
                            // Status dot
                            match dev_state {
                                Some(AcquisitionState::Running) => rsx! {
                                    span { class: "w-1.5 h-1.5 rounded-full bg-green-400 flex-shrink-0" }
                                },
                                Some(AcquisitionState::Error(_)) => rsx! {
                                    span { class: "w-1.5 h-1.5 rounded-full bg-red-400 flex-shrink-0" }
                                },
                                _ => rsx! {
                                    span { class: "w-1.5 h-1.5 rounded-full bg-zinc-600 flex-shrink-0" }
                                },
                            }
                            span { class: "text-xs text-zinc-300 truncate flex-1", "{label}" }
                        }

                        // Action buttons
                        div { class: "flex items-center gap-1 px-2 pb-1.5",
                            match dev_state {
                                Some(AcquisitionState::Running) => rsx! {
                                    button {
                                        class: "px-1.5 py-0.5 bg-zinc-700 hover:bg-zinc-600 text-zinc-300 rounded text-[10px] transition-colors",
                                        title: "Stop acquisition",
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
                                Some(AcquisitionState::Error(msg)) => {
                                    let msg = msg.clone();
                                    rsx! {
                                        span {
                                            class: "text-red-400 text-[10px] cursor-help truncate flex-1",
                                            title: "{msg}",
                                            "\u{26A0} {msg}"
                                        }
                                    }
                                }
                                _ => rsx! {
                                    button {
                                        class: "px-1.5 py-0.5 bg-zinc-700 hover:bg-zinc-600 text-zinc-300 rounded text-[10px] transition-colors",
                                        title: "Start acquisition",
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

                            span { class: "text-[10px] text-zinc-600 flex-1 text-right", "{sample_count} samp" }

                            button {
                                class: "px-1 py-0 bg-zinc-700 hover:bg-red-800 text-zinc-400 hover:text-red-200 rounded text-[10px] transition-colors",
                                title: "Disconnect device",
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
                    }
                }

                // Available devices section
                if !available.is_empty() {
                    div { class: "px-1.5 pt-2 pb-1",
                        p { class: "text-[10px] font-bold text-zinc-500 uppercase tracking-wider px-1",
                            "Available"
                        }
                    }
                    for r in &available {
                        div { class: "flex items-center justify-between px-2 py-1 mb-0.5 rounded hover:bg-zinc-800/30",
                            span { class: "text-xs text-zinc-500 truncate flex-1", "{r.driver}" }
                            button {
                                class: "ml-1 px-1.5 py-0.5 bg-green-800 hover:bg-green-700 text-green-200 rounded text-[10px] transition-colors",
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
            }

            // Persistent navigation
            div { class: "border-t border-zinc-800 px-1.5 pt-1 pb-2 space-y-0.5",
                button {
                    class: "w-full text-left px-2.5 py-1.5 rounded text-xs text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50 transition-colors",
                    "\u{2699}  Settings"
                }
                button {
                    class: "w-full text-left px-2.5 py-1.5 rounded text-xs text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50 transition-colors",
                    "\u{1F4C2}  Capture Library"
                }
                button {
                    class: "w-full text-left px-2.5 py-1.5 rounded text-xs text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50 transition-colors",
                    "\u{2139}  About"
                }
            }
        }
    }
}

/// Single device icon in the collapsed rail.
#[component]
fn DeviceRailIcon(
    id: rb_device::DeviceId,
    label: String,
    state: Option<rb_core::AcquisitionState>,
    is_selected: bool,
) -> Element {
    let state_ref: AppStateRef = use_context();

    let initial = label.chars().next().unwrap_or('?');
    let state_dot = match &state {
        Some(AcquisitionState::Running) => "bg-green-400",
        Some(AcquisitionState::Error(_)) => "bg-red-400",
        _ => "bg-zinc-600",
    };

    let border = if is_selected {
        "ring-1 ring-blue-500"
    } else {
        ""
    };

    rsx! {
        button {
            class: "relative w-8 h-8 rounded flex items-center justify-center text-xs font-medium text-zinc-300 hover:bg-zinc-800/50 transition-colors {border}",
            title: "{label}",
            onclick: {
                let state = state_ref.clone();
                let id = id.clone();
                move |_| {
                    state.borrow_mut().selected_device = Some(id.clone());
                }
            },
            "{initial}"
            span { class: "absolute bottom-0.5 right-0.5 w-1.5 h-1.5 rounded-full {state_dot}" }
        }
    }
}
