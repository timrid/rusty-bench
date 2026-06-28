//! Top bar: Device dropdown (left) | Session tab bar (center) | Settings icon (right).
//!
//! The device dropdown lists all scanned devices. Selecting a device assigns it
//! to the active session but does NOT connect — connection happens on Play.
//! A "Refresh" button inside the dropdown triggers a scan (and WebUSB permission
//! on WASM).

use dioxus::prelude::*;

use crate::state::SessionId;

use super::app::AppStateRef;

/// The unified top bar replacing the old TitleBar + SessionSidebar navigation.
#[component]
pub fn TopBar(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let s = state.borrow();

    let session_ids: Vec<SessionId> = {
        let mut ids: Vec<SessionId> = s.sessions.keys().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids
    };
    let active_session = s.active_session;

    let scan_results = s.scan_results.clone();
    let scan_error = s.scan_error.clone();

    // Build session tab infos.
    let tabs: Vec<_> = session_ids
        .iter()
        .map(|&id| {
            let label = s.sessions.get(&id).map(|ss| ss.label.clone()).unwrap_or_default();
            let is_active = id == active_session;
            let is_running = s.sessions.get(&id).is_some_and(|ss| ss.is_running());
            (id, label, is_active, is_running)
        })
        .collect();

    // Current device label for the active session (shown in dropdown).
    let active_device_label = s
        .active_session_state()
        .map(|ss| ss.label.clone())
        .filter(|l| l != "Untitled");

    drop(s);

    rsx! {
        div { class: "h-8 bg-zinc-900 border-b border-zinc-800 flex items-center flex-shrink-0",
            // ── Device Dropdown ──────────────────────────────────────────
            DeviceDropdown {
                scan_results,
                scan_error,
                active_device_label,
                data_version,
            }

            div { class: "w-px bg-zinc-800 h-full" }

            // ── Session Tab Bar ──────────────────────────────────────────
            SessionTabBar {
                tabs,
                active_session,
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
    active_device_label: Option<String>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let mut open = use_signal(|| false);

    let display_text = active_device_label.unwrap_or_else(|| "Select device…".into());

    // Build device entry elements in regular Rust.
    let device_entries: Vec<_> = scan_results
        .iter()
        .map(|result| {
            let driver = result.driver.clone();
            let address = result.candidate.address.clone();
            let result = result.clone();
            (driver, address, result)
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

                div { class: "absolute top-full left-0 mt-0.5 w-64 bg-zinc-800 border border-zinc-700 rounded shadow-xl z-20",
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

                    // Error message
                    if let Some(ref err) = scan_error {
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

                    for (driver, address, scan_result) in &device_entries {
                        {
                            let driver = driver.clone();
                            let address = address.clone();
                            let sr = scan_result.clone();
                            let state = state.clone();
                            rsx! {
                                button {
                                    key: "{driver}-{address}",
                                    class: "w-full text-left px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors flex items-center gap-2",
                                    onclick: move |_| {
                                        let session_id = state.borrow().active_session;
                                        state.borrow_mut().assign_device_to_session(session_id, sr.clone());
                                        // Auto-connect immediately so device capabilities and
                                        // channel metadata are available in the session view.
                                        state.borrow_mut().connect_blocking_with_session(session_id, &sr);
                                        data_version += 1;
                                        open.set(false);
                                    },
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

// ── Session Tab Bar ───────────────────────────────────────────────────────────

#[component]
fn SessionTabBar(
    tabs: Vec<(SessionId, String, bool, bool)>,
    active_session: SessionId,
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
                                    state.borrow_mut().active_session = id;
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
                                    title: "Close session",
                                    onclick: {
                                        let state = state.clone();
                                        move |evt| {
                                            evt.stop_propagation();
                                            state.borrow_mut().close_session(id);
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
                title: "New session",
                onclick: {
                    let state = state.clone();
                    move |_| {
                        state.borrow_mut().create_session("Untitled");
                        data_version += 1;
                    }
                },
                "\u{002B}"
            }

            div { class: "flex-1 border-b border-zinc-800" }
        }
    }
}
