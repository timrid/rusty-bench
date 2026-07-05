//! Time ruler sub-component: sticky header with adaptively scaled tick marks.
//! Rendered as HTML elements (not canvas).

use dioxus::prelude::*;
use crate::logic_analyzer::waveform_state::row_layout::TIME_RULER_H;

/// Time ruler showing major and minor ticks as HTML elements.
#[component]
pub fn TimeRuler(tick_elements: Vec<(f64, String, Vec<f64>)>) -> Element {
    rsx! {
        div {
            class: "w-full flex-shrink-0 relative bg-gray-100 border-b border-gray-200 dark:bg-[#0a0e14] dark:border-b dark:border-[#30363d] select-none",
            style: "height: {TIME_RULER_H}px",
            for (pct, label, minor_pcts) in &tick_elements {
                // Major tick
                div {
                    class: "absolute top-1/2 bottom-0 border-l border-gray-300 dark:border-[#30363d]",
                    style: "left: {pct:.2}%"
                }
                span {
                    class: "absolute text-[9px] text-gray-400 dark:text-[#8b949e]",
                    style: "left: calc({pct:.2}% + 2px); top: 0",
                    "{label}"
                }
                // Minor ticks
                for mpct in minor_pcts {
                    div {
                        class: "absolute top-[70%] bottom-0 border-l border-gray-300 dark:border-[#30363d] opacity-50",
                        style: "left: {mpct:.2}%"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: render a Dioxus Element to an HTML string via dioxus-ssr.
    fn render_to_string(el: Element) -> String {
        dioxus_ssr::render_element(el)
    }

    #[test]
    fn empty_ticks() {
        let html = render_to_string(rsx! { TimeRuler { tick_elements: vec![] } });
        assert!(html.contains("height"), "should render container div");
    }

    #[test]
    fn single_tick() {
        let ticks = vec![(50.0f64, "1ms".to_string(), vec![])];
        let html = render_to_string(rsx! { TimeRuler { tick_elements: ticks } });
        assert!(html.contains("1ms"), "should contain tick label");
        assert!(html.contains("left: 50.00%"), "should position at 50%");
    }

    #[test]
    fn tick_with_minors() {
        let ticks = vec![(
            25.0f64,
            "500µs".to_string(),
            vec![12.5f64, 18.75f64],
        )];
        let html = render_to_string(rsx! { TimeRuler { tick_elements: ticks } });
        assert!(html.contains("500µs"), "major label present");
        assert!(html.contains("left: 12.50%"), "first minor at 12.5%");
        assert!(html.contains("left: 18.75%"), "second minor at 18.75%");
    }

    #[test]
    fn multiple_ticks() {
        let ticks = vec![
            (0.0f64, "0s".to_string(), vec![]),
            (50.0f64, "500ms".to_string(), vec![]),
            (100.0f64, "1s".to_string(), vec![]),
        ];
        let html = render_to_string(rsx! { TimeRuler { tick_elements: ticks } });
        assert!(html.contains("0s"));
        assert!(html.contains("500ms"));
        assert!(html.contains("1s"));
    }
}
