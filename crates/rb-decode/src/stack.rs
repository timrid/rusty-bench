//! Stacked decoder: pipes a lower [`Decoder`]'s byte output into a higher
//! [`ByteDecoder`].
//!
//! The composed decoder implements [`Decoder`] itself, so stacks can be
//! nested arbitrarily deep.  Both lower and upper annotations are returned
//! from each `feed` call.

use crate::{
    annotation::Annotation,
    decoder::{ByteDecoder, DecodedByte, Decoder},
};

/// Composes a lower [`Decoder`] (logic → bytes) with an upper [`ByteDecoder`]
/// (bytes → higher-level annotations).
///
/// # Example
///
/// ```rust,ignore
/// use rb_decode::{StackedDecoder, UartDecoder, UartConfig};
/// use rb_decode::decoder::ByteDecoder;
///
/// let lower = Box::new(UartDecoder::new(UartConfig::default()));
/// let upper = Box::new(MyProtocolDecoder::new());
/// let mut stacked = StackedDecoder::new(lower, upper);
/// let anns = stacked.feed(&words, 0, 1_000_000.0);
/// ```
pub struct StackedDecoder {
    lower: Box<dyn Decoder>,
    upper: Box<dyn ByteDecoder>,
    name: String,
}

impl StackedDecoder {
    /// Creates a stacked decoder.  The composed name is `"<lower>/<upper>"`.
    pub fn new(lower: Box<dyn Decoder>, upper: Box<dyn ByteDecoder>) -> Self {
        let name = format!("{}/{}", lower.name(), upper.name());
        Self { lower, upper, name }
    }
}

impl Decoder for StackedDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn feed(&mut self, words: &[u64], from_sample: usize, rate_hz: f64) -> Vec<Annotation> {
        // Run the lower decoder.
        let lower_anns = self.lower.feed(words, from_sample, rate_hz);

        // Extract decoded bytes from Data annotations that carry a raw byte.
        let bytes: Vec<DecodedByte> = lower_anns
            .iter()
            .filter_map(|a| {
                a.data_byte.map(|b| DecodedByte {
                    byte: b,
                    range: a.range.clone(),
                })
            })
            .collect();

        // Feed to the upper decoder only when there is something to process.
        let upper_anns = if bytes.is_empty() {
            vec![]
        } else {
            self.upper.feed_bytes(&bytes)
        };

        // Return both layers of annotations.
        let mut result = lower_anns;
        result.extend(upper_anns);
        result
    }

    fn reset(&mut self) {
        self.lower.reset();
        self.upper.reset();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        annotation::{Annotation, AnnotationKind},
        uart::{UartConfig, UartDecoder},
    };

    /// A trivial upper decoder that labels printable ASCII bytes.
    struct AsciiDecoder;

    impl ByteDecoder for AsciiDecoder {
        fn name(&self) -> &str {
            "ASCII"
        }

        fn feed_bytes(&mut self, bytes: &[DecodedByte]) -> Vec<Annotation> {
            bytes
                .iter()
                .filter(|b| b.byte.is_ascii_graphic() || b.byte == b' ')
                .map(|b| Annotation {
                    range: b.range.clone(),
                    label: format!("'{}'", b.byte as char),
                    kind: AnnotationKind::Data,
                    data_byte: None,
                })
                .collect()
        }

        fn reset(&mut self) {}
    }

    /// Encode bytes as UART logic words (same helper as uart.rs tests).
    fn uart_encode(bytes: &[u8], baud: u32, rate_hz: f64) -> Vec<u64> {
        let spp = (rate_hz / baud as f64).round() as usize;
        let mut words = vec![1u64; 5]; // idle
        for &byte in bytes {
            words.extend(vec![0u64; spp]);
            for b in 0..8u8 {
                let bit = (byte >> b) & 1;
                words.extend(vec![bit as u64; spp]);
            }
            words.extend(vec![1u64; spp]);
        }
        words.extend(vec![1u64; 5]);
        words
    }

    #[test]
    fn stacked_name_is_composed() {
        let lower = Box::new(UartDecoder::new(UartConfig::default()));
        let upper = Box::new(AsciiDecoder);
        let stacked = StackedDecoder::new(lower, upper);
        assert_eq!(stacked.name(), "UART/ASCII");
    }

    #[test]
    fn stacked_produces_both_layers() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let words = uart_encode(b"AB", baud, rate);

        let lower = Box::new(UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        }));
        let upper = Box::new(AsciiDecoder);
        let mut stacked = StackedDecoder::new(lower, upper);

        let anns = stacked.feed(&words, 0, rate);

        // Should have at least 2 UART data annotations (0x41, 0x42)
        let uart_data: Vec<_> = anns
            .iter()
            .filter(|a| a.data_byte.is_some() && a.label.starts_with("0x"))
            .collect();
        assert_eq!(uart_data.len(), 2, "expected 2 UART bytes; got {anns:?}");

        // And 2 ASCII labels ('A', 'B') from the upper decoder.
        let ascii: Vec<_> = anns.iter().filter(|a| a.label.starts_with('\'')).collect();
        assert_eq!(ascii.len(), 2, "expected 2 ASCII labels; got {anns:?}");
        assert!(ascii.iter().any(|a| a.label == "'A'"));
        assert!(ascii.iter().any(|a| a.label == "'B'"));
    }

    #[test]
    fn stacked_reset_delegates_to_both() {
        let baud = 100_000u32;
        let rate = 1_000_000.0f64;
        let words = uart_encode(&[0x41], baud, rate);

        let lower = Box::new(UartDecoder::new(UartConfig {
            baud_rate: baud,
            ..Default::default()
        }));
        let upper = Box::new(AsciiDecoder);
        let mut stacked = StackedDecoder::new(lower, upper);

        // Feed once, then reset, then feed again — should still produce output.
        stacked.feed(&words, 0, rate);
        stacked.reset();
        let anns = stacked.feed(&words, 0, rate);
        assert!(anns.iter().any(|a| a.data_byte == Some(0x41)));
    }
}
