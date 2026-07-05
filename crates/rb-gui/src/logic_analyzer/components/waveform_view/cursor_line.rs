//! Cursor line sub-component: vertical dashed line that follows the mouse.

use dioxus::prelude::*;

/// Vertical cursor line with time label, shown on mouse hover.
#[component]
pub fn CursorLine(cursor_px: Signal<Option<f64>>, cursor_label: Signal<String>) -> Element {
    let px = cursor_px();
    let label = cursor_label();
    rsx! {
        if let Some(px) = px {
            div {
                class: "pointer-events-none absolute top-0 bottom-0 z-20",
                style: "left: {px}px",
                div {
                    class: "absolute top-0 bottom-0 border-l border-dashed border-[#111827] dark:border-[#f0f6fc] opacity-70"
                }
                div {
                    class: "absolute text-[9px] whitespace-nowrap px-1 rounded bg-gray-100/70 dark:bg-[#0d1117aa] text-[#111827] dark:text-[#f0f6fc]",
                    style: "top: 0; left: 4px",
                    "{label}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;

    /// Wrapper that creates signals via use_signal (inside a runtime) and
    /// passes them to CursorLine, so we can SSR-render the result.
    fn render_cursor(px: Option<f64>, label: &str) -> String {
        let label = label.to_string();
        let mut vdom = VirtualDom::new_with_props(
            move || {
                let cursor_px = use_signal(|| px);
                let cursor_label = use_signal(|| label.clone());
                rsx! {
                    super::CursorLine { cursor_px, cursor_label }
                }
            },
            (),
        );
        vdom.rebuild_in_place();
        dioxus_ssr::render(&vdom)
    }

    #[test]
    fn cursor_visible_when_some() {
        let html = render_cursor(Some(150.0), "1.500ms");
        assert!(html.contains("1.500ms"), "label should be visible, got: {html}");
        assert!(html.contains("left: 150px"), "cursor at 150px, got: {html}");
    }

    #[test]
    fn cursor_hidden_when_none() {
        let html = render_cursor(None, "");
        assert!(!html.contains("border-dashed"), "no cursor line, got: {html}");
    }

    #[test]
    fn cursor_updates_with_signal() {
        let html = render_cursor(Some(42.0), "42ns");
        assert!(html.contains("42ns"), "label 42ns, got: {html}");
        assert!(html.contains("left: 42px"), "cursor at 42px, got: {html}");
    }
}
