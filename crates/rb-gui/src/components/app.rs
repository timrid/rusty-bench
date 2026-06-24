//! Root Dioxus component for the RustyBench GUI.
//!
//! Holds [`AppState`] via a shared [`Rc<RefCell<>>`] provided through Dioxus
//! context. A coroutine drives the acquisition polling loop — draining data
//! from background tasks and pumping the executor every ~16ms.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;

use crate::state::AppState;

use super::device_panel::DevicePanel;
use super::main_panel::MainPanel;

/// Tailwind CSS stylesheet, embedded at compile time via Dioxus's `asset!()` macro.
/// Works for both native (embedded) and web (served by trunk).
static TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// Shared application state, accessible by all child components via
/// [`use_context`].
pub type AppStateRef = Rc<RefCell<AppState>>;

/// Dioxus root component. Provides [`AppStateRef`] as context.
/// A coroutine drives the acquisition loop: drain + pump every 16ms.
#[component]
pub fn App() -> Element {
    let state: AppStateRef = use_context_provider(|| Rc::new(RefCell::new(AppState::new())));

    // Signal bumped whenever state changes (data arrives, actions complete).
    let data_version = use_signal(|| 0u64);

    // Coroutine: every 16ms, drain data and pump the executor.
    // Bumps data_version if new data arrived or acquisition is running.
    let state_for_coro = state.clone();
    let mut ver = data_version;
    use_coroutine(move |_rx: UnboundedReceiver<()>| {
        let state = state_for_coro.clone();
        async move {
            loop {
                futures_timer::Delay::new(Duration::from_millis(16)).await;

                let had_data = state.borrow_mut().drain_all();
                state.borrow_mut().pump_once();
                // Process deferred WASM scan/connect results.
                let was_pending = state.borrow().wasm_pending();
                state.borrow_mut().apply_pending_actions();

                if had_data || state.borrow().any_running() || was_pending {
                    ver += 1;
                }
            }
        }
    });

    rsx! {
        // Include Tailwind CSS (works for both desktop and web)
        document::Stylesheet { href: TAILWIND_CSS }
        div { class: "flex h-screen bg-zinc-950 text-zinc-300 font-mono text-sm",
            div { class: "w-64 flex-shrink-0 border-r border-zinc-800 overflow-y-auto p-2",
                DevicePanel { data_version }
            }
            div { class: "flex-1 flex flex-col overflow-hidden",
                MainPanel { data_version }
            }
        }
    }
}
