//! Top bar: Device dropdown (left) | Tab bar (center) | Settings icon (right).
//!
//! The device dropdown lists all scanned devices with connection status.
//! A device is connected once at the program level and shared across tabs.

use dioxus::prelude::*;
use rb_device::DeviceId;

use crate::tab_state::TabId;

use super::app::AppStateRef;

/// The unified top bar replacing the old TitleBar + SessionSidebar navigation.
#[component]
pub fn TopBar(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let s = state.borrow();

    let tab_ids: Vec<TabId> = {
        let mut ids: Vec<TabId> = s.tabs.keys().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids
    };
    let active_tab = s.active_tab;

    let scan_results = s.device_manager.scan_results.clone();
    let scan_error = s.device_manager.scan_error.clone();
    let connect_error = s.device_manager.connect_error.clone();

    // Build tab infos.
    let tabs: Vec<_> = tab_ids
        .iter()
        .map(|&id| {
            let label = s.tabs.get(&id).map(|t| t.label.clone()).unwrap_or_default();
            let is_active = id == active_tab;
            let is_running = s.tabs.get(&id).is_some_and(|t| t.is_running());
            (id, label, is_active, is_running)
        })
        .collect();

    // Current device label for the active tab (shown in dropdown).
    let active_device_label = s
        .active_tab_state()
        .map(|t| t.label.clone())
        .filter(|l| l != "Untitled");

    // DeviceId of the active tab (for highlighting in dropdown).
    let active_device_id = s
        .active_tab_state()
        .and_then(|t| t.assigned_device_id().cloned());

    // Set of connected device IDs (for status icons).
    let connected_ids: Vec<DeviceId> = s.device_manager.connected_device_ids();

    drop(s);

    rsx! {
        div { class: "h-8 bg-zinc-900 border-b border-zinc-800 flex items-center flex-shrink-0",
            // ── Device Dropdown ──────────────────────────────────────────
            DeviceDropdown {
                scan_results,
                scan_error,
                connect_error,
                active_device_label,
                active_device_id,
                connected_ids,
                data_version,
            }

            div { class: "w-px bg-zinc-800 h-full" }

            // ── Tab Bar ──────────────────────────────────────────────────
            TabBar {
                tabs,
                active_tab,
                data_version,
            }

            div { class: "w-px bg-zinc-800 h-full" }

            // ── Settings Button ─────────────────────────────────────────
            button {
                class: "text-zinc-500 hover:text-zinc-200 px-3 h-full flex items-center transition-colors",
                title: "Settings",
                "\u{2699}"
            }
        }
    }
}

// ── Device Dropdown ───────────────────────────────────────────────────────────

