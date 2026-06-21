//! SPI protocol decoder (four standard modes).
//!
//! Written clean-room from the Motorola SPI specification and publicly
//! available application notes. No GPLv3 source referenced.
//!
//! Supports:
//! - Modes 0–3 (CPOL / CPHA combinations)
//! - MSB-first or LSB-first bit order
//! - Configurable data-bit count per word (default 8)
//! - Separate MOSI and MISO annotation rows
//!
//! CS is active-low; `cs_bit = 1` means de-selected.

use crate::{
    annotation::{Annotation, AnnotationKind},
    decoder::Decoder,
};

// ── Configuration ─────────────────────────────────────────────────────────────

/// SPI decoder configuration.
#[derive(Clone, Debug)]
pub struct SpiConfig {
    /// Bit index of the CLK line.
    pub clk_bit: u8,
    /// Bit index of MOSI (master→slave).
    pub mosi_bit: u8,
    /// Bit index of MISO (slave→master).
    pub miso_bit: u8,
    /// Bit index of CS/SS (active-low chip-select).
    pub cs_bit: u8,
    /// SPI mode 0–3.  Encodes CPOL and CPHA:
    /// - Mode 0: CPOL=0, CPHA=0 → sample on rising edge  (CLK idles low)
    /// - Mode 1: CPOL=0, CPHA=1 → sample on falling edge (CLK idles low)
    /// - Mode 2: CPOL=1, CPHA=0 → sample on falling edge (CLK idles high)
    /// - Mode 3: CPOL=1, CPHA=1 → sample on rising edge  (CLK idles high)
    pub mode: u8,
    /// Bits per SPI word (default 8).
    pub data_bits: u8,
    /// `true` = MSB first (default), `false` = LSB first.
    pub msb_first: bool,
}

impl Default for SpiConfig {
    fn default() -> Self {
        Self {
            clk_bit: 0,
            mosi_bit: 1,
            miso_bit: 2,
            cs_bit: 3,
            mode: 0,
            data_bits: 8,
            msb_first: true,
        }
    }
}

impl SpiConfig {
    /// Clock polarity: `true` means CLK idles high (modes 2 and 3).
    pub fn cpol(&self) -> bool {
        self.mode >= 2
    }

