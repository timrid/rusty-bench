//! Golden integration tests for the UART decoder.
//!
//! Each test synthesises a known waveform, decodes it, and asserts that the
//! annotation sequence matches the expected protocol structure.  The tests use
//! `serde_json` to serialise annotations to JSON so the output can be compared
//! as a stable, human-readable snapshot.

use rb_decode::{AnnotationKind, Decoder, Parity, UartConfig, UartDecoder};

fn uart_encode(bytes: &[u8], baud: u32, rate_hz: f64, rx_bit: u8) -> Vec<u64> {
    let spp = (rate_hz / baud as f64).round() as usize;
    let bit_mask = 1u64 << rx_bit;
    let mut words = vec![bit_mask; 5];
    for &byte in bytes {
        words.extend(vec![0u64; spp]);
        for b in 0..8u8 {
            let bit = (byte >> b) & 1;
            words.extend(vec![(bit as u64) << rx_bit; spp]);
        }
        words.extend(vec![bit_mask; spp]);
    }
    words.extend(vec![bit_mask; 5]);
    words
}

#[test]
fn golden_uart_ascii_hello() {
    let baud = 100_000u32;
    let rate = 1_000_000.0f64;
    let msg = b"Hello";
    let words = uart_encode(msg, baud, rate, 0);

    let mut dec = UartDecoder::new(UartConfig {
        baud_rate: baud,
        ..Default::default()
    });
    let anns = dec.feed(&words, 0, rate);

    // Exactly one annotation per byte.
    assert_eq!(anns.len(), msg.len(), "annotation count mismatch");

    // All annotations are Data.
    assert!(anns.iter().all(|a| a.kind == AnnotationKind::Data));

    // Decoded bytes match input.
    let decoded: Vec<u8> = anns.iter().filter_map(|a| a.data_byte).collect();
    assert_eq!(decoded, msg.to_vec());

    // Labels are hex strings.
    assert_eq!(anns[0].label, "0x48"); // 'H'
    assert_eq!(anns[1].label, "0x65"); // 'e'
    assert_eq!(anns[4].label, "0x6F"); // 'o'

    // Annotations are in strictly increasing order.
    for w in anns.windows(2) {
        assert!(
            w[1].range.start > w[0].range.start,
            "annotations not ordered"
        );
    }

    // Ranges are non-empty.
    for ann in &anns {
        assert!(ann.range.end > ann.range.start, "empty annotation range");
    }
}

#[test]
fn golden_uart_8e1_no_errors() {
    // 8-bit even parity: encode bytes and supply correct parity bits manually.
    let baud = 100_000u32;
    let rate = 1_000_000.0f64;
    let spp = (rate / baud as f64).round() as usize;
    let half = spp / 2;

    let test_bytes = [0x00u8, 0xFF, 0x55, 0xAA];
    let mut words = vec![1u64; 5]; // idle

    for &byte in &test_bytes {
        let ones = byte.count_ones();
        let parity_bit = (ones % 2) as u64; // even parity: bit = 0 when ones is even

        words.extend(vec![0u64; spp]); // start
        for b in 0..8u8 {
            let bit = ((byte >> b) & 1) as u64;
            words.extend(vec![bit; spp]);
        }
        words.extend(vec![parity_bit; spp]); // parity
        words.extend(vec![1u64; spp + half + 1]); // stop + inter-frame gap
    }

    let mut dec = UartDecoder::new(UartConfig {
        baud_rate: baud,
        data_bits: 8,
        parity: Parity::Even,
        ..Default::default()
    });
    let anns = dec.feed(&words, 0, rate);

    let data: Vec<u8> = anns
        .iter()
        .filter(|a| a.kind == AnnotationKind::Data)
        .filter_map(|a| a.data_byte)
        .collect();
    assert_eq!(
        data,
        test_bytes.to_vec(),
        "all bytes should decode without parity errors"
    );
}

#[test]
fn golden_uart_json_snapshot() {
    let baud = 100_000u32;
    let rate = 1_000_000.0f64;
    let words = uart_encode(&[0x41, 0x42], baud, rate, 0); // "AB"

    let mut dec = UartDecoder::new(UartConfig {
        baud_rate: baud,
        ..Default::default()
    });
    let anns = dec.feed(&words, 0, rate);

    let json = serde_json::to_string_pretty(&anns).expect("serialisation must succeed");
    // Verify structural properties of the JSON snapshot.
    assert!(json.contains("\"0x41\""), "label 0x41 must appear in JSON");
    assert!(json.contains("\"0x42\""), "label 0x42 must appear in JSON");
    assert!(json.contains("\"Data\""), "kind Data must appear in JSON");
    assert!(!json.contains("Error"), "no Error annotations expected");
}