#[component]
fn DeviceDropdown(
    scan_results: Vec<rb_core::ScanResult>,
    scan_error: Option<String>,
    connect_error: Option<String>,
    active_device_label: Option<String>,
    active_device_id: Option<DeviceId>,
    connected_ids: Vec<DeviceId>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let mut open = use_signal(|| false);

    let display_text = active_device_label.unwrap_or_else(|| "Select device…".into());

    // Build device entries with connection status.
    let device_entries: Vec<_> = scan_results
        .iter()
        .map(|sr| {
            let driver = sr.driver.clone();
            let address = sr.candidate.address.clone();
            let scan_result = sr.clone();
            // Determine connection status via DeviceManager's direct mapping.
            let is_connected = state
                .borrow()
                .device_manager
                .is_connected_result(&scan_result);
            // Check if this is the device used by the active session.
            let is_active_device = active_device_id.as_ref().is_some_and(|active_did| {
                connected_ids.contains(active_did) && is_connected
            });
            (driver, address, scan_result, is_connected, is_active_device)
        })
        .collect();

    rsx! {
        div { class: "relative",
            // Dropdown trigger
            button {
                class: "flex items-center gap-1.5 h-full px-3 text-xs text-zinc-300 hover:bg-zinc-800 transition-colors min-w-[160px]",
                onclick: move |_| open.set(!open()),
                span { class: "truncate", "{display_text}" }
                span { class: "text-zinc-500 text-[9px]", "\u{25BC}" }
            }

            // Dropdown menu
            if open() {
                // Backdrop to close on outside click
                div {
                    class: "fixed inset-0 z-10",
                    onclick: move |_| open.set(false),
                }

                div { class: "absolute top-full left-0 mt-0.5 w-72 bg-zinc-800 border border-zinc-700 rounded shadow-xl z-20",
                    // Refresh button
                    button {
                        class: "w-full flex items-center gap-1.5 px-3 py-1.5 text-xs text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700 transition-colors border-b border-zinc-700",
                        onclick: {
                            let state = state.clone();
                            move |_| {
                                state.borrow_mut().trigger_scan();
                                data_version += 1;
                            }
                        },
                        span { class: "text-[10px]", "\u{27F3}" }
                        "Refresh"
                    }

                    // Error messages
                    if let Some(ref err) = scan_error {
                        div { class: "px-3 py-1 text-[10px] text-red-400 border-b border-zinc-700",
                            "{err}"
                        }
                    }
                    if let Some(ref err) = connect_error {
                        div { class: "px-3 py-1 text-[10px] text-red-400 border-b border-zinc-700",
                            "{err}"
                        }
                    }

                    // Device list
                    if device_entries.is_empty() {
                        div { class: "px-3 py-2 text-xs text-zinc-600 italic",
                            "No devices found. Click Refresh to scan."
                        }
                    }

                    for (driver, address, scan_result, is_connected, is_active_device) in &device_entries {
                        {
                            let driver = driver.clone();
                            let address = address.clone();
                            let sr = scan_result.clone();
                            let state = state.clone();
                            let is_connected = *is_connected;
                            let is_active_device = *is_active_device;

                            // Connection status icon
                            let status_icon = if is_connected {
                                "\u{26A1}"  // ⚡ connected
                            } else {
                                "\u{25CB}"  // ○ not connected
                            };
                            let status_color = if is_connected {
                                "text-green-400"
                            } else {
                                "text-zinc-600"
                            };
                            let row_class = if is_active_device {
                                "w-full text-left px-3 py-1.5 text-xs text-zinc-200 bg-zinc-700/50 hover:bg-zinc-600 transition-colors flex items-center gap-2 border-l-2 border-l-blue-500"
                            } else {
                                "w-full text-left px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors flex items-center gap-2"
                            };

                            rsx! {
                                button {
                                    key: "{driver}-{address}",
                                    class: "{row_class}",
                                    onclick: move |_| {
                                        let tab_id = state.borrow().active_tab;
                                        if !is_connected {
                                            let _ = state.borrow_mut().connect_and_assign(tab_id, &sr);
                                        } else {
                                            // Already connected — assign the existing DeviceId.
                                            let did = state.borrow().device_manager.device_id_for_result(&sr);
                                            if let Some(did) = did {
                                                state.borrow_mut().assign_device_to_tab(tab_id, did);
                                            }
                                        }
                                        data_version += 1;
                                        open.set(false);
                                    },
                                    span { class: "text-[10px] {status_color} flex-shrink-0", "{status_icon}" }
                                    span { class: "text-zinc-400 font-mono text-[10px] truncate", "{driver}" }
                                    span { class: "text-zinc-600 text-[10px] truncate flex-1 text-right", "{address}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Tab Bar ───────────────────────────────────────────────────────────────────

#[component]
fn TabBar(
    tabs: Vec<(TabId, String, bool, bool)>,
    active_tab: TabId,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();

    rsx! {
        div { class: "flex items-center flex-1 overflow-x-auto px-1 gap-0.5",
            for (id, label, is_active, is_running) in &tabs {
                {
                    let id = *id;
                    let label = label.clone();
                    let is_active = *is_active;
                    let is_running = *is_running;
                    rsx! {
                        div {
                            class: if is_active {
                                "flex items-center gap-1 px-3 py-1 text-xs bg-zinc-800 text-zinc-200 rounded-t cursor-pointer border-t border-x border-zinc-700 h-full"
                            } else {
                                "flex items-center gap-1 px-3 py-1 text-xs text-zinc-500 hover:text-zinc-300 cursor-pointer rounded-t h-full"
                            },
                            onclick: {
                                let state = state.clone();
                                move |_| {
                                    state.borrow_mut().active_tab = id;
                                    data_version += 1;
                                }
                            },
                            // Recording indicator or label
                            if is_running {
                                span { class: "w-1.5 h-1.5 rounded-full bg-red-500 animate-pulse flex-shrink-0" }
                            }
                            span { class: "truncate max-w-[120px]", "{label}" }

                            // Close button — only when NOT running
                            if !is_running {
                                button {
                                    class: "ml-1 text-zinc-600 hover:text-red-400 rounded hover:bg-zinc-700/50 transition-colors flex-shrink-0",
                                    title: "Close tab",
                                    onclick: {
                                        let state = state.clone();
                                        move |evt| {
                                            evt.stop_propagation();
                                            state.borrow_mut().close_tab(id);
                                            data_version += 1;
                                        }
                                    },
                                    span { class: "text-[10px]", "\u{2715}" }
                                }
                            }
                        }
                    }
                }
            }

            // "+" New Tab button
            button {
                class: "px-2 py-1 text-xs text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800/50 rounded transition-colors flex-shrink-0",
                title: "New tab",
                onclick: {
                    let state = state.clone();
                    move |_| {
                        // Inherit the device from the currently active tab.
                        let assigned_device_id = state
                            .borrow()
                            .active_tab_state()
                            .and_then(|t| t.assigned_device_id().cloned());
                        let mut s = state.borrow_mut();
                        let new_id = s.create_tab("Untitled");
                        if let Some(ref did) = assigned_device_id {
                            s.assign_device_to_tab(new_id, did.clone());
                        }
                        data_version += 1;
                    }
                },
                "\u{002B}"
            }

            div { class: "flex-1 border-b border-zinc-800" }
        }
    }
}
