//! CSV export: one row per sample, analog as physical values, digital as bits.

use std::io;

use crate::{CaptureError, DeviceCapture};

/// Writes `capture` as CSV to `writer`.
///
/// Header: `sample_index,time_s,<analog channel names…>,<digital channel names…>`
///
/// Each data row contains the sample index, the timestamp in seconds (9 decimal
/// places), one physical-unit value per analog channel, and one `0`/`1` bit per
/// digital channel.
///
/// # Errors
/// Returns [`CaptureError::Io`] if writing fails.
pub fn write_csv(capture: &DeviceCapture, writer: &mut dyn io::Write) -> Result<(), CaptureError> {
    // Per-sample time step from the first available timebase.
    let sample_period_s = capture
        .analog
        .first()
        .map(|a| a.timebase.sample_period_s())
        .or_else(|| {
            capture
                .digital
                .as_ref()
                .map(|d| d.timebase.sample_period_s())
        })
        .unwrap_or(1.0);

    // Header row.
    write!(writer, "sample_index,time_s")?;
    for acap in &capture.analog {
        write!(writer, ",{}", acap.channel.name)?;
    }
    if let Some(dcap) = &capture.digital {
        for ch in &dcap.channels {
            write!(writer, ",{}", ch.name)?;
        }
    }
    writeln!(writer)?;

    // Data rows.
    let n = capture.sample_count();
    for i in 0..n {
        let t = i as f64 * sample_period_s;
        write!(writer, "{i},{t:.9}")?;

        for acap in &capture.analog {
            let raw = acap.samples[i];
            let phys = acap.channel.format.to_physical(raw);
            write!(writer, ",{phys:.9}")?;
        }
        if let Some(dcap) = &capture.digital {
            let word = dcap.words[i];
            for ch in &dcap.channels {
                let bit = (word >> ch.bit) & 1;
                write!(writer, ",{bit}")?;
            }
        }
        writeln!(writer)?;
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use rb_device::DeviceInfo;
    use rb_model::{AnalogChannel, AnalogFormat, ChannelId, DigitalChannel, Timebase};

    use super::*;
    use crate::{AnalogCapture, DigitalCapture};

    fn demo_capture() -> DeviceCapture {
        DeviceCapture {
            info: DeviceInfo::new("Test", "Device"),
            analog: vec![AnalogCapture {
                channel: AnalogChannel::new(
                    ChannelId(0),
                    "A0",
                    AnalogFormat::new(1.0 / 30_000.0, 0.0),
                )
                .with_unit("V"),
                timebase: Timebase::new(1_000_000.0, 0.0),
                // sin quadrature: 0, 30000 (peak), 0, −30000 (trough)
                samples: vec![0, 30_000, 0, -30_000],
            }],
            digital: Some(DigitalCapture {
                channels: vec![
                    DigitalChannel::new(ChannelId(1000), "D0", 0),
                    DigitalChannel::new(ChannelId(1001), "D1", 1),
                ],
                timebase: Timebase::new(1_000_000.0, 0.0),
                words: vec![0b00, 0b01, 0b10, 0b11],
            }),
        }
    }

    fn csv(cap: &DeviceCapture) -> String {
        let mut buf = Vec::new();
        write_csv(cap, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn csv_header_has_all_channel_names() {
        let s = csv(&demo_capture());
        let header = s.lines().next().unwrap();
        assert_eq!(header, "sample_index,time_s,A0,D0,D1");
    }

    #[test]
    fn csv_row_count_equals_samples_plus_header() {
        let s = csv(&demo_capture());
        assert_eq!(s.lines().count(), 5, "header + 4 data rows");
    }

    #[test]
    fn csv_sample_zero_is_origin() {
        let s = csv(&demo_capture());
        let row0 = s.lines().nth(1).unwrap();
        let fields: Vec<&str> = row0.split(',').collect();
        assert_eq!(fields[0], "0");
        assert_eq!(fields[1], "0.000000000");
        // A0: raw=0 → physical=0
        let a0: f64 = fields[2].parse().unwrap();
        assert!(a0.abs() < 1e-12, "expected 0.0, got {a0}");
        // D0=0, D1=0 at sample 0
        assert_eq!(fields[3], "0");
        assert_eq!(fields[4], "0");
    }

    #[test]
    fn csv_peak_sample_physical_equals_one() {
        // sample 1: raw=30_000 → 30_000 * (1/30_000) = 1.0
        let s = csv(&demo_capture());
        let row1 = s.lines().nth(2).unwrap();
        let a0: f64 = row1.split(',').nth(2).unwrap().parse().unwrap();
        assert!((a0 - 1.0).abs() < 1e-9, "expected 1.0, got {a0}");
    }

    #[test]
    fn csv_digital_bit_extraction() {
        let s = csv(&demo_capture());
        let rows: Vec<&str> = s.lines().collect();
        // sample 0: word=0b00 → D0=0,D1=0
        let r0: Vec<&str> = rows[1].split(',').collect();
        assert_eq!((r0[3], r0[4]), ("0", "0"));
        // sample 1: word=0b01 → D0=1,D1=0
        let r1: Vec<&str> = rows[2].split(',').collect();
        assert_eq!((r1[3], r1[4]), ("1", "0"));
        // sample 2: word=0b10 → D0=0,D1=1
        let r2: Vec<&str> = rows[3].split(',').collect();
        assert_eq!((r2[3], r2[4]), ("0", "1"));
        // sample 3: word=0b11 → D0=1,D1=1
        let r3: Vec<&str> = rows[4].split(',').collect();
        assert_eq!((r3[3], r3[4]), ("1", "1"));
    }

    #[test]
    fn csv_time_step_reflects_sample_rate() {
        let s = csv(&demo_capture());
        // sample 1 at 1 MHz → t = 1/1_000_000 = 0.000001 s
        let row1_time: f64 = s
            .lines()
            .nth(2)
            .unwrap()
            .split(',')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (row1_time - 1e-6).abs() < 1e-12,
            "expected 1e-6, got {row1_time}"
        );
    }

    #[test]
    fn csv_analog_only_has_no_digital_columns() {
        let cap = DeviceCapture {
            info: DeviceInfo::new("X", "Y"),
            analog: vec![AnalogCapture {
                channel: AnalogChannel::new(ChannelId(0), "CH1", AnalogFormat::identity()),
                timebase: Timebase::new(1_000.0, 0.0),
                samples: vec![100, 200],
            }],
            digital: None,
        };
        let s = csv(&cap);
        let header = s.lines().next().unwrap();
        assert_eq!(header, "sample_index,time_s,CH1");
        assert_eq!(s.lines().count(), 3);
    }
}
