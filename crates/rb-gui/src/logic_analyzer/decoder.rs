//! Protocol decoder configuration and state.
//!
//! [`DecoderConfig`] owns all decoder-related state (kind, per-protocol
//! parameters, cached decoder instance, and annotations). It is stored as a
//! field on [`LogicAnalyzerContent`](crate::logic_analyzer::LogicAnalyzerContent).
//!
//! The UI widget for editing the config lives in
//! [`components::decoder_setup`](crate::logic_analyzer::components::decoder_setup::DecoderSetup).

use rb_decode::{Annotation, Decoder, I2cConfig, I2cDecoder, SpiConfig, SpiDecoder, UartConfig, UartDecoder};
use rb_model::DigitalTrace;

// Re-export DecoderKind from row_layout (it's used by both RowDescriptor and DecoderConfig).
pub use crate::logic_analyzer::waveform_state::row_layout::DecoderKind;

// ── Decoder config ────────────────────────────────────────────────────────────

/// Configuration for protocol decoding: protocol selection, per-protocol
/// parameters, the (non-Clone) decoder instance, and its output annotations.
///
/// Manual Clone impl because `Box<dyn Decoder>` isn't Clone — the decoder
/// is rebuilt on demand after cloning.
pub struct DecoderConfig {
    /// Which protocol decoder is selected.
    pub kind: DecoderKind,
    /// Rebuilt on demand; skipped by Clone (reconstructed from config).
    #[allow(clippy::type_complexity)]
    decoder: Option<Box<dyn Decoder>>,
    /// Annotations produced by the decoder.
    pub annotations: Vec<Annotation>,
    /// How many digital-store words have been fed to the decoder so far.
    decoded_up_to: usize,
    /// When `true`, the decoder is rebuilt and all annotations are cleared on
    /// the next `feed()` call.
    pub dirty: bool,

    // ── Per-protocol parameters ──────────────────────────────────────────
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

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            kind: DecoderKind::None,
            decoder: None,
            annotations: Vec::new(),
            decoded_up_to: 0,
            dirty: false,
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

// Manual Clone because `Box<dyn Decoder>` isn't Clone.
impl Clone for DecoderConfig {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            decoder: None, // rebuilt on demand
            annotations: self.annotations.clone(),
            decoded_up_to: 0,
            dirty: true, // force rebuild
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

impl DecoderConfig {
    /// Rebuild the decoder from the current `kind` + parameters, clearing
    /// cached annotations.
    pub fn rebuild(&mut self) {
        self.decoder = match self.kind {
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
        self.dirty = false;
    }

    /// Feed new digital samples to the decoder, appending any new annotations
    /// to `self.annotations`.
    pub fn feed(&mut self, dt: &DigitalTrace) {
        if self.dirty {
            self.rebuild();
        }
        if let Some(dec) = &mut self.decoder {
            let store = dt.store();
            let rate = dt.timebase().sample_rate_hz();
            if self.decoded_up_to < store.len() {
                let words = store.words_range(self.decoded_up_to..store.len());
                let new_anns =
                    dec.feed(&words, self.decoded_up_to, rate);
                self.annotations.extend(new_anns);
                self.decoded_up_to = store.len();
            }
        }
    }

    /// Convenience: feed the decoder from a tab's digital trace.
    pub fn update_from_tab(&mut self, digital: Option<&DigitalTrace>) {
        if self.dirty {
            self.rebuild();
        }
        if let Some(dt) = digital {
            self.feed(dt);
        }
    }
}
