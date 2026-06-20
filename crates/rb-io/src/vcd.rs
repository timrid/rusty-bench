//! VCD (Value Change Dump, IEEE 1364) export — digital channels only.

use std::io;

use crate::{CaptureError, DeviceCapture};

/// Writes digital channels from `capture` as a VCD file to `writer`.
///
/// Only value *changes* are emitted; timestamps are in nanoseconds derived from
/// the digital-trace timebase.  Analog channels are omitted (VCD is a digital
/// format).
///
/// VCD identifiers use printable ASCII starting at `!` (0x21), one per channel,
/// allowing up to 94 channels in a single-character identifier scheme.
///
/// # Errors
/// Returns [`CaptureError`] if the capture has no digital channels or if
/// writing fails.
pub fn write_vcd(capture: &DeviceCapture, writer: &mut dyn io::Write) -> Result<(), CaptureError> {
    let dcap = capture.digital.as_ref().ok_or_else(|| {
        CaptureError::Format("VCD format requires at least one digital channel".into())
    })?;

    let period_ns = (1e9 / dcap.timebase.sample_rate_hz()).round() as u64;
    let channels = &dcap.channels;

    // VCD identifiers: printable ASCII '!' (33) through '~' (126).
    let ids: Vec<char> = channels
        .iter()
        .enumerate()
        .map(|(i, _)| {
            char::from_u32(33 + i as u32)
                .expect("at most 94 channels in single-char VCD identifier scheme")
        })
        .collect();

    // Header.
    writeln!(writer, "$timescale 1 ns $end")?;
    writeln!(writer, "$version RustyBench-IO $end")?;
    writeln!(writer, "$scope module top $end")?;
    for (ch, id) in channels.iter().zip(&ids) {
        writeln!(writer, "$var wire 1 {id} {} $end", ch.name)?;
    }
    writeln!(writer, "$upscope $end")?;
    writeln!(writer, "$enddefinitions $end")?;

    // Initial values at t = 0.
    let words = &dcap.words;
    writeln!(writer, "#0")?;
    writeln!(writer, "$dumpvars")?;
    if !words.is_empty() {
        for (ch, id) in channels.iter().zip(&ids) {
            let bit = (words[0] >> ch.bit) & 1;
            writeln!(writer, "{bit}{id}")?;
        }
    }
    writeln!(writer, "$end")?;

    // Value changes — only emit when at least one channel transitions.
    let mut prev = words.first().copied().unwrap_or(0);
    for (i, &word) in words.iter().enumerate().skip(1) {
        if word != prev {
            writeln!(writer, "#{}", i as u64 * period_ns)?;
            for (ch, id) in channels.iter().zip(&ids) {
                let pb = (prev >> ch.bit) & 1;
                let cb = (word >> ch.bit) & 1;
                if pb != cb {
                    writeln!(writer, "{cb}{id}")?;
                }
            }
            prev = word;
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use rb_device::DeviceInfo;
    use rb_model::{ChannelId, DigitalChannel, Timebase};

    use super::*;
    use crate::{AnalogCapture, DigitalCapture};

    fn binary_counter_capture(samples: usize, rate_hz: f64) -> DeviceCapture {
        DeviceCapture {
            info: DeviceInfo::new("Test", "Device"),
            analog: vec![],
            digital: Some(DigitalCapture {
                channels: vec![
                    DigitalChannel::new(ChannelId(0), "D0", 0),
                    DigitalChannel::new(ChannelId(1), "D1", 1),
                    DigitalChannel::new(ChannelId(2), "D2", 2),
                    DigitalChannel::new(ChannelId(3), "D3", 3),
                ],
                timebase: Timebase::new(rate_hz, 0.0),
                words: (0..samples as u64).map(|i| i & 0x0F).collect(),
            }),
        }
    }

    fn vcd(cap: &DeviceCapture) -> String {
        let mut buf = Vec::new();
        write_vcd(cap, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn vcd_header_contains_required_sections() {
        let s = vcd(&binary_counter_capture(10, 1_000_000.0));
        assert!(s.contains("$timescale"), "missing $timescale");
        assert!(
            s.contains("$enddefinitions $end"),
            "missing $enddefinitions"
        );
        assert!(s.contains("$dumpvars"), "missing $dumpvars");
        assert!(s.contains("$var wire 1"), "missing $var wire");
    }

    #[test]
    fn vcd_declares_all_channels() {
        let s = vcd(&binary_counter_capture(10, 1_000_000.0));
        assert!(s.contains("D0"), "missing D0");
        assert!(s.contains("D1"), "missing D1");
        assert!(s.contains("D2"), "missing D2");
        assert!(s.contains("D3"), "missing D3");
    }

    #[test]
    fn vcd_initial_values_at_t0_are_zero() {
        // Binary counter starts at 0, so all channels are initially 0.
        let s = vcd(&binary_counter_capture(10, 1_000_000.0));
        // After #0 / $dumpvars, all four channels should be 0.
        let dumpvars_block = s
            .split("$dumpvars")
            .nth(1)
            .unwrap()
            .split("$end")
            .next()
            .unwrap();
        for id in ['!', '"', '#', '$'] {
            assert!(
                dumpvars_block.contains(&format!("0{id}")),
                "expected initial 0 for {id}"
            );
        }
    }

    /// D0 (bit 0) is 0 at sample 0 and 1 at sample 1.  At 1 MHz the sample
    /// period is 1000 ns, so the first edge should appear at timestamp #1000.
    #[test]
    fn vcd_d0_first_edge_at_one_sample_period() {
        let s = vcd(&binary_counter_capture(10, 1_000_000.0));
        assert!(
            s.contains("#1000"),
            "expected edge at #1000 (1 sample @ 1 MHz)"
        );
    }

    #[test]
    fn vcd_period_scales_with_sample_rate() {
        // At 500 kHz the period is 2000 ns.
        let s = vcd(&binary_counter_capture(4, 500_000.0));
        assert!(
            s.contains("#2000"),
            "expected #2000 for 500 kHz sample rate"
        );
    }

    #[test]
    fn vcd_only_changed_channels_in_each_timestamp_block() {
        // sample 1 → word = 1 (only D0 changes 0→1; D1,D2,D3 stay 0).
        let s = vcd(&binary_counter_capture(3, 1_000_000.0));
        // Find the #1000 block.
        let block = s.split("#1000").nth(1).unwrap().split('#').next().unwrap();
        assert!(block.contains("1!"), "D0 should transition to 1 at #1000");
        // D1 identifier is '"'; it should NOT appear in the #1000 block.
        assert!(!block.contains('"'), "D1 should not appear at #1000");
    }

    #[test]
    fn vcd_no_digital_channels_returns_error() {
        let cap = DeviceCapture {
            info: DeviceInfo::new("X", "Y"),
            analog: vec![AnalogCapture {
                channel: rb_model::AnalogChannel::new(
                    ChannelId(0),
                    "A0",
                    rb_model::AnalogFormat::identity(),
                ),
                timebase: Timebase::new(1_000.0, 0.0),
                samples: vec![1, 2, 3],
            }],
            digital: None,
        };
        assert!(
            write_vcd(&cap, &mut Vec::new()).is_err(),
            "expected error for analog-only capture"
        );
    }
}
