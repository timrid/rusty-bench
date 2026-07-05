//! Dioxus UI components specific to the Logic Analyzer tab.
//!
//! These render the acquisition-setup sidebar, decoder setup panel,
//! and the waveform display with all its sub-components.

pub mod acquisition_setup;
pub mod decoder_setup;
pub mod waveform_view;

pub use acquisition_setup::AcquisitionSetup;
pub use decoder_setup::DecoderSetup;
pub use waveform_view::{CanvasToolbar, WaveformData, WaveformView};
