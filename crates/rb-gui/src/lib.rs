//! Platform-neutral RustyBench GUI built on Dioxus.
//!
//! The same component tree runs natively (via `rb-gui-native` using
//! [`dioxus-desktop`]) and in the browser (via `rb-gui-web` using
//! [`dioxus-web`]).
//!
//! # Architecture
//! 1. [`app_state::AppState`] owns all device lifecycle, acquisition spawning,
//!    and executor management — framework-agnostic, testable without a display.
//! 2. [`logic_analyzer::waveform_state::WaveformState`] owns pan/zoom,
//!    row layout, and marker state per device.
//! 3. [`components`] is the Dioxus component tree that renders the state.

#![deny(unsafe_code)]

pub mod components;
pub mod device_manager;
pub mod firmware;
pub mod logic_analyzer;
pub mod settings;
pub mod tab_content;
pub mod tab_state;
pub(crate) mod app_state;
pub(crate) mod title_bar;

pub use app_state::AppState;
pub use logic_analyzer::acquisition::AcquisitionConfig;
pub use logic_analyzer::decoder::DecoderConfig;
pub use logic_analyzer::waveform_state::WaveformState;
pub use tab_content::{LogicAnalyzerContent, TabContent};
pub use tab_state::{TabId, TabSource, TabState};
