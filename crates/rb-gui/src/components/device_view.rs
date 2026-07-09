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
    AcquisitionSetup, CanvasToolbar, DecoderSetup, WaveformData, WaveformView,
};

/// The main content view for the active session.
#[component]
pub fn DeviceView(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let Ok(s) = state.try_borrow() else {
        return rsx! { div { class: "flex-1" } };
    };
    let active_tab = s.active_tab;
    let device_id = s.device_id_for_tab(active_tab);
    let connect_error = s.device_manager.connect_error.clone();
    drop(s);

    // No device connected — show placeholder.
    let Some(_device_id) = device_id else {
        return rsx! {
            div { class: "flex-1 flex flex-col items-center justify-center",
                div { class: "text-center space-y-3",
                    div { class: "text-5xl mb-2", "\u{1F50C}" }
                    h2 { class: "text-lg font-bold text-gray-500 dark:text-zinc-400", "No Device" }
                    p { class: "text-xs text-gray-400 dark:text-zinc-600 max-w-sm",
                        "Connect a device from the dropdown above to get started."
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
                            move |_| { crate::app_state::AppState::trigger_scan(&state, data_version, false); }
                        },
                        "\u{27F3}  Scan for Devices"
                    }
                }
            }
        };
    };

    // Device connected — show full view.
    let theme: Signal<crate::app_state::Theme> = use_context();

    // Compute WaveformData from tab's own traces.
    let mut waveform_data = use_signal(WaveformData::empty);
    {
        let Ok(s) = state.try_borrow() else { return prev_frame(); };
        let wd = if let Some(tab) = s.active_tab_state() {
            let la = tab.logic_analyzer();
            WaveformData {
                acq_state: la.acq_state.clone(),
                analog: la.analog.clone(),
                digital: la.digital.clone(),
                sample_count: la.sample_count,
            }
        } else {
            WaveformData::empty()
        };
        waveform_data.set(wd);
    }

    // Per-tab waveform state, decoder config, and acquisition config from the active tab.
    let (wf_state, decoder_cfg, acq_config, version) = {
        let Ok(s) = state.try_borrow() else { return prev_frame(); };
        let tab = s.active_tab_state();
        let la = tab.map(|t| t.logic_analyzer());
        let ws = la.map(|l| l.waveform_state.clone()).unwrap_or_default();
        let dc = la.map(|l| l.decoder_config.clone()).unwrap_or_default();
        let ac = la.map(|l| l.acquisition_config.clone()).unwrap_or_default();
        let ver = la.map(|l| l.content_version).unwrap_or(0);
        (ws, dc, ac, ver)
    };
    let mut wf_state_signal = use_signal(move || wf_state);
    let mut decoder_config_signal = use_signal(move || decoder_cfg);
    let mut config_signal = use_signal(move || acq_config);
    let mut seen_version = use_signal(move || version);
    let mut sample_count_signal = use_signal(|| 0u64);
    let cursor_sample_pos = use_signal(|| None::<u64>);

    // Persist waveform state, decoder config, and acquisition config across tab switches.
    let mut prev_tab = use_signal(|| active_tab);
    if prev_tab() != active_tab {
        // Save current state to the old tab.
        {
            let Ok(mut s) = state.try_borrow_mut() else { return prev_frame(); };
            if let Some(old) = s.tabs.get_mut(&prev_tab()) {
                old.logic_analyzer_mut().waveform_state = wf_state_signal.read().clone();
                old.logic_analyzer_mut().decoder_config = decoder_config_signal.read().clone();
                old.logic_analyzer_mut().acquisition_config = config_signal.read().clone();
            }
        }
        // Load the new tab's saved state.
        let (new_ws, new_dc, new_ac, new_version) = {
            let Ok(s) = state.try_borrow() else { return prev_frame(); };
            let la = s.tabs.get(&active_tab).map(|t| t.logic_analyzer());
            let ws = la.map(|l| l.waveform_state.clone()).unwrap_or_default();
            let dc = la.map(|l| l.decoder_config.clone()).unwrap_or_default();
            let ac = la.map(|l| l.acquisition_config.clone()).unwrap_or_default();
            let ver = la.map(|l| l.content_version).unwrap_or(0);
            (ws, dc, ac, ver)
        };
        wf_state_signal.set(new_ws);
        decoder_config_signal.set(new_dc);
        config_signal.set(new_ac);
        seen_version.set(new_version);
        prev_tab.set(active_tab);
    } else {
        // Same tab — check if content was replaced (device switch).
        let Ok(mut s) = state.try_borrow_mut() else { return prev_frame(); };
        if let Some(active) = s.tabs.get_mut(&active_tab) {
            let current_version = active.logic_analyzer().content_version;
            if current_version != seen_version() {
                // Content was replaced (device switch) — load fresh state.
                wf_state_signal.set(active.logic_analyzer().waveform_state.clone());
                decoder_config_signal.set(active.logic_analyzer().decoder_config.clone());
                config_signal.set(active.logic_analyzer().acquisition_config.clone());
                seen_version.set(current_version);
            } else {
                // Content unchanged — persist UI edits back to tab state.
                active.logic_analyzer_mut().waveform_state = wf_state_signal.read().clone();
                active.logic_analyzer_mut().decoder_config = decoder_config_signal.read().clone();
                active.logic_analyzer_mut().acquisition_config = config_signal.read().clone();
            }
        }
    }

    // Update sample_count from the active tab's traces.
    {
        let Ok(s) = state.try_borrow() else { return prev_frame(); };
        if let Some(tab) = s.active_tab_state() {
            sample_count_signal.set(tab.logic_analyzer().sample_count as u64);
        }
    }

    rsx! {
        div { class: "flex-1 flex flex-col overflow-hidden",
            // Canvas toolbar
            CanvasToolbar {
                tab_id: active_tab,
                wf_state: wf_state_signal,
                cursor_sample_pos,
                data_version,
            }

            // Three-panel area: Signal List | Waveform Display | Decoder Config
            div { class: "flex-1 flex overflow-hidden",
                AcquisitionSetup {
                    config: config_signal,
                    wf_state: wf_state_signal,
                    sample_count: sample_count_signal,
                    on_sample_rate_change: Callback::new(move |hz| {
                        let Ok(mut s) = state.try_borrow_mut() else { return; };
                        if let Some(tab) = s.tabs.get_mut(&active_tab) {
                            tab.logic_analyzer_mut().acquisition_config.sample_rate_hz = hz;
                        }
                    }),
                }
                div { class: "flex-1 overflow-hidden",
                    WaveformView {
                        tab_id: active_tab,
                        data_version,
                        wf_state: wf_state_signal,
                        decoder_config: decoder_config_signal,
                        cursor_sample_pos,
                        waveform_data,
                        theme,
                    }
                }
                div { class: "w-48 flex-shrink-0 border-l border-zinc-800 bg-zinc-900/50 overflow-y-auto p-2",
                    DecoderSetup { decoder_config: decoder_config_signal }
                }
            }
        }
    }
}

/// Minimal placeholder returned when `AppState` is busy (borrowed by `connect_single`).
fn prev_frame() -> Element {
    rsx! { div { class: "flex-1" } }
}
