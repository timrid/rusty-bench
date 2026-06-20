//! [`DeviceCapture`]: in-memory capture + native `.rbc` format (ZIP + JSON + binary blobs).

use std::io::{Read, Seek, Write};

use rb_device::DeviceInfo;
use rb_model::{AnalogChannel, AnalogFormat, ChannelId, DigitalChannel, LogicWord, Timebase};
use serde::{Deserialize, Serialize};

use crate::CaptureError;

// ── Public types ──────────────────────────────────────────────────────────────

/// A complete acquisition from a single device: all channels with their raw
/// samples, ready to serialize to the native `.rbc` format or export as
/// CSV/VCD.
#[derive(Clone, Debug)]
pub struct DeviceCapture {
    /// Device identity.
    pub info: DeviceInfo,
    /// Analog channels, in channel order.
    pub analog: Vec<AnalogCapture>,
    /// Digital channels, if the device had any.
    pub digital: Option<DigitalCapture>,
}

impl DeviceCapture {
    /// Number of samples (length of the first present series, or 0).
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.analog
            .first()
            .map(|a| a.samples.len())
            .or_else(|| self.digital.as_ref().map(|d| d.words.len()))
            .unwrap_or(0)
    }

    /// Serialize to the native `.rbc` format: a ZIP archive containing a JSON
    /// manifest and binary sample blobs.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if ZIP writing or JSON serialisation fails.
    pub fn write_to<W: Write + Seek>(&self, writer: W) -> Result<(), CaptureError> {
        write_native(self, writer)
    }

    /// Deserialize from the native `.rbc` format.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the archive is malformed or incomplete.
    pub fn read_from<R: Read + Seek>(reader: R) -> Result<Self, CaptureError> {
        read_native(reader)
    }
}

/// One analog channel with its raw samples and timebase.
#[derive(Clone, Debug)]
pub struct AnalogCapture {
    /// Channel metadata (name, scale/offset, unit).
    pub channel: AnalogChannel,
    /// Maps sample indices to time.
    pub timebase: Timebase,
    /// Raw ADC counts, one per sample.
    pub samples: Vec<i32>,
}

/// The logic group: all digital channels and their packed samples.
#[derive(Clone, Debug)]
pub struct DigitalCapture {
    /// Channel metadata (name, bit position).
    pub channels: Vec<DigitalChannel>,
    /// Maps sample indices to time.
    pub timebase: Timebase,
    /// Bit-packed logic words, one per sample.
    pub words: Vec<LogicWord>,
}

// ── Native format: ZIP + JSON manifest + binary blobs ────────────────────────

// Format version written by this implementation.
const FORMAT_VERSION: u32 = 1;

/// Serde types used only for manifest (de)serialization — keep private.
#[derive(Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    device: DeviceInfoSer,
    analog: Vec<AnalogChannelSer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digital: Option<DigitalSer>,
    sample_count: usize,
}

