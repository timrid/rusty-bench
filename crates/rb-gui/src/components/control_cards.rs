//! Control cards for non-time-based device capabilities.
//!
//! Rendered below the canvas area. Each card represents one capability
//! that doesn't produce a time-domain signal (Power Supply, Multimeter,
//! Waveform Generator, Electronic Load).

use dioxus::prelude::*;

use super::app::AppStateRef;

/// Renders control cards for a device's non-timeline capabilities.
#[component]
pub fn ControlCards(
    device_id: rb_device::DeviceId,
    data_version: Signal<u64>,
) -> Element {
    let state: AppStateRef = use_context();
    let _version = data_version();

    // Determine which non-time capabilities this device has.
    let s = state.borrow();
    let classes = s
        .session
        .device(&device_id)
        .map(|h| h.device().classes())
        .unwrap_or_default();
    drop(s);

    // For now, render placeholder cards for each non-timeline class.
    // Analog (Oscilloscope) and Digital (LogicAnalyzer) are rendered on the canvas.
    let cards: Vec<_> = classes
        .iter()
        .filter(|c| {
            !matches!(
                c,
                rb_device::DeviceClass::Oscilloscope | rb_device::DeviceClass::LogicAnalyzer
            )
        })
        .collect();

    if cards.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "border-t border-zinc-800 bg-zinc-900/50 px-3 py-2 flex-shrink-0",
            div { class: "flex gap-3 overflow-x-auto",
                for class in &cards {
                    ControlCard { class: **class }
                }
            }
        }
    }
}

/// A single control card for one capability.
#[component]
fn ControlCard(class: rb_device::DeviceClass) -> Element {
    let (label, icon, color) = match class {
        rb_device::DeviceClass::PowerSupply => ("Power Supply", "\u{1F50B}", "bg-green-900/40 border-green-700 text-green-300"),
        rb_device::DeviceClass::Multimeter => ("Multimeter", "\u{1F522}", "bg-cyan-900/40 border-cyan-700 text-cyan-300"),
        rb_device::DeviceClass::WaveformGenerator => ("Waveform Gen", "\u{223F}", "bg-purple-900/40 border-purple-700 text-purple-300"),
        rb_device::DeviceClass::ElectronicLoad => ("Electronic Load", "\u{26A1}", "bg-orange-900/40 border-orange-700 text-orange-300"),
        rb_device::DeviceClass::SpectrumAnalyzer => ("Spectrum Anlz", "\u{1F4CA}", "bg-sky-900/40 border-sky-700 text-sky-300"),
        rb_device::DeviceClass::SdrReceiver => ("SDR Receiver", "\u{1F4E1}", "bg-pink-900/40 border-pink-700 text-pink-300"),
        _ => return rsx! {},
    };

    rsx! {
        div { class: "rounded-lg border {color} p-3 min-w-[160px] flex-shrink-0",
            div { class: "flex items-center gap-1.5 mb-2",
                span { class: "text-sm", "{icon}" }
                span { class: "text-xs font-medium text-zinc-300", "{label}" }
            }
            div { class: "text-center py-2",
                p { class: "text-[10px] text-zinc-500 italic", "Controls coming soon" }
            }
        }
    }
}
