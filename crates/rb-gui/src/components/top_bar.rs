//! Top bar: Device dropdown (left) | Tab bar (center) | Settings icon (right).
//!
//! The device dropdown shows known devices split into Connected and
//! Not Connected sections. Connect/Disconnect is explicit and is the only
//! way to manage device connections.

use dioxus::prelude::*;
use rb_core::KnownDevice;
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

    let known_devices = s.device_manager.known_devices_owned();
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

    // Lock tab-switching during acquisition (not the whole dropdown).
    let is_locked = s.is_device_locked();

    drop(s);

    rsx! {
        div { class: "h-8 bg-zinc-900 border-b border-zinc-800 flex items-center flex-shrink-0",
            // ── Device Dropdown ──────────────────────────────────────────
            DeviceDropdown {
                known_devices,
                scan_error,
                connect_error,
                active_device_label,
                active_device_id,
                is_locked,
                data_version,
            }

            div { class: "w-px bg-zinc-800 h-full" }

            // ── Tab Bar ──────────────────────────────────────────────────
            TabBar {
                tabs,
                active_tab,
                is_locked,
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
    known_devices: Vec<KnownDevice>,
    scan_error: Option<String>,
    connect_error: Option<String>,
    active_device_label: Option<String>,
    active_device_id: Option<DeviceId>,
    is_locked: bool,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let mut open = use_signal(|| false);

    // Read data_version to establish a reactive dependency
    // (after connect/disconnect, this forces a re-render of the dropdown).
    let _ver = data_version();

    let display_text = active_device_label.unwrap_or_else(|| "Select device…".into());

    // Stable-sort: connection state must not reorder the list.
    let mut known_devices = known_devices;
    known_devices.sort_by(|a, b| {
        a.driver.cmp(&b.driver)
            .then_with(|| a.candidate.address.cmp(&b.candidate.address))
    });

    let has_devices = !known_devices.is_empty();
    let open_signal = open;

    // Track which device is currently being connected (key = "driver-address").
    let connecting_device = use_signal(|| Option::<String>::None);

    // ── Desktop: auto-refresh while dropdown is open ────────────────────
    #[cfg(not(target_arch = "wasm32"))]
    {
        state.borrow().set_auto_scan(open());
    }

    rsx! {
        div { class: "relative",
            // Dropdown trigger — always clickable to open the menu
            button {
                class: "flex items-center gap-1.5 h-full px-3 text-xs text-zinc-300 hover:bg-zinc-800 transition-colors w-[180px] flex-shrink-0",
                onclick: move |_| {
                    let was_closed = !open();
                    open.set(!open());
                    if was_closed {
                        crate::app_state::AppState::trigger_scan(&state, data_version, false);
                    }
                },
                span { class: "truncate", "{display_text}" }
                span { class: "text-zinc-500 text-[9px]", "\u{25BC}" }
            }

            // Dropdown menu
            if open() {
                div {
                    class: "fixed inset-0 z-10",
                    onclick: move |_| open.set(false),
                }

                div { class: "absolute top-full left-0 mt-0.5 w-80 bg-zinc-800 border border-zinc-700 rounded shadow-xl z-20 max-h-[70vh] overflow-y-auto",
                    // Error messages
                    if let Some(ref err) = scan_error {
                        div { class: "px-3 py-1 text-[10px] text-red-400 border-b border-zinc-700", "{err}" }
                    }
                    if let Some(ref err) = connect_error {
                        div { class: "px-3 py-1 text-[10px] text-red-400 border-b border-zinc-700", "{err}" }
                    }

                    // ── Device list (single flat list) ──────────────────
                    if has_devices {
                        for kd in &known_devices {
                            DeviceRow {
                                key: "{kd.driver}-{kd.candidate.address}",
                                kd: kd.clone(),
                                is_active: active_device_id.as_ref().is_some_and(|did| {
                                    state.borrow().device_manager.device_id_for_result(kd).as_ref() == Some(did)
                                }),
                                is_locked,
                                open_signal,
                                connecting_device,
                                data_version,
                            }
                        }
                    } else {
                        div { class: "px-3 py-2 text-xs text-zinc-600 italic",
                            "No devices found."
                        }
                    }

                    // ── Footer Actions ────────────────────────────────
                    {
                        #[cfg(not(target_arch = "wasm32"))]
                        {{
                            rsx! {
                                div { class: "border-t border-zinc-700 flex",
                                    button {
                                        class: "flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs text-zinc-500 hover:text-zinc-300 hover:bg-zinc-700 transition-colors cursor-not-allowed",
                                        title: "Coming soon — manually add IP/network devices",
                                        disabled: true,
                                        span { class: "text-[10px]", "+" }
                                        "Add device…"
                                    }
                                }
                            }
                        }}
                        #[cfg(target_arch = "wasm32")]
                        {{
                            rsx! {
                                div { class: "border-t border-zinc-700",
                                    button {
                                        class: "w-full flex items-center gap-1.5 px-3 py-1.5 text-xs text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700 transition-colors",
                                        onclick: {
                                            let state = state.clone();
                                            move |_| {
                                                crate::app_state::AppState::trigger_scan(&state, data_version, true);
                                                data_version += 1;
                                            }
                                        },
                                        span { class: "text-[10px]", "\u{1F50C}" }
                                        "Connect USB devices"
                                    }
                                    button {
                                        class: "w-full flex items-center gap-1.5 px-3 py-1.5 text-xs text-zinc-600 transition-colors",
                                        title: "Coming soon — WebSerial support",
                                        disabled: true,
                                        span { class: "text-[10px]", "\u{1F4E1}" }
                                        "Connect serial devices"
                                    }
                                    button {
                                        class: "w-full flex items-center gap-1.5 px-3 py-1.5 text-xs text-zinc-600 transition-colors",
                                        title: "Coming soon — WebBluetooth support",
                                        disabled: true,
                                        span { class: "text-[10px]", "\u{1F4F6}" }
                                        "Connect Bluetooth devices"
                                    }
                                }
                            }
                        }}
                    }
                }
            }
        }
    }
}

// ── Device Row (unified — single list, auto-connect on click) ─────────────────

#[component]
fn DeviceRow(
    kd: KnownDevice,
    is_active: bool,
    is_locked: bool,
    open_signal: Signal<bool>,
    connecting_device: Signal<Option<String>>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();

    let driver = kd.driver.clone();
    let vendor = kd.candidate.info.vendor.clone();
    let model = kd.candidate.info.model.clone();
    let serial = kd.candidate.info.serial.clone();
    let address = kd.candidate.address.clone();

    let is_connected = state.borrow().device_manager.is_connected_result(&kd);
    let device_key = format!("{}-{}", driver, address);
    let is_connecting = connecting_device() == Some(device_key.clone());

    // Additional info only for connected devices.
    let additional_info: Vec<(String, String)> = if is_connected {
        state.borrow().device_manager.device_id_for_result(&kd)
            .map(|did| {
                state.borrow().device_manager.additional_info(&did)
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let row_class = if is_active {
        "w-full text-left px-3 py-1.5 text-xs text-zinc-200 bg-zinc-700/30 border-l-2 border-l-blue-500"
    } else if is_connected {
        "w-full text-left px-3 py-1.5 text-xs text-zinc-200 border-l-2 border-l-green-500"
    } else if is_locked {
        "w-full text-left px-3 py-1.5 text-xs text-zinc-500"
    } else {
        "w-full text-left px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors"
    };

    let label = if !model.is_empty() {
        format!("{} {}", vendor, model)
    } else {
        vendor.clone()
    };
    let sub = if is_connected {
        if let Some(ref s) = serial { format!("S/N: {} · {}", s, address) }
        else { address.clone() }
    } else {
        address.clone()
    };

    rsx! {
        div { class: "{row_class}",
            onclick: {
                let state = state.clone();
                let kd = kd.clone();
                move |_| {
                    if is_locked { return; }
                    if is_connected {
                        // Connected device — no-op on row click (disconnect is a separate button).
                        return;
                    }
                    // Not connected — auto-connect.
                    let driver = kd.driver.clone();
                    let candidate = kd.candidate.clone();
                    let origin = kd.origin;
                    let dk = device_key.clone();
                    connecting_device.set(Some(dk));
                    let state_for_async = state.clone();
                    spawn(async move {
                        let result = crate::app_state::AppState::connect_single(
                            &state_for_async, &driver, &candidate, origin,
                        ).await;
                        if let Err(e) = result {
                            state_for_async.borrow_mut().device_manager.connect_error = Some(e);
                        }
                        connecting_device.set(None);
                        data_version += 1;
                        open_signal.set(false);
                    });
                }
            },
            div { class: "flex items-center gap-2",
                // Status icon
                span { class: "text-[10px] flex-shrink-0",
                    if is_active {
                        span { class: "text-blue-400", "\u{25CF}" }
                    } else if is_connected {
                        span { class: "text-green-400", "\u{26A1}" }
                    } else {
                        span { class: "text-zinc-600", "\u{25CB}" }
                    }
                }
                div { class: "flex-1 min-w-0",
                    div { class: "truncate text-zinc-200", "{label}" }
                    div { class: "text-[9px] text-zinc-500 truncate", "{sub}" }
                    div { class: "text-[9px] text-zinc-600 font-mono", "{driver}" }
                    for (key, value) in &additional_info {
                        div { class: "text-[9px] text-zinc-500",
                            span { class: "text-zinc-600", "{key}: " }
                            "{value}"
                        }
                    }
                }
                // Right side: Disconnect button or Connecting spinner
                if is_connecting {
                    span {
                        class: "text-[9px] text-yellow-400 px-1.5 py-0.5 rounded border border-yellow-700/50 bg-yellow-900/20 flex-shrink-0 flex items-center gap-1",
                        span { class: "inline-block w-2.5 h-2.5 border border-yellow-400 border-t-transparent rounded-full animate-spin" }
                        "Connecting…"
                    }
                } else if is_connected {
                    button {
                        class: "text-[9px] text-zinc-600 hover:text-red-400 px-1.5 py-0.5 rounded hover:bg-red-900/20 transition-colors flex-shrink-0",
                        onclick: {
                            let state = state.clone();
                            let kd = kd.clone();
                            move |evt| {
                                evt.stop_propagation();
                                let did = state.borrow().device_manager.device_id_for_result(&kd);
                                if let Some(ref did) = did {
                                    state.borrow_mut().device_manager.disconnect(did);
                                }
                                data_version += 1;
                            }
                        },
                        "Disconnect"
                    }
                }
            }
        }
    }
}

// ── Tab Bar ───────────────────────────────────────────────────────────────────

// ── Tab Bar ───────────────────────────────────────────────────────────────────

#[component]
fn TabBar(
    tabs: Vec<(TabId, String, bool, bool)>,
    active_tab: TabId,
    is_locked: bool,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();

    rsx! {
        div { class: "flex items-stretch flex-1 overflow-x-auto gap-0.5 h-full",
            for (id, label, is_active, is_running) in &tabs {
                {
                    let id = *id;
                    let label = label.clone();
                    let is_active = *is_active;
                    let is_running = *is_running;
                    rsx! {
                        div {
                            class: if is_active {
                                "flex items-center gap-1 px-3 text-xs bg-zinc-800 text-zinc-200 cursor-pointer border-t border-x border-zinc-700 h-full"
                            } else {
                                "flex items-center gap-1 px-3 text-xs text-zinc-500 hover:text-zinc-300 cursor-pointer h-full"
                            },
                            onclick: {
                                let state = state.clone();
                                move |_| {
                                    if !is_locked {
                                        state.borrow_mut().active_tab = id;
                                        data_version += 1;
                                    }
                                }
                            },
                            // Recording indicator or label
                            if is_running {
                                span { class: "w-1.5 h-1.5 rounded-full bg-red-500 animate-pulse flex-shrink-0" }
                            }
                            span { class: "truncate max-w-[120px]", "{label}" }

                            // Close button — hidden while device is locked
                            if !is_locked {
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

            // "+" New Tab button (disabled when device is locked)
            button {
                class: if is_locked {
                    "px-2 py-1 text-xs text-zinc-700 rounded flex-shrink-0 cursor-not-allowed"
                } else {
                    "px-2 py-1 text-xs text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800/50 rounded transition-colors flex-shrink-0"
                },
                disabled: is_locked,
                title: if is_locked { "Stop acquisition to create a new tab" } else { "New tab" },
                onclick: move |_| {
                    if !is_locked {
                        let state = state.clone();
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

            div { class: "flex-1" }
        }
    }
}
