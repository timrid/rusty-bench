//! Settings dialog — opens when the user clicks the gear icon.
//!
//! Uses the persistent [`AppSettings`](crate::settings::AppSettings) signal.
//! The theme toggle button in the top bar is a shortcut for the theme setting here.
//!
//! The dialog keeps a local working copy. Changes are only written to the
//! persistent signal on **Apply** or **Save**. **Cancel** discards changes.

use dioxus::prelude::*;
use crate::app_state::Theme;
use crate::settings::AppSettings;

/// Global signal: `None` = closed, `Some` = open.
pub static SETTINGS_OPEN: GlobalSignal<Option<()>> = Signal::global(|| None);

/// Open the settings dialog.
pub fn open_settings() {
    *SETTINGS_OPEN.write() = Some(());
}

/// Close the settings dialog.
fn close_settings() {
    *SETTINGS_OPEN.write() = None;
}

// ── Component ────────────────────────────────────────────────────────────────

#[component]
pub fn SettingsDialog() -> Element {
    let open = SETTINGS_OPEN();
    let persistent: Signal<AppSettings> = use_context();

    // Local working copy. Reset from persistent each time the dialog opens.
    let mut local = use_signal(|| persistent().clone());
    let mut last_open = use_signal(|| false);
    if open.is_some() && !last_open() {
        last_open.set(true);
        local.set(persistent().clone());
    } else if open.is_none() {
        last_open.set(false);
    }

    if open.is_none() {
        return rsx! {};
    }

    // ── Actions ──────────────────────────────────────────────────────
    let do_cancel = move || {
        close_settings();
    };

    let mut persistent_sig = persistent;
    let mut do_apply = move || {
        persistent_sig.write().theme = local().theme;
    };

    let mut do_save = move || {
        persistent_sig.write().theme = local().theme;
        close_settings();
    };

    rsx! {
        // Backdrop (click-through blocked, no action — use buttons to close)
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/60",

            // Modal card
            div {
                class: "bg-white border border-gray-200 dark:bg-zinc-800 dark:border-zinc-700 rounded-lg shadow-xl w-96 p-5",
                onclick: move |evt| evt.stop_propagation(),

                h2 { class: "text-sm font-semibold text-gray-800 dark:text-zinc-100 mb-4", "Settings" }

                // ── Theme ──────────────────────────────────────────────
                div { class: "mb-4",
                    label {
                        class: "block text-xs font-medium text-gray-500 dark:text-zinc-400 mb-1",
                        "Theme"
                    }
                    select {
                        class: "w-full px-2 py-1.5 text-xs bg-gray-100 border border-gray-300 dark:bg-zinc-700 dark:border-zinc-600 dark:text-zinc-200 rounded focus:outline-none focus:ring-1 focus:ring-blue-500",
                        value: match local().theme {
                            Theme::System => "system",
                            Theme::Light => "light",
                            Theme::Dark => "dark",
                        },
                        onchange: move |evt| {
                            let theme = match evt.value().as_str() {
                                "light" => Theme::Light,
                                "dark" => Theme::Dark,
                                _ => Theme::System,
                            };
                            local.write().theme = theme;
                        },
                        option { value: "system", "System (follow OS)" }
                        option { value: "light", "Light" }
                        option { value: "dark", "Dark" }
                    }
                }

                // ── Footer ─────────────────────────────────────────────
                div { class: "flex justify-end gap-2",
                    button {
                        class: "px-3 py-1.5 text-xs text-gray-500 hover:text-gray-700 bg-gray-100 hover:bg-gray-200 dark:text-zinc-400 dark:hover:text-zinc-200 dark:bg-zinc-700 dark:hover:bg-zinc-600 rounded transition-colors",
                        onclick: move |_| do_cancel(),
                        "Cancel"
                    }
                    button {
                        class: "px-3 py-1.5 text-xs text-gray-500 hover:text-gray-700 bg-gray-100 hover:bg-gray-200 dark:text-zinc-400 dark:hover:text-zinc-200 dark:bg-zinc-700 dark:hover:bg-zinc-600 rounded transition-colors",
                        onclick: move |_| do_apply(),
                        "Apply"
                    }
                    button {
                        class: "px-3 py-1.5 text-xs text-white bg-blue-600 hover:bg-blue-500 rounded transition-colors",
                        onclick: move |_| do_save(),
                        "Save"
                    }
                }
            }
        }
    }
}
