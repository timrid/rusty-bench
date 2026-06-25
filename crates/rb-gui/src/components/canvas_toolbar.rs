//! Canvas toolbar: Run/Stop, timebase display, zoom, follow toggle.
//! Sits directly above the waveform canvas.

use dioxus::prelude::*;
use rb_core::AcquisitionState;
use crate::waveform_state::WaveformView;

use super::app::AppStateRef;

/// Toolbar above the canvas with acquisition controls and timebase info.
#[component]
pub fn CanvasToolbar(
    device_id: rb_device::DeviceId,
    view: Signal<WaveformView>,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    let (acq_state, sample_count) = {
        let s = state.borrow();
        if let Some(acq) = s.acquisitions.get(&device_id) {
            (acq.state().clone(), acq.sample_count())
        } else if let Some(handle) = s.session.device(&device_id) {
            (handle.state().clone(), handle.sample_count())
        } else {
            (AcquisitionState::Idle, 0)
        }
    };

    let is_running = matches!(acq_state, AcquisitionState::Running);
    let view_samples = view.read().view_samples;

    // Estimate time/div from view window (assuming ~1400px canvas width, ~10 divisions)
    let approx_div_samples = (view_samples as f64 / 10.0).max(1.0);
    let time_div_str = format_samples_to_time(approx_div_samples as u64, 1_000_000.0); // TODO: real sample rate

    rsx! {
        div { class: "flex items-center gap-3 px-3 py-1.5 border-b border-zinc-800 bg-zinc-900/80 flex-shrink-0",
            // Run / Stop
            if is_running {
                button {
                    class: "flex items-center gap-1 px-2 py-1 bg-red-800 hover:bg-red-700 text-red-200 rounded text-xs font-medium transition-colors",
                    title: "Stop acquisition",
                    onclick: {
                        let state = state.clone();
                        let id = device_id.clone();
                        move |_| {
                            state.borrow_mut().stop_blocking(&id);
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
                        let id = device_id.clone();
                        move |_| {
                            state.borrow_mut().start_blocking(&id);
                            data_version.set(data_version() + 1);
                        }
                    },
                    span { class: "text-[10px]", "\u{25B6}" }
                    "Run"
                }
            }

            // Separator
            span { class: "text-zinc-700", "|" }

            // Timebase
            div { class: "flex items-center gap-1 text-xs",
                span { class: "text-zinc-500", "Time/Div:" }
                span { class: "text-zinc-300 font-mono", "{time_div_str}" }
            }

            // Separator
            span { class: "text-zinc-700", "|" }

            // Zoom controls
            div { class: "flex items-center gap-0.5",
                button {
                    class: "px-1.5 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-zinc-300 rounded text-xs transition-colors",
                    title: "Zoom in",
                    onclick: {
                        let mut view = view;
                        move |_| {
                            let mut v = view.write();
                            v.view_samples = (v.view_samples as f64 * 0.5) as usize;
                            if v.view_samples < 16 { v.view_samples = 16; }
                        }
                    },
                    "+"
                }
                button {
                    class: "px-1.5 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-zinc-300 rounded text-xs transition-colors",
                    title: "Zoom out",
                    onclick: {
                        let mut view = view;
                        let max_samples = sample_count;
                        move |_| {
                            let mut v = view.write();
                            v.view_samples = (v.view_samples as f64 * 2.0) as usize;
                            if v.view_samples > max_samples { v.view_samples = max_samples; }
                        }
                    },
                    "\u{2212}"
                }
                button {
                    class: "px-1.5 py-0.5 bg-zinc-800 hover:bg-zinc-700 text-zinc-300 rounded text-xs transition-colors",
                    title: "Reset zoom",
                    onclick: {
                        let mut view = view;
                        let max_samples = sample_count;
                        move |_| {
                            let mut v = view.write();
                            v.view_samples = max_samples;
                            v.view_start = 0;
                        }
                    },
                    "\u{21BA}"
                }
            }

            // Separator
            span { class: "text-zinc-700", "|" }

            // Follow / auto-scroll
            label { class: "flex items-center gap-1 cursor-pointer text-xs text-zinc-400 hover:text-zinc-200 select-none",
                input {
                    r#type: "checkbox",
                    class: "accent-blue-600 w-3 h-3",
                    checked: view.read().auto_scroll,
                    onchange: {
                        let mut view = view;
                        move |evt| {
                            view.write().auto_scroll = evt.checked();
                        }
                    },
                }
                "Follow"
            }

            div { class: "flex-1" }

            // Sample count
            span { class: "text-xs text-zinc-500 font-mono",
                "{sample_count} samples"
            }

            // Status indicator
            match &acq_state {
                AcquisitionState::Running => rsx! {
                    span { class: "text-xs text-green-400 font-medium", "\u{25CF} Running" }
                },
                AcquisitionState::Idle => rsx! {
                    span { class: "text-xs text-zinc-500", "\u{25CB} Idle" }
                },
                AcquisitionState::Stopped => rsx! {
                    span { class: "text-xs text-zinc-500", "\u{25CB} Stopped" }
                },
                AcquisitionState::Error(msg) => rsx! {
                    span { class: "text-xs text-red-400", title: "{msg}", "\u{26A0} Error" }
                },
            }
        }
    }
}

/// Format a sample count as a human-readable time at the given sample rate.
fn format_samples_to_time(samples: u64, rate_hz: f64) -> String {
    if rate_hz <= 0.0 {
        return format!("{samples} samp");
    }
    let seconds = samples as f64 / rate_hz;
    if seconds >= 1.0 {
        format!("{seconds:.2} s")
    } else if seconds >= 1e-3 {
        format!("{:.2} ms", seconds * 1e3)
    } else if seconds >= 1e-6 {
        format!("{:.2} µs", seconds * 1e6)
    } else {
        format!("{:.2} ns", seconds * 1e9)
    }
}
