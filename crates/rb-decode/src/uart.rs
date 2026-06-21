//! UART (asynchronous serial) protocol decoder.
//!
//! Written clean-room from the public UART specification (no GPLv3 source).
//!
//! Supported features:
//! - 5–8 data bits (default 8)
//! - Optional odd/even parity
//! - 1 stop bit (2-stop-bit captures also decode correctly: the second stop
//!   bit is treated as idle time, and the next start bit is found via the
//!   normal falling-edge detector)
//! - Configurable bit position in the packed [`LogicWord`]
//!
//! The decoder is purely sample-based: it locates the start bit by falling-edge
//! detection and then samples each subsequent bit at the centre of its bit-cell
//! (derived from `rate_hz` and `baud_rate`).

use crate::{annotation::Annotation, decoder::Decoder};

// ── Configuration ─────────────────────────────────────────────────────────────

/// Parity bit mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Parity {
    /// No parity bit.
    #[default]
    None,
    /// Total 1-bit count (data + parity) is even.
    Even,
    /// Total 1-bit count (data + parity) is odd.
    Odd,
}

/// Number of stop bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StopBits {
    #[default]
    One,
    Two,
}

/// UART decoder configuration.
#[derive(Clone, Debug)]
pub struct UartConfig {
    /// Bit index of the RX line in each packed [`LogicWord`].
    pub rx_bit: u8,
    /// Baud rate in bits per second.
    pub baud_rate: u32,
    /// Number of data bits per frame (5–8; default 8).
    pub data_bits: u8,
    /// Parity configuration.
    pub parity: Parity,
    /// Stop-bit count.
    pub stop_bits: StopBits,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self {
            rx_bit: 0,
            baud_rate: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
        }
    }
}

// ── Internal state ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum UartState {
    /// Waiting for a falling edge (start bit).
    Idle,
    /// Detected start bit at `frame_start`; sampling centre at `next`.
    StartBit { frame_start: usize, next: usize },
    /// Accumulating data bits.
    DataBits {
        frame_start: usize,
        bits_done: u8,
        data: u8,
        next: usize,
    },
    /// Waiting for the parity bit.
    ParityBit {
        frame_start: usize,
        data: u8,
        next: usize,
    },
    /// Waiting for the stop bit.
    StopBit {
        frame_start: usize,
        data: u8,
        next: usize,
    },
}

// ── Decoder ────────────────────────────────────────────────────────────────────

/// UART protocol decoder.
///
/// Detects start bits by falling-edge detection, then samples each bit at the
/// centre of its cell. Works with any sample rate ≥ 2 × baud rate.
pub struct UartDecoder {
    config: UartConfig,
    state: UartState,
    /// Level of the RX line at the last processed sample (true = high/idle).
    prev_level: bool,
}

impl UartDecoder {
    /// Creates a decoder with the given configuration.
    #[must_use]
    pub fn new(config: UartConfig) -> Self {
        Self {
            config,
            state: UartState::Idle,
            prev_level: true, // idle line is high
        }
    }

    #[inline]
    fn bit(&self, word: u64) -> bool {
        (word >> self.config.rx_bit) & 1 != 0
    }

    #[inline]
    fn half_bit(rate_hz: f64, baud: u32) -> usize {
        ((rate_hz / baud as f64) / 2.0).round() as usize
    }

    #[inline]
    fn full_bit(rate_hz: f64, baud: u32) -> usize {
        (rate_hz / baud as f64).round() as usize
    }
}

impl Decoder for UartDecoder {
    fn name(&self) -> &str {
        "UART"
    }

