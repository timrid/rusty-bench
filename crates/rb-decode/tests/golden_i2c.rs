//! Golden integration tests for the I²C decoder.

use rb_decode::{AnnotationKind, Decoder, I2cConfig, I2cDecoder};

const CPP: usize = 6;

fn encode_write(addr7: u8, data: &[u8]) -> Vec<u64> {
    // SCL = bit 0, SDA = bit 1
    let set = |scl: bool, sda: bool| -> u64 { (scl as u64) | ((sda as u64) << 1) };
    let mut words = vec![set(true, true); CPP];

    words.push(set(true, false)); // START
    words.push(set(false, false));

    let clock = |w: &mut Vec<u64>, bit: bool| {
        for _ in 0..CPP / 2 {
            w.push(set(false, bit));
        }
        for _ in 0..CPP / 2 {
            w.push(set(true, bit));
        }
        for _ in 0..CPP / 2 {
            w.push(set(false, bit));
        }
    };

    let addr_byte = (addr7 << 1) | 0;
    for b in (0..8).rev() {
        clock(&mut words, (addr_byte >> b) & 1 != 0);
    }
    clock(&mut words, false); // ACK

    for &byte in data {
        for b in (0..8).rev() {
            clock(&mut words, (byte >> b) & 1 != 0);
        }
        clock(&mut words, false); // ACK
    }

    words.push(set(false, false));
    words.push(set(true, false));
    words.push(set(true, true)); // STOP

    words.extend(vec![set(true, true); CPP]);
    words
}

#[test]
fn golden_i2c_write_single_byte() {
    let words = encode_write(0x48, &[0xAA]);
    let mut dec = I2cDecoder::new(I2cConfig::default());
    let anns = dec.feed(&words, 0, 1_000_000.0);

    // START annotation
    assert!(
        anns.iter()
            .any(|a| a.kind == AnnotationKind::Frame && a.label == "START")
    );
    // STOP annotation
    assert!(
        anns.iter()
            .any(|a| a.kind == AnnotationKind::Frame && a.label == "STOP")
    );
    // Address annotation
    let addr = anns
        .iter()
        .find(|a| a.kind == AnnotationKind::Address)
        .expect("must have Address annotation");
    assert_eq!(addr.data_byte, Some(0x48));
    assert!(addr.label.contains("W"));
    // Data annotation
    let data: Vec<u8> = anns
        .iter()
        .filter(|a| a.kind == AnnotationKind::Data)
        .filter_map(|a| a.data_byte)
        .collect();
    assert_eq!(data, vec![0xAA]);
}

#[test]
fn golden_i2c_write_multi_byte() {
    let payload = [0x10u8, 0x20, 0x30];
    let words = encode_write(0x20, &payload);
    let mut dec = I2cDecoder::new(I2cConfig::default());
    let anns = dec.feed(&words, 0, 1_000_000.0);

    let data: Vec<u8> = anns
        .iter()
        .filter(|a| a.kind == AnnotationKind::Data)
        .filter_map(|a| a.data_byte)
        .collect();
    assert_eq!(data, payload.to_vec());
}

#[test]
fn golden_i2c_annotation_order() {
    // Annotations must be ordered by range start.
    let words = encode_write(0x42, &[0x01, 0x02]);
    let mut dec = I2cDecoder::new(I2cConfig::default());
    let anns = dec.feed(&words, 0, 1_000_000.0);
    for w in anns.windows(2) {
        assert!(
            w[1].range.start >= w[0].range.start,
            "annotations not in order: {:?} vs {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn golden_i2c_json_snapshot() {
    let words = encode_write(0x42, &[0xA5]);
    let mut dec = I2cDecoder::new(I2cConfig::default());
    let anns = dec.feed(&words, 0, 1_000_000.0);
    let json = serde_json::to_string_pretty(&anns).expect("serialisation must succeed");
    assert!(json.contains("START"));
    assert!(json.contains("STOP"));
    assert!(json.contains("0x42"));
    assert!(json.contains("0xA5"));
}
