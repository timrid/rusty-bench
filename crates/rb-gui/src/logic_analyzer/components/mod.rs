//! Dioxus UI components specific to the Logic Analyzer tab.
//!
//! These render the waveform canvas, acquisition-setup sidebar,
//! canvas toolbar, and decoder setup panel.

pub mod acquisition_setup;
pub mod canvas_toolbar;
pub mod decoder_setup;
pub mod interactions;
pub mod waveform_canvas;

pub use acquisition_setup::AcquisitionSetup;
pub use canvas_toolbar::CanvasToolbar;
pub use decoder_setup::DecoderSetup;
pub use waveform_canvas::WaveformCanvas;
