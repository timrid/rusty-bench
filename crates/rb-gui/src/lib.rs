//! Platform-neutral RustyBench GUI built on Dioxus.
//!
//! The same component tree runs natively (via `rb-gui-native` using
//! [`dioxus-desktop`]) and in the browser (via `rb-gui-web` using
//! [`dioxus-web`]).
//!
//! # Architecture
//! 1. [`state::AppState`] owns all device lifecycle, acquisition spawning,
//!    and executor management — framework-agnostic, testable without a display.
//! 2. [`waveform_state::WaveformView`] owns pan/zoom and decoder state per device.
//! 3. [`components`] is the Dioxus component tree that renders the state.

#![forbid(unsafe_code)]

pub mod components;
pub mod device_acquisition;
pub mod device_manager;
pub mod session_state;
pub(crate) mod app_state;
pub mod waveform_state;

pub use app_state::AppState;
pub use device_acquisition::{AcquisitionConfig, DeviceAcquisition};
pub use session_state::{SessionId, SessionState};
pub use waveform_state::WaveformView;
