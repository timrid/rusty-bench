//! Tests for the `record` subcommand.

use rb_cli::{OutputFormat, RecordBounds, RecordOpts};

// ── CSV structural ──────────────────────────────────────────────────────────

/// Recording with --time should produce roughly the expected number of rows.
/// At 1 MHz, 500 µs → ~500 samples (+ header).
#[test]
fn record_csv_time_based_stops_after_duration() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: None,
                time: Some(0.0005), // 500 µs
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    assert!(lines.len() >= 450, "expected ~500 data rows, got {}", lines.len() - 1);
    assert!(lines.len() <= 600, "expected ~500 data rows, got {}", lines.len() - 1);
}

/// --continuous is not yet implemented and should produce a clear error.
#[test]
fn record_continuous_returns_error() {
    let result = rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Continuous,
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut Vec::new(),
    );
    let err = result.unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("not yet implemented"),
        "--continuous should say 'not yet implemented', got: {msg}"
    );
}

/// Recording with an unknown address should fail with a clear error.
#[test]
fn record_fails_for_unknown_address() {
    let result = rb_cli::run_record(
        RecordOpts {
            address: "does-not-exist:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(10),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut Vec::new(),
    );
    assert!(result.is_err(), "should error on unknown address");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("does-not-exist"),
        "error should mention the unknown address: {msg}"
    );
}

#[test]
fn record_csv_line_count_equals_samples_plus_header() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(500),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    assert_eq!(lines.len(), 501, "header + 500 data rows");
    assert!(lines[0].contains("sample_index"), "missing sample_index header");
    assert!(lines[0].contains("time_s"), "missing time_s header");
    assert!(lines[0].contains("A0"), "missing A0 header");
    assert!(lines[0].contains("D0"), "missing D0 header");
    assert!(lines[0].contains("D1"), "missing D1 header");
    assert!(lines[0].contains("D2"), "missing D2 header");
    assert!(lines[0].contains("D3"), "missing D3 header");
}

// ── CSV golden values ────────────────────────────────────────────────────────

#[test]
fn record_csv_sample_zero_is_origin() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(10),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let mut lines = s.lines();
    let _header = lines.next().unwrap();
    let first = lines.next().unwrap();
    let fields: Vec<&str> = first.split(',').collect();

    assert_eq!(fields[0], "0", "sample_index at row 0");
    assert_eq!(fields[1], "0.000000000", "time_s at sample 0");

    let a0: f64 = fields[2].parse().unwrap();
    assert!(a0.abs() < 1e-9, "A0 physical value at sample 0 should be 0, got {a0}");

    for field in &fields[3..=6] {
        assert_eq!(*field, "0", "digital channel should be 0 at sample 0");
    }
}

#[test]
fn record_csv_quarter_period_is_peak() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(251),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    let sample_250_fields: Vec<&str> = lines[251].split(',').collect();
    let a0: f64 = sample_250_fields[2].parse().unwrap();
    assert!(
        (a0 - 1.0).abs() < 1e-4,
        "A0 physical at quarter-period should be 1.0, got {a0}"
    );
}

#[test]
fn record_csv_custom_rate_changes_time_column() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(2),
                time: None,
            },
            rate: Some(500_000.0),
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let mut lines = s.lines();
    let _header = lines.next().unwrap();
    let _row0 = lines.next().unwrap();
    let row1 = lines.next().unwrap();
    let fields: Vec<&str> = row1.split(',').collect();
    assert_eq!(
        fields[1], "0.000002000",
        "time_s at sample 1 with 500 kHz rate"
    );
}

// ── VCD ─────────────────────────────────────────────────────────────────────

#[test]
fn record_vcd_has_valid_header_structure() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(100),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("$timescale"), "missing $timescale");
    assert!(s.contains("$enddefinitions $end"), "missing $enddefinitions");
    assert!(s.contains("$dumpvars"), "missing $dumpvars");
    assert!(s.contains("$var wire 1"), "missing $var wire declaration");
    assert!(s.contains("D0"), "missing D0 in VCD");
    assert!(s.contains("#0"), "missing initial timestamp #0");
}

#[test]
fn record_vcd_d0_transitions_at_first_sample() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(10),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("#1000"), "missing #1000 timestamp for D0 first edge");
}

#[test]
fn record_vcd_period_reflects_sample_rate() {
    let mut out = Vec::new();
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(4),
                time: None,
            },
            rate: Some(500_000.0),
            channels: vec![],
            config: vec![],
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("#2000"), "period should be 2000 ns at 500 kHz");
}

// ── Output file ─────────────────────────────────────────────────────────────

#[test]
fn record_csv_to_file_creates_valid_output() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join("rb_cli_test_output");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test_output.csv");
    let mut file = std::fs::File::create(&path)?;
    rb_cli::run_record(
        RecordOpts {
            address: "demo:0".into(),
            bounds: RecordBounds::Finite {
                samples: Some(50),
                time: None,
            },
            rate: None,
            channels: vec![],
            config: vec![],
            format: OutputFormat::Csv,
        },
        &mut file,
    )?;
    drop(file);

    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 51, "header + 50 data rows");
    assert!(lines[0].contains("sample_index"), "CSV header present");
    assert!(lines[0].contains("A0"), "analog channel in header");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
