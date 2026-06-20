//! Sigrok `.sr` import — logic-only (ZIP archive with INI metadata + raw bytes).
//!
//! The sigrok `.sr` format stores digital captures as a ZIP file containing:
//! - `metadata`: an INI-like key=value file that describes the device, sample
//!   rate, number of probes, unit size, and probe names.
//! - `<capturefile>-1`: raw bytes, `unitsize` bytes per sample, LSB = probe 1.
//!
//! Only logic data is supported.  The returned [`DeviceCapture`] has an empty
//! `analog` list and the digital channels ordered as `probe1..probeN`.

use std::io::{Read, Seek};

use rb_model::{ChannelId, DigitalChannel, Timebase};

use crate::capture::{AnalogCapture, DigitalCapture};
use crate::{CaptureError, DeviceCapture};

/// Imports a sigrok `.sr` logic capture.
///
/// # Errors
/// Returns [`CaptureError`] if the archive is malformed, the metadata is
/// incomplete, or if I/O fails.
pub fn read_sr<R: Read + Seek>(reader: R) -> Result<DeviceCapture, CaptureError> {
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(reader).map_err(|e| CaptureError::Zip(e.to_string()))?;

    // Parse the metadata file.
    let meta = {
        let mut entry = archive
            .by_name("metadata")
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        s
    };
    let md = parse_metadata(&meta)?;

    // Read the raw logic bytes.
    let logic_file = format!("{}-1", md.capturefile);
    let raw_bytes = {
        let mut entry = archive
            .by_name(&logic_file)
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };

    // Convert raw bytes to LogicWords.  Each sample is `unitsize` bytes (LE),
    // padded to u64.  probe N (1-based) lives at bit N-1.
    let words: Vec<u64> = if md.unitsize == 0 || md.unitsize > 8 {
        return Err(CaptureError::Format(format!(
            "unsupported unitsize {} (expected 1–8)",
            md.unitsize
        )));
    } else {
        raw_bytes
            .chunks_exact(md.unitsize)
            .map(|chunk| {
                let mut word = 0u64;
                for (b, &byte) in chunk.iter().enumerate() {
                    word |= (byte as u64) << (8 * b);
                }
                word
            })
            .collect()
    };

    let channels: Vec<DigitalChannel> = md
        .probes
        .into_iter()
        .enumerate()
        .map(|(i, name)| DigitalChannel::new(ChannelId(i as u32), name, i as u8))
        .collect();

    let digital = DigitalCapture {
        channels,
        timebase: Timebase::new(md.sample_rate_hz, 0.0),
        words,
    };

    Ok(DeviceCapture {
        info: rb_device::DeviceInfo::new("sigrok", &md.device_model),
        analog: Vec::<AnalogCapture>::new(),
        digital: Some(digital),
    })
}

// ── Metadata parser ───────────────────────────────────────────────────────────

struct SrMetadata {
    device_model: String,
    sample_rate_hz: f64,
    capturefile: String,
    unitsize: usize,
    /// Probe names in order: index 0 = probe1.
    probes: Vec<String>,
}

/// Parse the sigrok INI-like metadata text.
fn parse_metadata(text: &str) -> Result<SrMetadata, CaptureError> {
    // Collect `key = value` pairs from the `[device 1]` section (case-insensitive).
    let mut in_device = false;
    let mut kv: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_device = line.to_lowercase().starts_with("[device");
            continue;
        }
        if in_device {
            if let Some((k, v)) = line.split_once('=') {
                kv.push((k.trim().to_lowercase(), v.trim().to_string()));
            }
        }
    }

    let get =
        |key: &str| -> Option<&str> { kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()) };

    let capturefile = get("capturefile")
        .ok_or_else(|| CaptureError::Format("missing capturefile in metadata".into()))?
        .to_string();

    let rate_str = get("samplerate")
        .ok_or_else(|| CaptureError::Format("missing samplerate in metadata".into()))?;
    let sample_rate_hz = parse_rate(rate_str)?;

    let unitsize: usize = get("unitsize")
        .ok_or_else(|| CaptureError::Format("missing unitsize in metadata".into()))?
        .parse()
        .map_err(|_| CaptureError::Format("unitsize is not a valid integer".into()))?;

    let total_probes: usize = get("total probes")
        .ok_or_else(|| CaptureError::Format("missing 'total probes' in metadata".into()))?
        .parse()
        .map_err(|_| CaptureError::Format("total probes is not a valid integer".into()))?;

    if total_probes > 64 {
        return Err(CaptureError::Format(format!(
            "too many probes: {total_probes} (max 64)"
        )));
    }

    // Collect probe names in order.  Missing names default to "D{i}".
    let mut probes = Vec::with_capacity(total_probes);
    for i in 1..=total_probes {
        let name = get(&format!("probe{i}"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("D{}", i - 1));
        probes.push(name);
    }

    // Optional device description.
    let device_model = get("model")
        .or_else(|| get("description"))
        .unwrap_or("unknown")
        .to_string();

    Ok(SrMetadata {
        device_model,
        sample_rate_hz,
        capturefile,
        unitsize,
        probes,
    })
}