    /// `true` when data should be sampled on the CLK rising edge.
    /// This is the case when CPOL == CPHA (modes 0 and 3).
    pub fn sample_on_rising(&self) -> bool {
        let cpha = self.mode & 1 != 0;
        self.cpol() == cpha
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum SpiState {
    /// CS de-asserted — not currently in a transfer.
    Idle,
    /// CS asserted — accumulating bits for the current word.
    Active {
        bits: u8,
        mosi: u16,
        miso: u16,
        start: usize,
    },
}

// ── Decoder ────────────────────────────────────────────────────────────────────

/// SPI protocol decoder.
pub struct SpiDecoder {
    config: SpiConfig,
    state: SpiState,
    prev_cs: bool,
    prev_clk: bool,
}

impl SpiDecoder {
    /// Creates a decoder with the given configuration.
    #[must_use]
    pub fn new(config: SpiConfig) -> Self {
        let prev_clk = config.cpol(); // CLK idles at CPOL level
        Self {
            config,
            state: SpiState::Idle,
            prev_cs: true, // CS de-asserted (high)
            prev_clk,
        }
    }

    #[inline]
    fn bit_of(&self, word: u64, bit: u8) -> bool {
        (word >> bit) & 1 != 0
    }
}

impl Decoder for SpiDecoder {
    fn name(&self) -> &str {
        "SPI"
    }

    fn feed(&mut self, words: &[u64], from_sample: usize, _rate_hz: f64) -> Vec<Annotation> {
        let mut annotations = Vec::new();
        let sample_rising = self.config.sample_on_rising();

        for (i, &word) in words.iter().enumerate() {
            let si = from_sample + i;
            let cs = self.bit_of(word, self.config.cs_bit);
            let clk = self.bit_of(word, self.config.clk_bit);

            let cs_asserted = self.prev_cs && !cs; // CS: 1 → 0
            let cs_deasserted = !self.prev_cs && cs; // CS: 0 → 1
            let clk_rose = !self.prev_clk && clk;
            let clk_fell = self.prev_clk && !clk;

            // ── CS de-assert: end of transfer ────────────────────────────────
            if cs_deasserted {
                annotations.push(Annotation::frame("CS↑", si..si + 1));
                self.state = SpiState::Idle;
                self.prev_cs = cs;
                self.prev_clk = clk;
                continue;
            }

            // ── CS assert: start of transfer ──────────────────────────────────
            if cs_asserted {
                annotations.push(Annotation::frame("CS↓", si..si + 1));
                self.state = SpiState::Active {
                    bits: 0,
                    mosi: 0,
                    miso: 0,
                    start: si,
                };
                self.prev_cs = cs;
                self.prev_clk = clk;
                continue;
            }

            // ── Sample edge ───────────────────────────────────────────────────
            let should_sample = if sample_rising { clk_rose } else { clk_fell };

            if should_sample {
                if let SpiState::Active {
                    bits,
                    mosi,
                    miso,
                    start,
                } = self.state
                {
                    let mosi_bit = self.bit_of(word, self.config.mosi_bit) as u16;
                    let miso_bit = self.bit_of(word, self.config.miso_bit) as u16;

                    let (new_mosi, new_miso) = if self.config.msb_first {
                        ((mosi << 1) | mosi_bit, (miso << 1) | miso_bit)
                    } else {
                        (mosi | (mosi_bit << bits), miso | (miso_bit << bits))
                    };
                    let new_bits = bits + 1;

                    if new_bits == self.config.data_bits {
                        let mosi_byte = new_mosi as u8;
                        let miso_byte = new_miso as u8;
                        annotations.push(Annotation {
                            range: start..si + 1,
                            label: format!("MOSI:0x{mosi_byte:02X}"),
                            kind: AnnotationKind::Data,
                            data_byte: Some(mosi_byte),
                        });
                        annotations.push(Annotation {
                            range: start..si + 1,
                            label: format!("MISO:0x{miso_byte:02X}"),
                            kind: AnnotationKind::Data,
                            data_byte: Some(miso_byte),
                        });
                        // Start the next word immediately.
                        self.state = SpiState::Active {
                            bits: 0,
                            mosi: 0,
                            miso: 0,
                            start: si + 1,
                        };
                    } else {
                        self.state = SpiState::Active {
                            bits: new_bits,
                            mosi: new_mosi,
                            miso: new_miso,
                            start,
                        };
                    }
                }
            }

            self.prev_cs = cs;
            self.prev_clk = clk;
        }

        annotations
    }

    fn reset(&mut self) {
        self.state = SpiState::Idle;
        self.prev_cs = true;
        self.prev_clk = self.config.cpol();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Samples per clock half-cycle used in the test encoder.
    const CPP: usize = 4;

    /// Encode SPI Mode-0 transfers (CLK idles low, sample on rising edge).
    /// CS is bit 3, CLK=0, MOSI=1, MISO=2.
    fn encode_mode0(
        mosi_bytes: &[u8],
        miso_bytes: &[u8],
        clk_bit: u8,
        mosi_bit: u8,
        miso_bit: u8,
        cs_bit: u8,
    ) -> Vec<u64> {
        let idle_word = 1u64 << cs_bit; // CS=1, CLK=0
        let mut words = vec![idle_word; CPP]; // pre-idle

        let set = |cs: bool, clk: bool, mosi: bool, miso: bool| -> u64 {
            ((cs as u64) << cs_bit)
                | ((clk as u64) << clk_bit)
                | ((mosi as u64) << mosi_bit)
                | ((miso as u64) << miso_bit)
        };

        // Assert CS
        for _ in 0..CPP {
            words.push(set(false, false, false, false)); // CS=0, CLK=0
        }

        let n = mosi_bytes.len().max(miso_bytes.len());
        for idx in 0..n {
            let mosi_byte = *mosi_bytes.get(idx).unwrap_or(&0);
            let miso_byte = *miso_bytes.get(idx).unwrap_or(&0);
            // MSB first: bit 7 down to bit 0
            for b in (0..8).rev() {
                let mo = (mosi_byte >> b) & 1 != 0;
                let mi = (miso_byte >> b) & 1 != 0;
                // CLK low (setup MOSI/MISO)
                for _ in 0..CPP {
                    words.push(set(false, false, mo, mi));
                }
                // CLK high (sample edge)
                for _ in 0..CPP {
                    words.push(set(false, true, mo, mi));
                }
            }
        }

        // De-assert CS
        for _ in 0..CPP {
            words.push(set(false, false, false, false)); // CLK low before CS
        }
        for _ in 0..CPP {
            words.push(idle_word); // CS=1
        }

        words
    }

    fn default_cfg() -> SpiConfig {
        SpiConfig {
            clk_bit: 0,
            mosi_bit: 1,
            miso_bit: 2,
            cs_bit: 3,
            mode: 0,
            data_bits: 8,
            msb_first: true,
        }
    }

    #[test]
    fn decode_single_byte() {
        let words = encode_mode0(&[0xA5], &[0x5A], 0, 1, 2, 3);
        let mut dec = SpiDecoder::new(default_cfg());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let data: Vec<_> = anns
            .iter()
            .filter(|a| a.kind == AnnotationKind::Data)
            .collect();
        assert_eq!(data.len(), 2, "expected MOSI + MISO; got {data:?}");

        let mosi = data.iter().find(|a| a.label.starts_with("MOSI")).unwrap();
        let miso = data.iter().find(|a| a.label.starts_with("MISO")).unwrap();
        assert_eq!(mosi.data_byte, Some(0xA5));
        assert_eq!(miso.data_byte, Some(0x5A));
    }

    #[test]
    fn decode_multiple_bytes() {
        let words = encode_mode0(&[0x01, 0x02], &[0xFE, 0xFD], 0, 1, 2, 3);
        let mut dec = SpiDecoder::new(default_cfg());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let mosi_bytes: Vec<u8> = anns
            .iter()
            .filter(|a| a.label.starts_with("MOSI"))
            .filter_map(|a| a.data_byte)
            .collect();
        assert_eq!(mosi_bytes, vec![0x01, 0x02]);
    }

    #[test]
    fn cs_frame_annotations_emitted() {
        let words = encode_mode0(&[0xFF], &[0x00], 0, 1, 2, 3);
        let mut dec = SpiDecoder::new(default_cfg());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        assert!(
            anns.iter()
                .any(|a| a.kind == AnnotationKind::Frame && a.label == "CS\u{2193}"),
            "expected CS↓ frame"
        );
        assert!(
            anns.iter()
                .any(|a| a.kind == AnnotationKind::Frame && a.label == "CS\u{2191}"),
            "expected CS↑ frame"
        );
    }

    #[test]
    fn data_outside_cs_is_ignored() {
        // Build idle words (CS=1) that have clock edges — should produce no Data.
        let clk_bit = 0u8;
        let cs_bit = 3u8;
        // CLK toggling but CS=1 (de-asserted)
        let mut words = Vec::new();
        for _ in 0..4 {
            words.push(1u64 << cs_bit); // CS=1, CLK=0
            words.push((1u64 << cs_bit) | (1u64 << clk_bit)); // CS=1, CLK=1
        }
        let mut dec = SpiDecoder::new(default_cfg());
        let anns = dec.feed(&words, 0, 1_000_000.0);
        let data: Vec<_> = anns
            .iter()
            .filter(|a| a.kind == AnnotationKind::Data)
            .collect();
        assert!(data.is_empty(), "no Data expected outside CS; got {data:?}");
    }

    #[test]
    fn reset_clears_state() {
        let words = encode_mode0(&[0xAA], &[0x55], 0, 1, 2, 3);
        let mut dec = SpiDecoder::new(default_cfg());
        // Feed partial data, then reset.
        dec.feed(&words[..words.len() / 2], 0, 1_000_000.0);
        dec.reset();
        // Full decode from scratch.
        let anns = dec.feed(&words, 0, 1_000_000.0);
        let data: Vec<_> = anns
            .iter()
            .filter(|a| a.kind == AnnotationKind::Data)
            .collect();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].data_byte, Some(0xAA));
    }
}