    fn feed(&mut self, words: &[u64], from_sample: usize, rate_hz: f64) -> Vec<Annotation> {
        let mut annotations = Vec::new();
        let end = from_sample + words.len();
        // Clamp to 1 to avoid division-by-zero when rate ≈ baud.
        let half = Self::half_bit(rate_hz, self.config.baud_rate).max(1);
        let full = Self::full_bit(rate_hz, self.config.baud_rate).max(1);

        // `scan_idx` is the lower bound for Idle-state edge scanning *within*
        // this call.  It advances past already-checked samples when we re-enter
        // Idle after a false start or a completed frame.
        let mut scan_idx = from_sample;

        'outer: loop {
            match self.state {
                UartState::Idle => {
                    // Snapshot the scan start so the range bound is not mutated
                    // inside the loop body (fixes clippy::mut_range_bound).
                    let scan_start = scan_idx;
                    for si in scan_start..end {
                        let level = self.bit(words[si - from_sample]);
                        if self.prev_level && !level {
                            // Falling edge → potential start bit.
                            self.state = UartState::StartBit {
                                frame_start: si,
                                next: si + half,
                            };
                            self.prev_level = level;
                            scan_idx = si + 1;
                            continue 'outer;
                        }
                        self.prev_level = level;
                    }
                    // No falling edge in this chunk.
                    break;
                }

                UartState::StartBit { frame_start, next } => {
                    if next >= end {
                        break; // Wait for more data.
                    }
                    let level = self.bit(words[next - from_sample]);
                    if level {
                        // False start (noise spike) — resume idle scanning.
                        self.state = UartState::Idle;
                        self.prev_level = level;
                        scan_idx = next + 1;
                        continue 'outer;
                    }
                    // Valid start bit.
                    self.state = UartState::DataBits {
                        frame_start,
                        bits_done: 0,
                        data: 0,
                        next: next + full,
                    };
                    continue 'outer;
                }

                UartState::DataBits {
                    frame_start,
                    bits_done,
                    data,
                    next,
                } => {
                    if next >= end {
                        break;
                    }
                    let level = self.bit(words[next - from_sample]);
                    let new_data = data | ((level as u8) << bits_done);
                    let new_bits = bits_done + 1;

                    if new_bits == self.config.data_bits {
                        let next_next = next + full;
                        self.state = match self.config.parity {
                            Parity::None => UartState::StopBit {
                                frame_start,
                                data: new_data,
                                next: next_next,
                            },
                            _ => UartState::ParityBit {
                                frame_start,
                                data: new_data,
                                next: next_next,
                            },
                        };
                    } else {
                        self.state = UartState::DataBits {
                            frame_start,
                            bits_done: new_bits,
                            data: new_data,
                            next: next + full,
                        };
                    }
                    continue 'outer;
                }

                UartState::ParityBit {
                    frame_start,
                    data,
                    next,
                } => {
                    if next >= end {
                        break;
                    }
                    let level = self.bit(words[next - from_sample]);
                    let parity_ok = match self.config.parity {
                        Parity::Even => (data.count_ones() + level as u32) % 2 == 0,
                        Parity::Odd => (data.count_ones() + level as u32) % 2 == 1,
                        Parity::None => unreachable!(),
                    };
                    if parity_ok {
                        self.state = UartState::StopBit {
                            frame_start,
                            data,
                            next: next + full,
                        };
                    } else {
                        annotations.push(Annotation::error(
                            format!("PARITY ERR 0x{data:02X}"),
                            frame_start..next + full,
                        ));
                        self.state = UartState::Idle;
                        self.prev_level = true; // assume line went high
                        scan_idx = next + full;
                    }
                    continue 'outer;
                }

                UartState::StopBit {
                    frame_start,
                    data,
                    next,
                } => {
                    if next >= end {
                        break;
                    }
                    let level = self.bit(words[next - from_sample]);
                    let frame_end = next + half;
                    if level {
                        annotations.push(Annotation::data(data, frame_start..frame_end));
                    } else {
                        annotations.push(Annotation::error(
                            format!("FRAME ERR 0x{data:02X}"),
                            frame_start..frame_end,
                        ));
                    }
                    self.state = UartState::Idle;
                    self.prev_level = level;
                    scan_idx = frame_end;
                    continue 'outer;
                }
            }
        }

