//! Tab view: the main content area for the active tab.
//!
//! Layout:
//! ┌──────────────────────────────────────┐
//! │ Canvas Toolbar                       │
//! ├────────┬───────────────┬─────────────┤
//! │ Signal │               │  Decoder    │
//! │ List   │    Canvas     │  Config     │
//! │        │               │             │
//! └────────┴───────────────┴─────────────┘

use dioxus::prelude::*;

use super::app::AppStateRef;
use crate::logic_analyzer::components::{
    CanvasToolbar, ChannelConfig, DecoderConfig, WaveformCanvas,
};

/// The main content view for the active session.
#[component]
pub fn DeviceView(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let s = state.borrow();
    let active_tab = s.active_tab;
    let device_id = s.device_id_for_tab(active_tab);
    let connect_error = s.device_manager.connect_error.clone();
    drop(s);

    // No device connected — show placeholder.
    let Some(ref device_id) = device_id else {
        return rsx! {
            div { class: "flex-1 flex flex-col items-center justify-center",
                div { class: "text-center space-y-3",
                    div { class: "text-5xl mb-2", "\u{1F50C}" }
                    h2 { class: "text-lg font-bold text-zinc-400", "No Device" }
                    p { class: "text-xs text-zinc-600 max-w-sm",
                        "Select a device from the dropdown above. It will be connected automatically."
                    }
                    if let Some(ref err) = connect_error {
                        p { class: "text-xs text-red-400 bg-red-900/20 border border-red-800 rounded px-3 py-1.5 mt-2 max-w-sm",
                            "{err}"
                        }
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

    // Device connected — show full view.
    // Per-tab view state from the active tab.
    let view = {
        let s = state.borrow();
        s.active_tab_state()
            .map(|t| t.view().clone())
            .unwrap_or_default()
    };
    let mut view_signal = use_signal(move || view);
    let cursor_sample_pos = use_signal(|| None::<u64>);

    // Persist view state across tab switches.
    // When the active tab changes: save the current view to the
    // previous tab and load the new tab's saved view.
    let mut prev_tab = use_signal(|| active_tab);
    if prev_tab() != active_tab {
        // Save current view to the old tab.
        {
            let mut s = state.borrow_mut();
            if let Some(old) = s.tabs.get_mut(&prev_tab()) {
                *old.view_mut() = view_signal.read().clone();
            }
        }
        // Load the new tab's saved view.
        let new_view = {
            let s = state.borrow();
            s.tabs.get(&active_tab)
                .map(|t| t.view().clone())
                .unwrap_or_default()
        };
        view_signal.set(new_view);
        prev_tab.set(active_tab);
    } else {
        // Same tab — sync view state back on each render.
        let mut s = state.borrow_mut();
        if let Some(active) = s.tabs.get_mut(&active_tab) {
            *active.view_mut() = view_signal.read().clone();
        }
    }

    rsx! {
        div { class: "flex-1 flex flex-col overflow-hidden",
            // Canvas toolbar
            CanvasToolbar {
                tab_id: active_tab,
                view: view_signal,
                cursor_sample_pos,
                data_version,
            }

            // Three-panel area: Signal List | Canvas | Decoder Config
            div { class: "flex-1 flex overflow-hidden",
                ChannelConfig {
                    tab_id: active_tab,
                    view: view_signal,
                    data_version,
                }
                div { class: "flex-1 overflow-hidden",
                    WaveformCanvas {
                        tab_id: active_tab,
                        data_version,
                        view: view_signal,
                        cursor_sample_pos,
                    }
                }
                div { class: "w-48 flex-shrink-0 border-l border-zinc-800 bg-zinc-900/50 overflow-y-auto p-2",
                    DecoderConfig { view: view_signal }
                }
            }
        }
    }
}
