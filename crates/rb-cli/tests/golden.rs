//! End-to-end golden tests for the `rb-cli` commands.
//!
//! These tests call the public command functions directly (not via subprocess)
//! and assert both structural invariants and exact deterministic values produced
//! by the Demo Device.

use rb_cli::{AcquireOpts, OutputFormat};

// ── scan ──────────────────────────────────────────────────────────────────────

#[test]
fn scan_finds_demo_device() {
    let mut out = Vec::new();
    rb_cli::run_scan(&mut out).unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("[demo]"), "missing driver tag");
    assert!(s.contains("demo:0"), "missing demo address");
    assert!(s.contains("RustyBench"), "missing vendor");
    assert!(s.contains("Demo Device"), "missing model");
}

// ── connect ───────────────────────────────────────────────────────────────────

#[test]
fn connect_confirms_single_line() {
    let mut out = Vec::new();
    rb_cli::run_connect("demo:0", &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("Connected"), "missing confirmation");
    assert!(s.contains("Demo Device"), "missing model name");
    assert!(s.contains("demo:0"), "missing address in confirmation");
}

#[test]
fn connect_fails_for_unknown_address() {
    let result = rb_cli::run_connect("demo:99", &mut Vec::new());
    assert!(result.is_err(), "should error on unknown address");
}

// ── info ──────────────────────────────────────────────────────────────────────

#[test]
fn info_prints_capabilities() {
    let mut out = Vec::new();
    rb_cli::run_info("demo:0", &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("Demo Device"), "missing model");
    assert!(s.contains("Oscilloscope"), "missing Oscilloscope class");
    assert!(s.contains("LogicAnalyzer"), "missing LogicAnalyzer class");
    assert!(s.contains("A0"), "missing analog channel A0");
    assert!(s.contains("D0"), "missing digital channel D0");
    assert!(s.contains("1000000"), "missing sample rate");
}

// ── acquire: CSV structural invariants ────────────────────────────────────────

#[test]
fn acquire_csv_line_count_equals_samples_plus_header() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 500,
            rate: None,
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    assert_eq!(lines.len(), 501, "header + 500 data rows");
    assert!(
        lines[0].contains("sample_index"),
        "missing sample_index header"
    );
    assert!(lines[0].contains("time_s"), "missing time_s header");
    assert!(lines[0].contains("A0"), "missing A0 header");
    assert!(lines[0].contains("D0"), "missing D0 header");
    assert!(lines[0].contains("D1"), "missing D1 header");
    assert!(lines[0].contains("D2"), "missing D2 header");
    assert!(lines[0].contains("D3"), "missing D3 header");
}

// ── acquire: CSV golden values ────────────────────────────────────────────────

/// DemoDevice defaults: 1 MHz rate, 1 kHz sine, amplitude 30 000, 4 digital ch.
/// At sample 0: sin(0) = 0 → raw = 0 → physical = 0; all digital bits = 0.
#[test]
fn acquire_csv_sample_zero_is_origin() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 10,
            rate: None,
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
    assert!(
        a0.abs() < 1e-9,
        "A0 physical value at sample 0 should be 0, got {a0}"
    );

    // All four digital channels are 0 at sample 0 (binary counter = 0).
    for field in &fields[3..=6] {
        assert_eq!(*field, "0", "digital channel should be 0 at sample 0");
    }
}

/// At the quarter-period (sample 250): sin(2π·1000·250/1 000 000) = sin(π/2) = 1.0
/// → raw = 30 000 → physical = 30 000 × (1/30 000) = 1.0.
#[test]
fn acquire_csv_quarter_period_is_peak() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 251, // need sample index 250
            rate: None,
            format: OutputFormat::Csv,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = s.lines().collect();

    // lines[0] = header, lines[1] = sample 0, lines[251] = sample 250.
    let sample_250_fields: Vec<&str> = lines[251].split(',').collect();
    let a0: f64 = sample_250_fields[2].parse().unwrap();
    assert!(
        (a0 - 1.0).abs() < 1e-4,
        "A0 physical at quarter-period should be 1.0, got {a0}"
    );
}

/// Sample rate override: acquire 100 samples at 500 kHz.
/// The time of sample 1 should be 1/500_000 s = 0.000002000 s.
#[test]
fn acquire_csv_custom_rate_changes_time_column() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 2,
            rate: Some(500_000.0),
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
    // 1 / 500_000 = 0.000002 s → formatted as "0.000002000"
    assert_eq!(
        fields[1], "0.000002000",
        "time_s at sample 1 with 500 kHz rate"
    );
}

// ── acquire: VCD structural invariants ────────────────────────────────────────

#[test]
fn acquire_vcd_has_valid_header_structure() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 100,
            rate: None,
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("$timescale"), "missing $timescale");
    assert!(
        s.contains("$enddefinitions $end"),
        "missing $enddefinitions"
    );
    assert!(s.contains("$dumpvars"), "missing $dumpvars");
    assert!(s.contains("$var wire 1"), "missing $var wire declaration");
    assert!(s.contains("D0"), "missing D0 in VCD");
    assert!(s.contains("#0"), "missing initial timestamp #0");
}

/// D0 (bit 0 of the binary counter) toggles every sample.
/// At 1 MHz, sample 1 → timestamp 1 × 1000 ns = #1000.
#[test]
fn acquire_vcd_d0_transitions_at_first_sample() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 10,
            rate: None,
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();

    // D0 transitions from 0→1 at sample 1 → timestamp 1000 ns.
    assert!(
        s.contains("#1000"),
        "missing #1000 timestamp for D0 first edge"
    );
}

/// VCD for an acquire at a different rate uses the correct period.
#[test]
fn acquire_vcd_period_reflects_sample_rate() {
    let mut out = Vec::new();
    rb_cli::run_acquire(
        AcquireOpts {
            address: "demo:0".into(),
            samples: 4,
            rate: Some(500_000.0), // period = 2000 ns
            format: OutputFormat::Vcd,
        },
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    // Sample 1 at 500 kHz → timestamp 1 × 2000 ns = #2000.
    assert!(
        s.contains("#2000"),
        "period should be 2000 ns at 500 kHz; got VCD:\n{s}"
    );
}
