//! Per-device waveform view state: pan/zoom window and decoder management.
//!
//! [`WaveformView`] holds the visible sample window (pan/zoom state) for one
//! connected device and manages the protocol-decoder lifecycle. Drawing is
//! handled by the canvas component in [`super::components::waveform_canvas`].
//!
//! # Pan / zoom
//! - Scroll wheel over the panel: zoom in/out around the view centre.
//! - Drag: pan left/right.
//! - "Follow" checkbox: auto-scrolls to the newest samples while running.

use std::ops::Range;

use rb_core::DeviceHandle;
use rb_decode::{Annotation, Decoder, I2cConfig, I2cDecoder, SpiConfig, SpiDecoder, UartConfig, UartDecoder};
use rb_model::DigitalTrace;

// ── Decoder kind selector ─────────────────────────────────────────────────────

/// Which protocol decoder (if any) is attached to this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DecoderKind {
    #[default]
    None,
    Uart,
    I2c,
    Spi,
}

impl DecoderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Uart => "UART",
            Self::I2c => "I²C",
            Self::Spi => "SPI",
        }
    }
}

// ── View state ────────────────────────────────────────────────────────────────

/// Pan/zoom and optional decoder state for one device's waveform display.
///
/// Manual Clone impl because `Box<dyn Decoder>` isn't Clone — the decoder
/// is rebuilt on demand after cloning.
pub struct WaveformView {
    /// Index of the first visible sample.
    pub view_start: usize,
    /// Number of samples in the visible window (controls zoom level).
    pub view_samples: usize,
    /// When `true`, the view tracks the newest data while the device is running.
    pub auto_scroll: bool,

    // ── Decoder state ─────────────────────────────────────────────────────────
    pub decoder_kind: DecoderKind,
    /// Rebuilt on demand; not Clone so we reconstruct from config.
    #[allow(clippy::type_complexity)]
    decoder: Option<Box<dyn Decoder>>,
    pub annotations: Vec<Annotation>,
    /// How many digital-store words have been fed to the decoder so far.
    decoded_up_to: usize,
    /// When `true`, the decoder is rebuilt and all annotations are cleared on
    /// the next update (triggered by kind or config changes).
    pub decoder_dirty: bool,

    // ── Per-decoder config ────────────────────────────────────────────────────
    pub uart_baud: u32,
    pub uart_rx_bit: u8,
    pub i2c_scl_bit: u8,
    pub i2c_sda_bit: u8,
    pub spi_mode: u8,
    pub spi_clk_bit: u8,
    pub spi_mosi_bit: u8,
    pub spi_miso_bit: u8,
    pub spi_cs_bit: u8,
}

// Manual Clone because `Box<dyn Decoder>` isn't Clone.
impl Clone for WaveformView {
    fn clone(&self) -> Self {
        Self {
            view_start: self.view_start,
            view_samples: self.view_samples,
            auto_scroll: self.auto_scroll,
            decoder_kind: self.decoder_kind,
            decoder: None, // rebuilt on demand
            annotations: self.annotations.clone(),
            decoded_up_to: 0,
            decoder_dirty: true, // force rebuild
            uart_baud: self.uart_baud,
            uart_rx_bit: self.uart_rx_bit,
            i2c_scl_bit: self.i2c_scl_bit,
            i2c_sda_bit: self.i2c_sda_bit,
            spi_mode: self.spi_mode,
            spi_clk_bit: self.spi_clk_bit,
            spi_mosi_bit: self.spi_mosi_bit,
            spi_miso_bit: self.spi_miso_bit,
            spi_cs_bit: self.spi_cs_bit,
        }
    }
}

impl Default for WaveformView {
    fn default() -> Self {
        Self {
            view_start: 0,
            view_samples: 1_000,
            auto_scroll: true,
            decoder_kind: DecoderKind::None,
            decoder: None,
            annotations: Vec::new(),
            decoded_up_to: 0,
            decoder_dirty: false,
            uart_baud: 115_200,
            uart_rx_bit: 0,
            i2c_scl_bit: 0,
            i2c_sda_bit: 1,
            spi_mode: 0,
            spi_clk_bit: 0,
            spi_mosi_bit: 1,
            spi_miso_bit: 2,
            spi_cs_bit: 3,
        }
    }
}

