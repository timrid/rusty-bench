//! Marker bar sub-component: strip below the time ruler showing user-placed
//! time markers as HTML overlays.

use dioxus::prelude::*;
use crate::logic_analyzer::waveform_state::marker::TimeMarker;
use crate::logic_analyzer::waveform_state::row_layout::MARKER_BAR_H;

/// Marker bar showing user-placed time markers as HTML overlays.
#[component]
pub fn MarkerBar(markers: Vec<TimeMarker>, range_start: usize, range_len: f64) -> Element {
    rsx! {
        div {
            class: "relative flex-shrink-0 border-b border-gray-200 dark:border-[#1a1a2e]",
            style: "height: {MARKER_BAR_H}px",
            for m in &markers {
                {
                    let sp = m.sample_pos;
                    let lbl = m.label.clone().unwrap_or_else(|| format!("M{}", m.id));
                    let rs = range_start as u64;
                    let rl = range_len;
                    let pct = if rl > 0.0 {
                        ((sp.saturating_sub(rs)) as f64 / rl * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    };
                    rsx! {
                        div {
                            class: "absolute top-0 bottom-0 flex items-center select-none",
                            style: "left: {pct:.2}%",
                            span { class: "text-[9px] text-amber-400", "\u{25C6}" }
                            span { class: "text-[9px] text-amber-400 ml-0.5", "{lbl}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_to_string(el: Element) -> String {
        dioxus_ssr::render_element(el)
    }

    #[test]
    fn empty_markers() {
        let html = render_to_string(rsx! {
            MarkerBar { markers: vec![], range_start: 0, range_len: 1000.0 }
        });
        // Should render the container div, even without markers.
        assert!(html.contains("relative"), "container should be present");
    }

    #[test]
    fn single_marker_mid() {
        let marker = TimeMarker { id: 1, sample_pos: 500, label: None };
        let html = render_to_string(rsx! {
            MarkerBar { markers: vec![marker], range_start: 0, range_len: 1000.0 }
        });
        assert!(html.contains("M1"), "default label from id");
        assert!(html.contains("left: 50.00%"), "marker at 50% position");
    }

    #[test]
    fn marker_with_custom_label() {
        let marker = TimeMarker { id: 2, sample_pos: 250, label: Some("Start".into()) };
        let html = render_to_string(rsx! {
            MarkerBar { markers: vec![marker], range_start: 0, range_len: 1000.0 }
        });
        assert!(html.contains("Start"), "custom label present");
        assert!(!html.contains("M2"), "default label suppressed when custom label set");
    }

    #[test]
    fn marker_clamped_to_range() {
        // sample_pos > range_end → should clamp to 100%
        let marker = TimeMarker { id: 1, sample_pos: 2000, label: None };
        let html = render_to_string(rsx! {
            MarkerBar { markers: vec![marker], range_start: 0, range_len: 1000.0 }
        });
        assert!(html.contains("left: 100.00%"), "clamped to 100%");
    }

    #[test]
    fn marker_before_range() {
        // sample_pos < range_start → should clamp to 0%
        let marker = TimeMarker { id: 1, sample_pos: 0, label: None };
        let html = render_to_string(rsx! {
            MarkerBar { markers: vec![marker], range_start: 500, range_len: 1000.0 }
        });
        assert!(html.contains("left: 0.00%"), "clamped to 0%");
    }
}