        annotations
    }

    fn reset(&mut self) {
        self.state = UartState::Idle;
        self.prev_level = true;
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::AnnotationKind;

    /// Encode bytes as UART logic words on `rx_bit`.
    /// Idle = high, start bit = low, data bits LSB-first, stop = high.
    fn uart_encode(bytes: &[u8], baud: u32, rate_hz: f64, rx_bit: u8) -> Vec<u64> {
        let spp = (rate_hz / baud as f64).round() as usize;
        let bit_mask = 1u64 << rx_bit;
        let mut words = vec![bit_mask; 5]; // pre-idle

        for &byte in bytes {
            // Start bit (low)
            words.extend(vec![0u64; spp]);
            // 8 data bits, LSB first
            for b in 0..8u8 {
                let bit_val = (byte >> b) & 1;
                words.extend(vec![(bit_val as u64) << rx_bit; spp]);
            }
            // Stop bit (high)
            words.extend(vec![bit_mask; spp]);
        }

        words.extend(vec![bit_mask; 5]); // post-idle
        words
    }

    #[test]
    fn decode_single_byte() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let words = uart_encode(&[0x41], baud, rate, 0);
        let mut dec = UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        });
        let anns = dec.feed(&words, 0, rate);
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].data_byte, Some(0x41));
        assert_eq!(anns[0].label, "0x41");
        assert_eq!(anns[0].kind, AnnotationKind::Data);
    }

    #[test]
    fn decode_multiple_bytes() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let words = uart_encode(&[0x41, 0x42], baud, rate, 0);
        let mut dec = UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        });
        let anns = dec.feed(&words, 0, rate);
        assert_eq!(anns.len(), 2);
        assert_eq!(anns[0].data_byte, Some(0x41));
        assert_eq!(anns[1].data_byte, Some(0x42));
    }

    #[test]
    fn chunked_feed_matches_single_feed() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let words = uart_encode(&[0x55], baud, rate, 0);

        let mut dec_one = UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        });
        let anns_one = dec_one.feed(&words, 0, rate);

        let mut dec_chunks = UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        });
        let chunk_size = 7;
        let mut all_anns = Vec::new();
        for (i, chunk) in words.chunks(chunk_size).enumerate() {
            all_anns.extend(dec_chunks.feed(chunk, i * chunk_size, rate));
        }

        assert_eq!(anns_one, all_anns);
    }

    #[test]
    fn reset_discards_in_progress_frame() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let spp = (rate / baud as f64).round() as usize;

        // Start bit only — frame is incomplete.
        let mut partial: Vec<u64> = vec![1u64; 5]; // idle
        partial.extend(vec![0u64; spp]); // start bit

        let mut dec = UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        });
        let anns = dec.feed(&partial, 0, rate);
        assert!(
            anns.is_empty(),
            "incomplete frame must not produce annotations"
        );

        dec.reset();

        // After reset, a fresh complete byte decodes normally.
        let words = uart_encode(&[0x20], baud, rate, 0);
        let anns2 = dec.feed(&words, 0, rate);
        assert_eq!(anns2.len(), 1);
        assert_eq!(anns2[0].data_byte, Some(0x20));
    }

    #[test]
    fn even_parity_correct() {
        // 0x41 = 0b01000001 → 2 ones → even parity bit = 0
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let spp = (rate / baud as f64).round() as usize;
        let half = spp / 2;
        let byte = 0x41u8;

        let mut words = vec![1u64; 5]; // idle
        words.extend(vec![0u64; spp]); // start bit
        for b in 0..8u8 {
            let bit_val = (byte >> b) & 1;
            words.extend(vec![bit_val as u64; spp]);
        }
        words.extend(vec![0u64; spp]); // even parity bit = 0
        words.extend(vec![1u64; spp + half + 1]); // stop + trailing idle

        let mut dec = UartDecoder::new(UartConfig {
            baud_rate: baud,
            data_bits: 8,
            parity: Parity::Even,
            ..Default::default()
        });
        let anns = dec.feed(&words, 0, rate);
        assert_eq!(anns.len(), 1, "should decode one byte with correct parity");
        assert_eq!(anns[0].data_byte, Some(0x41));
        assert_eq!(anns[0].kind, AnnotationKind::Data);
    }

    #[test]
    fn odd_parity_error_emits_error_annotation() {
        // 0x41 = 2 ones → odd parity bit should be 1 (to make total odd)
        // We send parity_bit = 0 (wrong) → should emit PARITY ERR
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let spp = (rate / baud as f64).round() as usize;
        let half = spp / 2;
        let byte = 0x41u8;

        let mut words = vec![1u64; 5]; // idle
        words.extend(vec![0u64; spp]); // start bit
        for b in 0..8u8 {
            let bit_val = (byte >> b) & 1;
            words.extend(vec![bit_val as u64; spp]);
        }
        words.extend(vec![0u64; spp]); // wrong parity bit (should be 1)
        words.extend(vec![1u64; spp + half + 1]); // stop + trailing

        let mut dec = UartDecoder::new(UartConfig {
            baud_rate: baud,
            data_bits: 8,
            parity: Parity::Odd,
            ..Default::default()
        });
        let anns = dec.feed(&words, 0, rate);
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].kind, AnnotationKind::Error);
        assert!(anns[0].label.contains("PARITY"));
    }
}