/// Parse a sigrok sample-rate string like `"1 MHz"`, `"500 kHz"`, `"10 MS/s"`.
fn parse_rate(s: &str) -> Result<f64, CaptureError> {
    let s = s.trim();
    // Try splitting into numeric part and unit.
    let split_idx = s
        .find(|c: char| c.is_alphabetic() || c == '/')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(split_idx);
    let num: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| CaptureError::Format(format!("cannot parse sample rate: {s:?}")))?;

    let unit = unit_str.trim().to_lowercase();
    let multiplier = match unit.as_str() {
        "" | "hz" | "s/s" => 1.0,
        "khz" | "ks/s" => 1_000.0,
        "mhz" | "ms/s" => 1_000_000.0,
        "ghz" | "gs/s" => 1_000_000_000.0,
        other => {
            return Err(CaptureError::Format(format!(
                "unknown sample-rate unit: {other:?}"
            )));
        }
    };

    Ok(num * multiplier)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::CompressionMethod;
    use zip::write::{SimpleFileOptions, ZipWriter};

    use super::*;

    /// Build a minimal in-memory `.sr` archive.
    fn make_sr(metadata: &str, logic_bytes: &[u8]) -> Cursor<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

            zw.start_file("metadata", opts).unwrap();
            zw.write_all(metadata.as_bytes()).unwrap();

            // The capturefile in the metadata must be "logic-1"; the actual
            // zip entry is "logic-1-1".
            zw.start_file("logic-1-1", opts).unwrap();
            zw.write_all(logic_bytes).unwrap();

            zw.finish().unwrap();
        }
        buf.set_position(0);
        buf
    }

    const META_4CH: &str = "\
[global]
sigrok version=0.3.0

[device 1]
capturefile=logic-1
total probes=4
samplerate=1 MHz
unitsize=1
probe1=CLK
probe2=MOSI
probe3=MISO
probe4=CS
";

    #[test]
    fn sr_reads_channel_names_and_count() {
        // Logic: 2 samples, all zeros.
        let sr = make_sr(META_4CH, &[0x00, 0x00]);
        let cap = read_sr(sr).unwrap();

        let dig = cap.digital.as_ref().unwrap();
        assert_eq!(dig.channels.len(), 4);
        assert_eq!(dig.channels[0].name, "CLK");
        assert_eq!(dig.channels[1].name, "MOSI");
        assert_eq!(dig.channels[2].name, "MISO");
        assert_eq!(dig.channels[3].name, "CS");
    }

    #[test]
    fn sr_reads_sample_rate() {
        let sr = make_sr(META_4CH, &[0x00]);
        let cap = read_sr(sr).unwrap();
        assert!(
            (cap.digital.as_ref().unwrap().timebase.sample_rate_hz() - 1_000_000.0).abs() < 1.0
        );
    }

    #[test]
    fn sr_converts_bytes_to_logic_words() {
        // 1 sample: byte = 0b0101 → D0=1, D1=0, D2=1, D3=0
        let sr = make_sr(META_4CH, &[0b0101]);
        let cap = read_sr(sr).unwrap();
        let word = cap.digital.as_ref().unwrap().words[0];
        assert_eq!(word & 1, 1, "D0 (bit 0) should be 1");
        assert_eq!((word >> 1) & 1, 0, "D1 (bit 1) should be 0");
        assert_eq!((word >> 2) & 1, 1, "D2 (bit 2) should be 1");
        assert_eq!((word >> 3) & 1, 0, "D3 (bit 3) should be 0");
    }

    #[test]
    fn sr_binary_counter_logic() {
        // 4 samples: binary counter 0,1,2,3 — D0 toggles every sample.
        let sr = make_sr(META_4CH, &[0x00, 0x01, 0x02, 0x03]);
        let cap = read_sr(sr).unwrap();
        let words = &cap.digital.as_ref().unwrap().words;
        assert_eq!(words.len(), 4);
        assert_eq!(words[0], 0b0000);
        assert_eq!(words[1], 0b0001);
        assert_eq!(words[2], 0b0010);
        assert_eq!(words[3], 0b0011);
    }

    #[test]
    fn sr_unitsize_2_reads_16_bit_words() {
        let meta = "\
[device 1]
capturefile=logic-1
total probes=16
samplerate=500 kHz
unitsize=2
";
        // One sample: 0x00F0 → bits 4-7 high.
        let sr = make_sr(meta, &[0xF0, 0x00]); // LE: byte0=0xF0, byte1=0x00
        let cap = read_sr(sr).unwrap();
        let word = cap.digital.as_ref().unwrap().words[0];
        // bits 4-7 should be set (0xF0 in the low byte)
        assert_eq!(word & 0xFF, 0xF0, "low byte should be 0xF0");
        assert_eq!((word >> 8) & 0xFF, 0x00, "high byte should be 0");
    }

    #[test]
    fn sr_parses_khz_sample_rate() {
        let meta = "\
[device 1]
capturefile=logic-1
total probes=1
samplerate=500 kHz
unitsize=1
probe1=D0
";
        let sr = make_sr(meta, &[0x00]);
        let cap = read_sr(sr).unwrap();
        let rate = cap.digital.as_ref().unwrap().timebase.sample_rate_hz();
        assert!(
            (rate - 500_000.0).abs() < 1.0,
            "expected 500 kHz, got {rate}"
        );
    }

    #[test]
    fn sr_parses_mhz_sample_rate() {
        let sr = make_sr(META_4CH, &[0x00]);
        let rate = read_sr(sr)
            .unwrap()
            .digital
            .unwrap()
            .timebase
            .sample_rate_hz();
        assert!((rate - 1_000_000.0).abs() < 1.0);
    }

    #[test]
    fn sr_missing_capturefile_is_error() {
        let meta = "\
[device 1]
total probes=1
samplerate=1 MHz
unitsize=1
";
        // Still provide the logic bytes so the zip is valid.
        let sr = make_sr(meta, &[0x00]);
        assert!(matches!(read_sr(sr), Err(CaptureError::Format(_))));
    }

    #[test]
    fn sr_analog_field_is_empty() {
        let sr = make_sr(META_4CH, &[0x01, 0x02]);
        let cap = read_sr(sr).unwrap();
        assert!(
            cap.analog.is_empty(),
            "sr import should not produce analog channels"
        );
    }
}
