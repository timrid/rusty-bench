//! Modal confirmation dialog.
//!
//! Uses a global [`Signal`] so any component can trigger a confirmation prompt
//! without threading state through every prop.

use dioxus::prelude::*;
use rb_device::DeviceId;

use super::app::AppStateRef;

/// A pending confirmation dialog.
#[derive(Clone)]
pub struct DialogConfig {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub cancel_label: String,
    /// If set, clicking Confirm switches the active tab to this device.
    pub switch_to_device: Option<DeviceId>,
}

impl DialogConfig {
    /// Convenience constructor for a device-switch warning.
    pub fn device_switch(target_did: DeviceId) -> Self {
        Self {
            title: "Switch device".into(),
            message: "Switching devices will discard all recorded data. Are you sure?".into(),
            confirm_label: "Switch".into(),
            cancel_label: "Cancel".into(),
            switch_to_device: Some(target_did),
        }
    }
}

/// Global dialog signal — set to `Some(config)` to show, `None` to hide.
pub static DIALOG: GlobalSignal<Option<DialogConfig>> = Signal::global(|| None);

/// Renders a modal overlay when [`DIALOG`] is `Some`.
#[component]
pub fn Dialog(data_version: Signal<u64>) -> Element {
    let state: AppStateRef = use_context();
    let dialog = DIALOG();

    let Some(cfg) = dialog.as_ref().cloned() else {
        return rsx! {};
    };

    let title = cfg.title.clone();
    let message = cfg.message.clone();
    let confirm_label = cfg.confirm_label.clone();
    let cancel_label = cfg.cancel_label.clone();
    let switch_to_device = cfg.switch_to_device.clone();

    rsx! {
        // Backdrop
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/60",
            onclick: move |_| {
                *DIALOG.write() = None;
            },

            // Modal card
            div {
                class: "bg-white border border-gray-200 dark:bg-zinc-800 dark:border-zinc-700 rounded-lg shadow-xl w-96 p-5",
                onclick: move |evt| evt.stop_propagation(),

                h2 { class: "text-sm font-semibold text-gray-800 dark:text-zinc-100 mb-2", "{title}" }
                p { class: "text-xs text-gray-500 dark:text-zinc-400 leading-relaxed mb-5", "{message}" }

                div { class: "flex justify-end gap-2",
                    button {
                        class: "px-3 py-1.5 text-xs text-gray-500 hover:text-gray-700 bg-gray-100 hover:bg-gray-200 dark:text-zinc-400 dark:hover:text-zinc-200 dark:bg-zinc-700 dark:hover:bg-zinc-600 rounded transition-colors",
                        onclick: move |_| {
                            *DIALOG.write() = None;
                        },
                        "{cancel_label}"
                    }
                    button {
                        class: "px-3 py-1.5 text-xs text-white bg-blue-600 hover:bg-blue-500 rounded transition-colors",
                        onclick: move |_| {
                            if let Some(ref target_did) = switch_to_device {
                                let mut s = state.borrow_mut();
                                let tab_id = s.active_tab;
                                s.assign_device_to_tab(tab_id, target_did.clone());
                                // Clear stale traces from the device handle
                                // so the WaveformView shows fresh channels.
                                if let Some(h) = s.device_manager.device_handle_mut(target_did) {
                                    h.discard_samples();
                                    let content = crate::logic_analyzer::init_content(h.device());
                                    if let Some(tab) = s.tabs.get_mut(&tab_id) {
                                        tab.content = Some(crate::tab_content::TabContent::LogicAnalyzer(content));
                                    }
                                }
                                data_version += 1;
                            }
                            *DIALOG.write() = None;
                        },
                        "{confirm_label}"
                    }
                }
            }
        }
    }
}