#[derive(Serialize, Deserialize)]
struct DeviceInfoSer {
    vendor: String,
    model: String,
    serial: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct AnalogChannelSer {
    id: u32,
    name: String,
    scale: f64,
    offset: f64,
    unit: Option<String>,
    sample_rate_hz: f64,
    start_offset_s: f64,
}

#[derive(Serialize, Deserialize)]
struct DigitalChannelSer {
    id: u32,
    name: String,
    bit: u8,
}

#[derive(Serialize, Deserialize)]
struct DigitalSer {
    channels: Vec<DigitalChannelSer>,
    sample_rate_hz: f64,
    start_offset_s: f64,
}

impl Manifest {
    fn from_capture(cap: &DeviceCapture) -> Self {
        let analog = cap
            .analog
            .iter()
            .map(|a| AnalogChannelSer {
                id: a.channel.id.0,
                name: a.channel.name.clone(),
                scale: a.channel.format.scale,
                offset: a.channel.format.offset,
                unit: a.channel.unit.clone(),
                sample_rate_hz: a.timebase.sample_rate_hz(),
                start_offset_s: a.timebase.start_offset_s(),
            })
            .collect();

        let digital = cap.digital.as_ref().map(|d| DigitalSer {
            channels: d
                .channels
                .iter()
                .map(|ch| DigitalChannelSer {
                    id: ch.id.0,
                    name: ch.name.clone(),
                    bit: ch.bit,
                })
                .collect(),
            sample_rate_hz: d.timebase.sample_rate_hz(),
            start_offset_s: d.timebase.start_offset_s(),
        });

        Manifest {
            format_version: FORMAT_VERSION,
            device: DeviceInfoSer {
                vendor: cap.info.vendor.clone(),
                model: cap.info.model.clone(),
                serial: cap.info.serial.clone(),
            },
            analog,
            digital,
            sample_count: cap.sample_count(),
        }
    }
}

fn write_native<W: Write + Seek>(cap: &DeviceCapture, writer: W) -> Result<(), CaptureError> {
    use zip::CompressionMethod;
    use zip::write::{SimpleFileOptions, ZipWriter};

    let mut zw = ZipWriter::new(writer);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // Manifest.
    let manifest_json = serde_json::to_string_pretty(&Manifest::from_capture(cap))?;
    zw.start_file("manifest.json", opts)
        .map_err(|e| CaptureError::Zip(e.to_string()))?;
    zw.write_all(manifest_json.as_bytes())?;

    // Analog blobs — one file per channel.
    for (i, acap) in cap.analog.iter().enumerate() {
        zw.start_file(format!("analog/{i}.bin"), opts)
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        for &s in &acap.samples {
            zw.write_all(&s.to_le_bytes())?;
        }
    }

    // Digital blob.
    if let Some(dcap) = &cap.digital {
        zw.start_file("digital.bin", opts)
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        for &w in &dcap.words {
            zw.write_all(&w.to_le_bytes())?;
        }
    }

    zw.finish().map_err(|e| CaptureError::Zip(e.to_string()))?;
    Ok(())
}

fn read_native<R: Read + Seek>(reader: R) -> Result<DeviceCapture, CaptureError> {
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(reader).map_err(|e| CaptureError::Zip(e.to_string()))?;

    // Read and parse manifest.
    let manifest: Manifest = {
        let mut entry = archive
            .by_name("manifest.json")
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        let mut json = String::new();
        entry.read_to_string(&mut json)?;
        serde_json::from_str(&json)?
    };

    if manifest.format_version != FORMAT_VERSION {
        return Err(CaptureError::Format(format!(
            "unsupported format version {}; expected {FORMAT_VERSION}",
            manifest.format_version
        )));
    }

    // Read all analog blobs up-front (one archive access per file).
    let mut analog_blobs: Vec<Vec<u8>> = Vec::with_capacity(manifest.analog.len());
    for i in 0..manifest.analog.len() {
        let mut entry = archive
            .by_name(&format!("analog/{i}.bin"))
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        analog_blobs.push(buf);
    }

    // Read digital blob if present.
    let digital_blob: Option<Vec<u8>> = if manifest.digital.is_some() {
        let mut entry = archive
            .by_name("digital.bin")
            .map_err(|e| CaptureError::Zip(e.to_string()))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        Some(buf)
    } else {
        None
    };

    // Reconstruct analog captures.
    let analog = manifest
        .analog
        .iter()
        .zip(analog_blobs.iter())
        .map(|(ach, blob)| {
            let samples = blob
                .chunks_exact(4)
                .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            AnalogCapture {
                channel: AnalogChannel {
                    id: ChannelId(ach.id),
                    name: ach.name.clone(),
                    format: AnalogFormat::new(ach.scale, ach.offset),
                    unit: ach.unit.clone(),
                },
                timebase: Timebase::new(ach.sample_rate_hz, ach.start_offset_s),
                samples,
            }
        })
        .collect();

    // Reconstruct digital capture.
    let digital = manifest.digital.zip(digital_blob).map(|(dsec, blob)| {
        let words = blob
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let channels = dsec
            .channels
            .iter()
            .map(|ch| DigitalChannel::new(ChannelId(ch.id), &ch.name, ch.bit))
            .collect();
        DigitalCapture {
            channels,
            timebase: Timebase::new(dsec.sample_rate_hz, dsec.start_offset_s),
            words,
        }
    });

    Ok(DeviceCapture {
        info: DeviceInfo {
            vendor: manifest.device.vendor,
            model: manifest.device.model,
            serial: manifest.device.serial,
        },
        analog,
        digital,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};

    use rb_model::{AnalogFormat, ChannelId};

    use super::*;

    fn analog_cap() -> AnalogCapture {
        AnalogCapture {
            channel: AnalogChannel::new(ChannelId(0), "A0", AnalogFormat::new(1.0 / 30_000.0, 0.0))
                .with_unit("V"),
            timebase: Timebase::new(1_000_000.0, 0.0),
            samples: vec![0, 15_000, 30_000, 15_000, 0, -15_000, -30_000, -15_000],
        }
    }

    fn digital_cap() -> DigitalCapture {
        DigitalCapture {
            channels: vec![
                DigitalChannel::new(ChannelId(1000), "D0", 0),
                DigitalChannel::new(ChannelId(1001), "D1", 1),
            ],
            timebase: Timebase::new(1_000_000.0, 0.0),
            words: vec![0b00, 0b01, 0b10, 0b11, 0b00, 0b01, 0b10, 0b11],
        }
    }

    fn write_read(cap: &DeviceCapture) -> DeviceCapture {
        let mut buf = Cursor::new(Vec::new());
        cap.write_to(&mut buf).expect("write failed");
        buf.set_position(0);
        DeviceCapture::read_from(buf).expect("read failed")
    }

    #[test]
    fn round_trip_analog_only() {
        let orig = DeviceCapture {
            info: DeviceInfo::new("TestVendor", "TestModel"),
            analog: vec![analog_cap()],
            digital: None,
        };
        let loaded = write_read(&orig);

        assert_eq!(loaded.info.vendor, "TestVendor");
        assert_eq!(loaded.info.model, "TestModel");
        assert_eq!(loaded.analog.len(), 1);
        assert_eq!(loaded.analog[0].channel.name, "A0");
        assert_eq!(loaded.analog[0].samples, orig.analog[0].samples);
        assert!(loaded.digital.is_none());
    }

    #[test]
    fn round_trip_with_digital() {
        let orig = DeviceCapture {
            info: DeviceInfo::new("RustyBench", "Demo Device"),
            analog: vec![analog_cap()],
            digital: Some(digital_cap()),
        };
        let loaded = write_read(&orig);

        let dig = loaded.digital.as_ref().unwrap();
        assert_eq!(dig.words, orig.digital.as_ref().unwrap().words);
        assert_eq!(dig.channels.len(), 2);
        assert_eq!(dig.channels[0].name, "D0");
        assert_eq!(dig.channels[1].name, "D1");
    }

    #[test]
    fn round_trip_preserves_channel_metadata() {
        let orig = DeviceCapture {
            info: DeviceInfo::new("V", "M"),
            analog: vec![analog_cap()],
            digital: Some(digital_cap()),
        };
        let loaded = write_read(&orig);

        let scale = loaded.analog[0].channel.format.scale;
        assert!((scale - 1.0 / 30_000.0).abs() < 1e-12);
        assert_eq!(loaded.analog[0].channel.unit, Some("V".to_string()));
        assert!((loaded.analog[0].timebase.sample_rate_hz() - 1_000_000.0).abs() < 1.0);
        assert_eq!(loaded.digital.unwrap().channels[1].bit, 1);
    }

    #[test]
    fn round_trip_empty_capture() {
        let orig = DeviceCapture {
            info: DeviceInfo::new("X", "Y"),
            analog: vec![],
            digital: None,
        };
        let loaded = write_read(&orig);
        assert_eq!(loaded.sample_count(), 0);
        assert!(loaded.analog.is_empty());
    }

    #[test]
    fn sample_count_uses_first_analog_then_digital() {
        let cap = DeviceCapture {
            info: DeviceInfo::new("X", "Y"),
            analog: vec![analog_cap()],   // 8 samples
            digital: Some(digital_cap()), // 8 words
        };
        assert_eq!(cap.sample_count(), 8);
    }

    #[test]
    fn read_wrong_version_is_error() {
        // Write a valid capture, then patch the manifest version.
        let cap = DeviceCapture {
            info: DeviceInfo::new("X", "Y"),
            analog: vec![],
            digital: None,
        };
        let mut buf = Cursor::new(Vec::new());
        cap.write_to(&mut buf).unwrap();
        buf.set_position(0);

        // Unpack and repack with version = 99.
        let mut archive = zip::ZipArchive::new(&mut buf).unwrap();
        let manifest_json = {
            let mut entry = archive.by_name("manifest.json").unwrap();
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            s.replace("\"format_version\": 1", "\"format_version\": 99")
        };
        drop(archive);

        let mut out = Cursor::new(Vec::new());
        {
            use zip::CompressionMethod;
            use zip::write::{SimpleFileOptions, ZipWriter};
            let mut zw = ZipWriter::new(&mut out);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zw.start_file("manifest.json", opts).unwrap();
            zw.write_all(manifest_json.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        out.set_position(0);

        let result = DeviceCapture::read_from(out);
        assert!(
            matches!(result, Err(CaptureError::Format(_))),
            "expected Format error, got: {result:?}"
        );
    }
}
