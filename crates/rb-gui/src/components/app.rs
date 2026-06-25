//! Root Dioxus component for the RustyBench GUI.
//!
//! Layout: SessionSidebar (left, collapsible) | DeviceView (center) | StatusBar (bottom).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;

use crate::state::AppState;

use super::session_sidebar::SessionSidebar;
use super::device_view::DeviceView;

static TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
pub type AppStateRef = Rc<RefCell<AppState>>;

#[component]
pub fn App() -> Element {
    let state: AppStateRef = use_context_provider(|| Rc::new(RefCell::new(AppState::new())));
    let data_version = use_signal(|| 0u64);

    // Coroutine: drain + pump every 16ms.
    let state_for_coro = state.clone();
    let mut ver = data_version;
    use_coroutine(move |_rx: UnboundedReceiver<()>| {
        let state = state_for_coro.clone();
        async move {
            loop {
                futures_timer::Delay::new(Duration::from_millis(16)).await;
                let had_data = state.borrow_mut().drain_all();
                state.borrow_mut().pump_once();
                let was_pending = state.borrow().wasm_pending();
                state.borrow_mut().apply_pending_actions();
                if had_data || state.borrow().any_running() || was_pending {
                    ver += 1;
                }
            }
        }
    });

    // Auto-select first connected device.
    {
        let s = state.borrow();
        let ids = s.connected_device_ids();
        if !ids.is_empty() && s.selected_device.is_none() {
            drop(s);
            state.borrow_mut().selected_device = ids.first().cloned();
        }
    }

    let _version = data_version();

    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }

        div { class: "flex flex-col h-screen bg-zinc-950 text-zinc-300 text-sm",
            // Title bar
            TitleBar { data_version }

            // Main area: sidebar + device view
            div { class: "flex-1 flex overflow-hidden",
                SessionSidebar { data_version }
                DeviceView { data_version }
            }

            // Status bar
            StatusBar { data_version }
        }
    }
}

/// Window title bar with app name and device info.
#[component]
fn TitleBar(data_version: Signal<u64>) -> Element {
    let _version = data_version();
    let state: AppStateRef = use_context();

    let label = state
        .borrow()
        .selected_device
        .as_ref()
        .and_then(|id| {
            let s = state.borrow();
            let lbl = s.device_label(id);
            if lbl.is_empty() { None } else { Some(lbl) }
        });

    rsx! {
        div { class: "h-8 bg-zinc-900 border-b border-zinc-800 flex items-center px-4 flex-shrink-0",
            span { class: "text-xs font-bold text-zinc-300 select-none",
                "RustyBench"
            }
            if let Some(ref device_label) = label {
                span { class: "text-xs text-zinc-600 mx-2", "\u{203A}" }
                span { class: "text-xs text-zinc-400", "{device_label}" }
            }
            div { class: "flex-1" }
            span { class: "text-zinc-700 text-xs select-none", "\u{2014}  \u{25A2}  \u{2715}" }
        }
    }
}

/// Bottom status bar.
#[component]
fn StatusBar(data_version: Signal<u64>) -> Element {
    let _version = data_version();
    let state: AppStateRef = use_context();
    let s = state.borrow();
    let device_count = s.connected_device_ids().len();
    let any_running = s.any_running();
    drop(s);

    let status_text = if any_running {
        format!("\u{25CF} {device_count} device(s)  |  Acquiring")
    } else if device_count > 0 {
        format!("\u{25CF} {device_count} device(s)  |  Idle")
    } else {
        "\u{25CB} No devices  |  Ready".to_string()
    };

    rsx! {
        div { class: "h-6 bg-zinc-900 border-t border-zinc-800 flex items-center px-4 flex-shrink-0",
            span { class: "text-[11px] text-zinc-500 select-none", "{status_text}" }
            div { class: "flex-1" }
            span { class: "text-[11px] text-green-600 font-medium select-none", "RustyBench v0.3.0" }
        }
    }
}
