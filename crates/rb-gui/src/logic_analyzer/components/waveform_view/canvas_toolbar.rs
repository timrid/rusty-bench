//! Canvas toolbar: Run/Stop, marker controls.
//! Sits directly above the waveform display.

use dioxus::prelude::*;
use rb_core::AcquisitionState;

use crate::logic_analyzer::control;
use crate::logic_analyzer::waveform_state::WaveformState;
use crate::components::app::AppStateRef;

/// Toolbar above the waveform view with acquisition controls and marker buttons.
#[component]
pub fn CanvasToolbar(
    tab_id: crate::tab_state::TabId,
    wf_state: Signal<WaveformState>,
    cursor_sample_pos: Signal<Option<u64>>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (acq_state, _sample_count) = {
        let s = state.borrow();
        if let Some(acq) = control::acq_for_tab(&s, tab_id) {
            (acq.state().clone(), acq.sample_count())
        } else if let Some(handle) = s.handle_for_tab(tab_id) {
            (handle.state().clone(), handle.sample_count())
        } else {
            (AcquisitionState::Idle, 0)
        }
    };

    let is_running = matches!(acq_state, AcquisitionState::Running);

    rsx! {
        div { class: "flex items-center gap-2 px-3 py-1.5 border-b border-gray-200 bg-gray-50/80 dark:border-zinc-800 dark:bg-zinc-900/80 flex-shrink-0",
            // Run / Stop
            if is_running {
                button {
                    class: "flex items-center gap-1 px-2 py-1 bg-red-800 hover:bg-red-700 text-red-200 rounded text-xs font-medium transition-colors",
                    title: "Stop acquisition (device stays connected)",
                    onclick: {
                        let state = state.clone();
                        let tid = tab_id;
                        move |_| {
                            control::stop(&mut *state.borrow_mut(), tid);
                            data_version += 1;
                        }
                    },
                    span { class: "text-[10px]", "\u{23F9}" }
                    "Stop"
                }
            } else {
                button {
                    class: "flex items-center gap-1 px-2 py-1 bg-green-800 hover:bg-green-700 text-green-200 rounded text-xs font-medium transition-colors",
                    title: "Start acquisition",
                    onclick: {
                        let state = state.clone();
                        let tid = tab_id;
                        move |_| {
                            control::start(&state, tid, data_version);
                        }
                    },
                    span { class: "text-[10px]", "\u{25B6}" }
                    "Run"
                }
            }

            span { class: "text-gray-300 dark:text-zinc-700", "|" }

            // Marker A button
            button {
                class: "px-2 py-0.5 bg-gray-200 hover:bg-gray-300 dark:bg-zinc-800 dark:hover:bg-zinc-700 text-amber-600 dark:text-amber-400 rounded text-xs font-medium transition-colors",
                title: "Add Marker A at cursor position",
                onclick: {
                    let mut wf_state = wf_state;
                    let cursor_sample_pos = cursor_sample_pos;
                    move |_| {
                        if let Some(pos) = cursor_sample_pos() {
                            wf_state.write().marker_set.add_marker(pos);
                        }
                    }
                },
                "\u{25C6} M"
            }
        }
    }
}