impl WaveformView {
    /// Rebuilds the decoder from the current kind + config, clearing cached
    /// annotations. Called when the user changes decoder kind or parameters.
    pub fn rebuild_decoder(&mut self) {
        self.decoder = match self.decoder_kind {
            DecoderKind::None => None,
            DecoderKind::Uart => Some(Box::new(UartDecoder::new(UartConfig {
                rx_bit: self.uart_rx_bit,
                baud_rate: self.uart_baud,
                ..Default::default()
            }))),
            DecoderKind::I2c => Some(Box::new(I2cDecoder::new(I2cConfig {
                scl_bit: self.i2c_scl_bit,
                sda_bit: self.i2c_sda_bit,
            }))),
            DecoderKind::Spi => Some(Box::new(SpiDecoder::new(SpiConfig {
                clk_bit: self.spi_clk_bit,
                mosi_bit: self.spi_mosi_bit,
                miso_bit: self.spi_miso_bit,
                cs_bit: self.spi_cs_bit,
                mode: self.spi_mode,
                ..Default::default()
            }))),
        };
        self.annotations.clear();
        self.decoded_up_to = 0;
        self.decoder_dirty = false;
    }

    /// Feed new digital samples to the decoder and return any new annotations.
    /// Call this before reading `self.annotations`.
    pub fn feed_decoder(&mut self, dt: &DigitalTrace) {
        if self.decoder_dirty {
            self.rebuild_decoder();
        }
        if let Some(dec) = &mut self.decoder {
            let words = dt.store().words();
            let rate = dt.timebase().sample_rate_hz();
            if self.decoded_up_to < words.len() {
                let new_anns = dec.feed(&words[self.decoded_up_to..], self.decoded_up_to, rate);
                self.annotations.extend(new_anns);
                self.decoded_up_to = words.len();
            }
        }
    }

    /// Clamp the view window to valid bounds and advance if auto-scrolling.
    /// Returns the visible sample range `[start, end)`.
    pub fn clamp_view(&mut self, sample_count: usize, is_running: bool) -> Range<usize> {
        if sample_count == 0 {
            self.view_start = 0;
            return 0..0;
        }
        self.view_samples = self.view_samples.clamp(16, sample_count);
        if self.auto_scroll && is_running {
            self.view_start = sample_count.saturating_sub(self.view_samples);
        }
        self.view_start = self
            .view_start
            .min(sample_count.saturating_sub(self.view_samples));
        let view_end = (self.view_start + self.view_samples).min(sample_count);
        self.view_start..view_end
    }

    /// Update pan: delta in pixels (positive = drag right, which pans left into
    /// older samples). `canvas_width` is the width of the drawing area in pixels.
    pub fn pan(&mut self, delta_px: f32, canvas_width: f32, sample_count: usize) {
        if sample_count == 0 || delta_px.abs() < 0.5 {
            return;
        }
        let spx = self.view_samples as f32 / canvas_width.max(1.0);
        let delta = (delta_px * spx) as isize;
        let max_start = sample_count.saturating_sub(self.view_samples) as isize;
        self.view_start = (self.view_start as isize - delta).clamp(0, max_start) as usize;
        self.auto_scroll = false;
    }

    /// Zoom: `factor < 1.0` = zoom in (fewer visible samples), `factor > 1.0` = zoom out.
    pub fn zoom(&mut self, factor: f64, sample_count: usize) {
        if sample_count == 0 {
            return;
        }
        let center = self.view_start + self.view_samples / 2;
        let new_samples = ((self.view_samples as f64 * factor) as usize).clamp(16, sample_count);
        self.view_samples = new_samples;
        self.view_start = center
            .saturating_sub(new_samples / 2)
            .min(sample_count.saturating_sub(new_samples));
        self.auto_scroll = false;
    }

    /// Update decoder config based on handle's digital trace.
    /// Call after `clamp_view` to ensure decoder has latest data.
    pub fn update_decoder(&mut self, handle: &DeviceHandle) {
        if self.decoder_dirty {
            self.rebuild_decoder();
        }
        if let Some(dt) = handle.digital_trace() {
            self.feed_decoder(dt);
        }
    }
}
