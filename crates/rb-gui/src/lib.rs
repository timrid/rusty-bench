//! Platform-neutral RustyBench GUI built on Dioxus.
//!
//! The same component tree runs natively (via `rb-gui-native` using
//! [`dioxus-desktop`]) and in the browser (via `rb-gui-web` using
//! [`dioxus-web`]).
//!
//! # Architecture
//! 1. [`state::AppState`] owns all device lifecycle, acquisition spawning,
//!    and executor management — framework-agnostic, testable without a display.
//! 2. [`logic_analyzer::view::WaveformView`] owns pan/zoom and decoder state per device.
//! 3. [`components`] is the Dioxus component tree that renders the state.

#![forbid(unsafe_code)]

pub mod components;
pub mod device_manager;
pub mod firmware;
pub mod logic_analyzer;
pub mod tab_content;
pub mod tab_state;
pub(crate) mod app_state;

pub use app_state::AppState;
pub use logic_analyzer::acquisition::{AcquisitionConfig, DeviceAcquisition};
pub use logic_analyzer::view::WaveformView;
pub use tab_content::{LogicAnalyzerContent, TabContent};
pub use tab_state::{TabId, TabSource, TabState};
