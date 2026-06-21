//! Golden integration tests for the SPI decoder.

use rb_decode::{AnnotationKind, Decoder, SpiConfig, SpiDecoder};

const CPP: usize = 4;

/// Encode SPI Mode-0 transfers: CLK=bit0, MOSI=bit1, MISO=bit2, CS=bit3.
fn encode_mode0(mosi_bytes: &[u8], miso_bytes: &[u8]) -> Vec<u64> {
    let idle = 1u64 << 3; // CS=1
    let set = |cs: bool, clk: bool, mo: bool, mi: bool| -> u64 {
        ((cs as u64) << 3) | ((clk as u64) << 0) | ((mo as u64) << 1) | ((mi as u64) << 2)
    };

    let mut words = vec![idle; CPP];

    // Assert CS
    for _ in 0..CPP {
        words.push(set(false, false, false, false));
    }

    let n = mosi_bytes.len().max(miso_bytes.len());
    for idx in 0..n {
        let mo_byte = *mosi_bytes.get(idx).unwrap_or(&0);
        let mi_byte = *miso_bytes.get(idx).unwrap_or(&0);
        for b in (0..8).rev() {
            let mo = (mo_byte >> b) & 1 != 0;
            let mi = (mi_byte >> b) & 1 != 0;
            for _ in 0..CPP {
                words.push(set(false, false, mo, mi)); // CLK low
            }
            for _ in 0..CPP {
                words.push(set(false, true, mo, mi)); // CLK high (sample)
            }
        }
    }

    // De-assert CS
    for _ in 0..CPP {
        words.push(set(false, false, false, false));
    }
    for _ in 0..CPP {
        words.push(idle);
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
fn golden_spi_single_byte_round_trip() {
    let words = encode_mode0(&[0xA5], &[0x5A]);
    let mut dec = SpiDecoder::new(default_cfg());
    let anns = dec.feed(&words, 0, 1_000_000.0);

    let mosi: Vec<u8> = anns
        .iter()
        .filter(|a| a.label.starts_with("MOSI"))
        .filter_map(|a| a.data_byte)
        .collect();
    let miso: Vec<u8> = anns
        .iter()
        .filter(|a| a.label.starts_with("MISO"))
        .filter_map(|a| a.data_byte)
        .collect();

    assert_eq!(mosi, vec![0xA5]);
    assert_eq!(miso, vec![0x5A]);
}

#[test]
fn golden_spi_multi_byte() {
    let mosi = [0x01u8, 0x02, 0x03];
    let miso = [0xFEu8, 0xFD, 0xFC];
    let words = encode_mode0(&mosi, &miso);
    let mut dec = SpiDecoder::new(default_cfg());
    let anns = dec.feed(&words, 0, 1_000_000.0);

    let got_mosi: Vec<u8> = anns
        .iter()
        .filter(|a| a.label.starts_with("MOSI"))
        .filter_map(|a| a.data_byte)
        .collect();
    let got_miso: Vec<u8> = anns
        .iter()
        .filter(|a| a.label.starts_with("MISO"))
        .filter_map(|a| a.data_byte)
        .collect();

    assert_eq!(got_mosi, mosi.to_vec());
    assert_eq!(got_miso, miso.to_vec());
}

#[test]
fn golden_spi_cs_frames_present() {
    let words = encode_mode0(&[0xFF], &[0x00]);
    let mut dec = SpiDecoder::new(default_cfg());
    let anns = dec.feed(&words, 0, 1_000_000.0);
    assert!(anns.iter().any(|a| a.kind == AnnotationKind::Frame));
}

#[test]
fn golden_spi_json_snapshot() {
    let words = encode_mode0(&[0xDE, 0xAD], &[0xBE, 0xEF]);
    let mut dec = SpiDecoder::new(default_cfg());
    let anns = dec.feed(&words, 0, 1_000_000.0);
    let json = serde_json::to_string_pretty(&anns).expect("serialisation must succeed");
    assert!(json.contains("MOSI:0xDE"));
    assert!(json.contains("MOSI:0xAD"));
    assert!(json.contains("MISO:0xBE"));
    assert!(json.contains("MISO:0xEF"));
}
