//! Dioxus UI components specific to the Logic Analyzer tab.
//!
//! These render the waveform canvas, channel-config sidebar,
//! canvas toolbar, and decoder config panel.

pub mod canvas_toolbar;
pub mod channel_config;
pub mod decoder_config;
pub mod interactions;
pub mod waveform_canvas;

pub use canvas_toolbar::CanvasToolbar;
pub use channel_config::ChannelConfig;
pub use decoder_config::DecoderConfig;
pub use waveform_canvas::WaveformCanvas;
