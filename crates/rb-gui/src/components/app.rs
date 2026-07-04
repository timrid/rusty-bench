//! Root Dioxus component for the RustyBench GUI.
//!
//! Layout: TopBar | SessionView (center) | StatusBar (bottom).

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

use crate::app_state::{AppState, Theme};

use super::device_view::DeviceView;
use super::dialog::Dialog;
use super::top_bar::TopBar;

static TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
pub type AppStateRef = Rc<RefCell<AppState>>;

#[component]
pub fn App() -> Element {
    let _state: AppStateRef = use_context_provider(|| Rc::new(RefCell::new(AppState::new())));
    let data_version = use_signal(|| 0u64);
    let _version = data_version();

    // ── Theme ───────────────────────────────────────────────────────────
    let theme: Signal<Theme> = use_context_provider(|| Signal::new(Theme::System));
    let _theme = theme();

    // ── Desktop: event-driven device discovery via hotplug ──────────────
    let mut spawned_hotplug = use_signal(|| false);
    if !spawned_hotplug() {
        spawned_hotplug.set(true);
        AppState::spawn_usb_hotplug_watch(&_state, data_version);
    }

    // Apply/remove the `dark` class on <html> whenever the theme changes.
    use_effect(move || {
        let t = theme();
        let state = _state.clone();

        // ── Native title bar (Windows) ──────────────────────────────────
        #[cfg(target_os = "windows")]
        match t {
            Theme::Dark => crate::title_bar::set_title_bar_theme(true),
            Theme::Light | Theme::System => crate::title_bar::set_title_bar_theme(false),
        }
        #[cfg(not(target_os = "windows"))]
        let _ = t;

        let eval = document::eval(
            match t {
                Theme::System => {
                    r#"const m=window.matchMedia('(prefers-color-scheme:dark)');
                       document.documentElement.classList.toggle('dark',m.matches);
                       m.onchange=e=>document.documentElement.classList.toggle('dark',e.matches);"#
                }
                Theme::Light => r#"document.documentElement.classList.remove('dark')"#,
                Theme::Dark => r#"document.documentElement.classList.add('dark')"#,
            }
        );
        state.borrow_mut().theme = t;
        spawn(async move { let _ = eval.await; });
    });

    rsx! {
        document::Stylesheet { href: TAILWIND_CSS }

        div {
            class: if theme() == Theme::Dark {
                "flex flex-col h-screen bg-white text-gray-800 dark:bg-zinc-950 dark:text-zinc-300 text-sm dark"
            } else {
                "flex flex-col h-screen bg-white text-gray-800 dark:bg-zinc-950 dark:text-zinc-300 text-sm"
            },
            // Top bar: device dropdown + tab bar + settings
            TopBar { data_version }

            // Main area: session content (full width)
            DeviceView { data_version }

            // Status bar
            StatusBar { data_version }

            // Modal dialogs (rendered on top of everything)
            Dialog { data_version }
        }
    }
}

/// Bottom status bar.
#[component]
fn StatusBar(data_version: Signal<u64>) -> Element {
    let _version = data_version();
    let state: AppStateRef = use_context();
    let s = state.borrow();
    let tab_count = s.tabs.len();
    let any_running = s.any_running();
    let active_label = s
        .active_tab_state()
        .map(|t| t.label.clone())
        .filter(|l| l != "Untitled")
        .unwrap_or_default();
    drop(s);

    let status_text = if any_running {
        format!("\u{25CF} {tab_count} tab(s)  |  Acquiring")
    } else if tab_count > 0 && !active_label.is_empty() {
        format!("\u{25CF} {tab_count} tab(s)  |  {active_label}  |  Idle")
    } else if tab_count > 0 {
        format!("\u{25CF} {tab_count} tab(s)  |  Idle")
    } else {
        "\u{25CB} No tabs  |  Ready".to_string()
    };

    rsx! {
        div { class: "h-6 bg-gray-100 border-t border-gray-200 dark:bg-zinc-900 dark:border-t dark:border-zinc-800 flex items-center px-4 flex-shrink-0",
            span { class: "text-[11px] text-gray-500 dark:text-zinc-500 select-none", "{status_text}" }
            div { class: "flex-1" }
            span { class: "text-[11px] text-green-600 font-medium select-none", "RustyBench v0.3.0" }
        }
    }
}
