//! Root Dioxus component for the RustyBench GUI.
//!
//! Layout: TopBar | SessionView (center) | StatusBar (bottom).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;

use crate::state::AppState;

use super::device_view::DeviceView;
use super::top_bar::TopBar;

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

    let _version = data_version();

    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }

        div { class: "flex flex-col h-screen bg-zinc-950 text-zinc-300 text-sm",
            // Top bar: device dropdown + tab bar + settings
            TopBar { data_version }

            // Main area: session content (full width)
            DeviceView { data_version }

            // Status bar
            StatusBar { data_version }
        }
    }
}

/// Bottom status bar.
#[component]
fn StatusBar(data_version: Signal<u64>) -> Element {
    let _version = data_version();
    let state: AppStateRef = use_context();
    let s = state.borrow();
    let session_count = s.sessions.len();
    let any_running = s.any_running();
    let active_label = s
        .active_session_state()
        .map(|ss| ss.label.clone())
        .filter(|l| l != "Untitled")
        .unwrap_or_default();
    drop(s);

    let status_text = if any_running {
        format!("\u{25CF} {session_count} session(s)  |  Acquiring")
    } else if session_count > 0 && !active_label.is_empty() {
        format!("\u{25CF} {session_count} session(s)  |  {active_label}  |  Idle")
    } else if session_count > 0 {
        format!("\u{25CF} {session_count} session(s)  |  Idle")
    } else {
        "\u{25CB} No sessions  |  Ready".to_string()
    };

    rsx! {
        div { class: "h-6 bg-zinc-900 border-t border-zinc-800 flex items-center px-4 flex-shrink-0",
            span { class: "text-[11px] text-zinc-500 select-none", "{status_text}" }
            div { class: "flex-1" }
            span { class: "text-[11px] text-green-600 font-medium select-none", "RustyBench v0.3.0" }
        }
    }
}
