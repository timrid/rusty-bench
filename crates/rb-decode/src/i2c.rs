//! I²C protocol decoder.
//!
//! Written clean-room from the UM10204 I²C-bus specification (NXP, freely
//! available). No GPLv3 source referenced.
//!
//! Supports:
//! - 7-bit addressing
//! - Write and read transfers
//! - ACK / NACK detection
//! - Repeated START
//!
//! This decoder is **edge-driven**: `rate_hz` is accepted by the [`Decoder`]
//! trait but not used (I²C timing is self-clocked by SCL).

use crate::{annotation::Annotation, decoder::Decoder};

// ── Configuration ─────────────────────────────────────────────────────────────

/// I²C decoder configuration.
#[derive(Clone, Debug)]
pub struct I2cConfig {
    /// Bit index of the SCL (clock) line in each packed [`LogicWord`].
    pub scl_bit: u8,
    /// Bit index of the SDA (data) line in each packed [`LogicWord`].
    pub sda_bit: u8,
}

impl Default for I2cConfig {
    fn default() -> Self {
        Self {
            scl_bit: 0,
            sda_bit: 1,
        }
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum I2cState {
    /// Bus idle (or not yet seen a START condition).
    Idle,
    /// Collecting the 7-bit address + R/W bit (8 SCL rising edges).
    Address {
        bits: u8,
        byte: u8,
        start_sample: usize,
    },
    /// Waiting for the 9th clock (ACK/NAK) after the address byte.
    AddressAck { addr_byte: u8, start_sample: usize },
    /// Collecting a data byte (8 SCL rising edges).
    Data {
        bits: u8,
        byte: u8,
        start_sample: usize,
        byte_num: u32,
    },
    /// Waiting for the 9th clock (ACK/NAK) after a data byte.
    DataAck {
        data_byte: u8,
        start_sample: usize,
        byte_num: u32,
    },
}

// ── Decoder ────────────────────────────────────────────────────────────────────

/// I²C protocol decoder (7-bit addressing, ACK/NACK, repeated START).
pub struct I2cDecoder {
    config: I2cConfig,
    state: I2cState,
    prev_scl: bool,
    prev_sda: bool,
}

impl I2cDecoder {
    /// Creates a decoder with the given configuration.
    #[must_use]
    pub fn new(config: I2cConfig) -> Self {
        Self {
            config,
            state: I2cState::Idle,
            prev_scl: true,
            prev_sda: true,
        }
    }

    #[inline]
    fn scl(&self, word: u64) -> bool {
        (word >> self.config.scl_bit) & 1 != 0
    }

    #[inline]
    fn sda(&self, word: u64) -> bool {
        (word >> self.config.sda_bit) & 1 != 0
    }
}

impl Decoder for I2cDecoder {
    fn name(&self) -> &str {
        "I\u{00B2}C" // "I²C"
    }

    fn feed(&mut self, words: &[u64], from_sample: usize, _rate_hz: f64) -> Vec<Annotation> {
        let mut annotations = Vec::new();

        for (i, &word) in words.iter().enumerate() {
            let si = from_sample + i;
            let scl = self.scl(word);
            let sda = self.sda(word);

            let scl_rose = !self.prev_scl && scl;
            let sda_fell = self.prev_sda && !sda;
            let sda_rose = !self.prev_sda && sda;

            // ── Condition detection (SCL stable high while SDA changes) ──────
            if scl && sda_fell {
                // START (or repeated START) condition.
                annotations.push(Annotation::frame("START", si..si + 1));
                self.state = I2cState::Address {
                    bits: 0,
                    byte: 0,
                    start_sample: si,
                };
                self.prev_scl = scl;
                self.prev_sda = sda;
                continue;
            }

            if scl && sda_rose {
                // STOP condition.
                annotations.push(Annotation::frame("STOP", si..si + 1));
                self.state = I2cState::Idle;
                self.prev_scl = scl;
                self.prev_sda = sda;
                continue;
            }

            // ── Data sampling on SCL rising edge ─────────────────────────────
            if scl_rose {
                // Take a snapshot of state fields to avoid borrow issues.
                match self.state.clone() {
                    I2cState::Address {
                        bits,
                        byte,
                        start_sample,
                    } => {
                        // MSB first: shift left and OR in the new bit.
                        let new_byte = (byte << 1) | sda as u8;
                        let new_bits = bits + 1;
                        if new_bits == 8 {
                            self.state = I2cState::AddressAck {
                                addr_byte: new_byte,
                                start_sample,
                            };
                        } else {
                            self.state = I2cState::Address {
                                bits: new_bits,
                                byte: new_byte,
                                start_sample,
                            };
                        }
                    }

                    I2cState::AddressAck {
                        addr_byte,
                        start_sample,
                    } => {
                        let addr = addr_byte >> 1;
                        let is_read = addr_byte & 1 == 1;
                        let rw = if is_read { "R" } else { "W" };
                        // ACK = SDA low (slave pulls down); NACK = SDA high.
                        let ack = !sda;
                        if ack {
                            annotations.push(Annotation::address(
                                addr,
                                format!("0x{addr:02X} {rw}"),
                                start_sample..si + 1,
                            ));
                            self.state = I2cState::Data {
                                bits: 0,
                                byte: 0,
                                start_sample: si,
                                byte_num: 0,
                            };
                        } else {
                            annotations.push(Annotation::error(
                                format!("NAK addr 0x{addr:02X} {rw}"),
                                start_sample..si + 1,
                            ));
                            self.state = I2cState::Idle;
                        }
                    }

                    I2cState::Data {
                        bits,
                        byte,
                        start_sample,
                        byte_num,
                    } => {
                        let new_byte = (byte << 1) | sda as u8;
                        let new_bits = bits + 1;
                        if new_bits == 8 {
                            self.state = I2cState::DataAck {
                                data_byte: new_byte,
                                start_sample,
                                byte_num,
                            };
                        } else {
                            self.state = I2cState::Data {
                                bits: new_bits,
                                byte: new_byte,
                                start_sample,
                                byte_num,
                            };
                        }
                    }

                    I2cState::DataAck {
                        data_byte,
                        start_sample,
                        byte_num,
                    } => {
                        let ack = !sda;
                        annotations.push(Annotation::data(data_byte, start_sample..si + 1));
                        if ack {
                            self.state = I2cState::Data {
                                bits: 0,
                                byte: 0,
                                start_sample: si,
                                byte_num: byte_num + 1,
                            };
                        } else {
                            // NACK by master = end of read, or master signalling stop.
                            self.state = I2cState::Idle;
                        }
                    }

                    I2cState::Idle => {
                        // Clock while idle — ignore (bus initialisation, etc.).
                    }
                }
            }

            self.prev_scl = scl;
            self.prev_sda = sda;
        }

        annotations
    }

    fn reset(&mut self) {
        self.state = I2cState::Idle;
        self.prev_scl = true;
        self.prev_sda = true;
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::AnnotationKind;

    // Each "bit time" uses `CPP` samples per SCL half-cycle.
    const CPP: usize = 6;

    /// Encode an I²C write transaction: START → addr(7-bit)+W → ACK → data bytes
    /// → ACK each → STOP.  SCL = `scl_bit`, SDA = `sda_bit`.
    fn encode_write(addr7: u8, data: &[u8], scl_bit: u8, sda_bit: u8) -> Vec<u64> {
        let set =
            |scl: bool, sda: bool| -> u64 { ((scl as u64) << scl_bit) | ((sda as u64) << sda_bit) };

        let mut words = vec![set(true, true); CPP]; // idle

        // START: SDA falls while SCL=1
        words.push(set(true, false));
        words.push(set(false, false)); // SCL falls

        // Clock out a single bit: setup → SCL high (sample) → SCL low
        let clock = |w: &mut Vec<u64>, bit: bool| {
            for _ in 0..CPP / 2 {
                w.push(set(false, bit)); // setup (SCL low)
            }
            for _ in 0..CPP / 2 {
                w.push(set(true, bit)); // SCL high → decoder samples here
            }
            for _ in 0..CPP / 2 {
                w.push(set(false, bit)); // SCL low again
            }
        };

        // Address byte (7-bit addr + R/W=0), MSB first
        let addr_byte = (addr7 << 1) | 0;
        for b in (0..8).rev() {
            clock(&mut words, (addr_byte >> b) & 1 != 0);
        }
        clock(&mut words, false); // ACK (slave pulls SDA=0)

        // Data bytes
        for &byte in data {
            for b in (0..8).rev() {
                clock(&mut words, (byte >> b) & 1 != 0);
            }
            clock(&mut words, false); // ACK
        }

        // STOP: SCL=1, then SDA rises
        words.push(set(false, false));
        words.push(set(true, false)); // SCL rises while SDA=0 (spurious bit — discarded)
        words.push(set(true, true)); // STOP: SDA rises while SCL=1

        words.extend(vec![set(true, true); CPP]); // post-idle
        words
    }

    #[test]
    fn decode_start_and_stop() {
        let words = encode_write(0x42, &[], 0, 1);
        let mut dec = I2cDecoder::new(I2cConfig::default());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let kinds: Vec<_> = anns.iter().map(|a| a.kind).collect();
        assert!(
            kinds.contains(&AnnotationKind::Frame),
            "expected Frame annotations; got {anns:?}"
        );
        assert!(anns.iter().any(|a| a.label == "START"));
        assert!(anns.iter().any(|a| a.label == "STOP"));
    }

    #[test]
    fn decode_write_address() {
        let words = encode_write(0x42, &[0xA5], 0, 1);
        let mut dec = I2cDecoder::new(I2cConfig::default());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let addr = anns
            .iter()
            .find(|a| a.kind == AnnotationKind::Address)
            .expect("expected Address annotation");
        assert_eq!(addr.data_byte, Some(0x42));
        assert!(addr.label.contains("0x42"), "label: {}", addr.label);
        assert!(addr.label.contains('W'), "label: {}", addr.label);
    }

    #[test]
    fn decode_write_data_byte() {
        let words = encode_write(0x42, &[0xA5], 0, 1);
        let mut dec = I2cDecoder::new(I2cConfig::default());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let data: Vec<_> = anns
            .iter()
            .filter(|a| a.kind == AnnotationKind::Data)
            .collect();
        assert_eq!(data.len(), 1, "expected 1 data byte; got {data:?}");
        assert_eq!(data[0].data_byte, Some(0xA5));
    }

    #[test]
    fn decode_multiple_data_bytes() {
        let words = encode_write(0x10, &[0x01, 0x02, 0x03], 0, 1);
        let mut dec = I2cDecoder::new(I2cConfig::default());
        let anns = dec.feed(&words, 0, 1_000_000.0);

        let data: Vec<u8> = anns
            .iter()
            .filter(|a| a.kind == AnnotationKind::Data)
            .filter_map(|a| a.data_byte)
            .collect();
        assert_eq!(data, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn chunked_feed_matches_single_feed() {
        let words = encode_write(0x20, &[0x55], 0, 1);
        let mut dec_one = I2cDecoder::new(I2cConfig::default());
        let anns_one = dec_one.feed(&words, 0, 1_000_000.0);

        let mut dec_chunks = I2cDecoder::new(I2cConfig::default());
        let chunk_size = 5;
        let mut all_anns = Vec::new();
        for (i, chunk) in words.chunks(chunk_size).enumerate() {
            all_anns.extend(dec_chunks.feed(chunk, i * chunk_size, 1_000_000.0));
        }
        assert_eq!(anns_one, all_anns);
    }

    #[test]
    fn reset_clears_in_progress_state() {
        let words = encode_write(0x42, &[0xA5], 0, 1);
        let mut dec = I2cDecoder::new(I2cConfig::default());
        // Feed only first half (partial transaction).
        dec.feed(&words[..words.len() / 2], 0, 1_000_000.0);
        dec.reset();
        // Full transaction from scratch.
        let anns = dec.feed(&words, 0, 1_000_000.0);
        assert!(anns.iter().any(|a| a.kind == AnnotationKind::Address));
        assert!(anns.iter().any(|a| a.kind == AnnotationKind::Data));
    }
}
