//! Session view: the main content area for the active session.
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
use super::canvas_toolbar::CanvasToolbar;
use super::decoder_config::DecoderConfig;
use super::signal_list::SignalList;
use super::waveform_canvas::WaveformCanvas;

/// The main content view for the active session.
#[component]
pub fn DeviceView(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let s = state.borrow();
    let active_session = s.active_session;
    let device_id = s.device_id_for_session(active_session);
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
    // Per-session view state from the active session.
    let view = {
        let s = state.borrow();
        s.active_session_state()
            .map(|ss| ss.view.clone())
            .unwrap_or_default()
    };
    let mut view_signal = use_signal(move || view);
    let cursor_sample_pos = use_signal(|| None::<u64>);

    // Persist view state across tab switches.
    // When the active session changes: save the current view to the
    // previous session and load the new session's saved view.
    let mut prev_session = use_signal(|| active_session);
    if prev_session() != active_session {
        // Save current view to the old session.
        {
            let mut s = state.borrow_mut();
            if let Some(old) = s.sessions.get_mut(&prev_session()) {
                old.view = view_signal.read().clone();
            }
        }
        // Load the new session's saved view.
        let new_view = {
            let s = state.borrow();
            s.sessions.get(&active_session)
                .map(|ss| ss.view.clone())
                .unwrap_or_default()
        };
        view_signal.set(new_view);
        prev_session.set(active_session);
    } else {
        // Same session — sync view state back on each render.
        let mut s = state.borrow_mut();
        if let Some(active) = s.sessions.get_mut(&active_session) {
            active.view = view_signal.read().clone();
        }
    }

    rsx! {
        div { class: "flex-1 flex flex-col overflow-hidden",
            // Canvas toolbar
            CanvasToolbar {
                session_id: active_session,
                view: view_signal,
                cursor_sample_pos,
                data_version,
            }

            // Three-panel area: Signal List | Canvas | Decoder Config
            div { class: "flex-1 flex overflow-hidden",
                SignalList {
                    session_id: active_session,
                    view: view_signal,
                    data_version,
                }
                div { class: "flex-1 overflow-hidden",
                    WaveformCanvas {
                        session_id: active_session,
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
