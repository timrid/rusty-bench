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
                            move |_| { crate::app_state::AppState::trigger_scan(&state, data_version); }
                        },
                        "\u{27F3}  Scan for Devices"
                    }
                }
            }
        };
    };

    // Device connected — show full view.
    // Per-tab view state from the active tab.
    let (view, config) = {
        let s = state.borrow();
        let tab = s.active_tab_state();
        let view = tab
            .map(|t| t.logic_analyzer().view.clone())
            .unwrap_or_default();
        let config = tab
            .map(|t| t.logic_analyzer().acquisition_config.clone())
            .unwrap_or_default();
        (view, config)
    };
    let mut view_signal = use_signal(move || view);
    let mut config_signal = use_signal(move || config);
    let mut sample_count_signal = use_signal(|| 0u64);
    let cursor_sample_pos = use_signal(|| None::<u64>);

    // Persist view and config state across tab switches.
    let mut prev_tab = use_signal(|| active_tab);
    if prev_tab() != active_tab {
        // Save current view and config to the old tab.
        {
            let mut s = state.borrow_mut();
            if let Some(old) = s.tabs.get_mut(&prev_tab()) {
                old.logic_analyzer_mut().view = view_signal.read().clone();
                old.logic_analyzer_mut().acquisition_config = config_signal.read().clone();
            }
        }
        // Load the new tab's saved view and config.
        let (new_view, new_config) = {
            let s = state.borrow();
            let v = s.tabs.get(&active_tab)
                .map(|t| t.logic_analyzer().view.clone())
                .unwrap_or_default();
            let c = s.tabs.get(&active_tab)
                .map(|t| t.logic_analyzer().acquisition_config.clone())
                .unwrap_or_default();
            (v, c)
        };
        view_signal.set(new_view);
        config_signal.set(new_config);
        prev_tab.set(active_tab);
    } else {
        // Same tab — sync view and config state back on each render.
        let mut s = state.borrow_mut();
        if let Some(active) = s.tabs.get_mut(&active_tab) {
            active.logic_analyzer_mut().view = view_signal.read().clone();
            active.logic_analyzer_mut().acquisition_config = config_signal.read().clone();
        }
    }

    // Update sample_count from the active tab's acquisition.
    {
        let s = state.borrow();
        if let Some(tab) = s.active_tab_state() {
            let sc = tab.logic_analyzer().acquisition.as_ref()
                .map(|a| a.sample_count() as u64)
                .unwrap_or(0);
            sample_count_signal.set(sc);
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
                    config: config_signal,
                    view: view_signal,
                    sample_count: sample_count_signal,
                    on_sample_rate_change: Callback::new(move |hz| {
                        let mut s = state.borrow_mut();
                        if let Some(tab) = s.tabs.get_mut(&active_tab) {
                            if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
                                acq.send_command(rb_core::AcquisitionCommand::SetSampleRate(hz));
                            }
                        }
                    }),
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
