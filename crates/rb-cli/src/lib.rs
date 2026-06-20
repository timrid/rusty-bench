//! RustyBench CLI core logic — scan, info, connect, acquire and output formatters.
//!
//! All public functions write their output to a `&mut dyn io::Write`, so tests
//! can capture the output in a `Vec<u8>` without spawning a subprocess.

#![forbid(unsafe_code)]

use std::io;

use anyhow::Context as _;
use futures::executor::block_on;
use rb_core::{AcquisitionCommand, DeviceHandle, DriverRegistry};
use rb_device::Device;

// ── Public types ──────────────────────────────────────────────────────────────

/// Output format for the `acquire` subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Comma-separated values — analog and digital, one row per sample.
    Csv,
    /// Value Change Dump (IEEE 1364) — digital channels only.
    Vcd,
}

/// Options forwarded from the `acquire` subcommand.
pub struct AcquireOpts {
    /// Opaque device address (e.g. `"demo:0"`).
    pub address: String,
    /// Number of samples to acquire.
    pub samples: usize,
    /// Override the device's default sample rate, in hertz.
    pub rate: Option<f64>,
    /// Output format.
    pub format: OutputFormat,
}

// ── Public command functions ───────────────────────────────────────────────────

/// Scans all registered drivers and prints one line per reachable device.
///
/// # Errors
/// Propagates any driver scan error.
pub fn run_scan(writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let registry = DriverRegistry::with_default_factories();
    let results = block_on(registry.scan_all())?;
    if results.is_empty() {
        writeln!(writer, "No devices found.")?;
    } else {
        for r in &results {
            writeln!(
                writer,
                "[{}] {}/{} @ {}",
                r.driver, r.candidate.info.vendor, r.candidate.info.model, r.candidate.address
            )?;
        }
    }
    Ok(())
}

/// Connects to `address` and prints a one-line confirmation.
///
/// # Errors
/// Returns an error if the device cannot be found or connected.
pub fn run_connect(address: &str, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let device = find_and_connect(address)?;
    let info = device.info();
    writeln!(
        writer,
        "Connected to {}/{} @ {address}",
        info.vendor, info.model
    )?;
    Ok(())
}

/// Connects to `address` and prints full capability details.
///
/// # Errors
/// Returns an error if the device cannot be found or connected.
pub fn run_info(address: &str, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let device = find_and_connect(address)?;
    let info = device.info();

    writeln!(writer, "Vendor:  {}", info.vendor)?;
    writeln!(writer, "Model:   {}", info.model)?;
    if let Some(serial) = &info.serial {
        writeln!(writer, "Serial:  {serial}")?;
    }
    writeln!(writer, "Address: {address}")?;
    let class_names: Vec<String> = device.classes().iter().map(|c| format!("{c:?}")).collect();
    writeln!(writer, "Classes: {}", class_names.join(", "))?;

    if let Some(scope) = device.as_oscilloscope() {
        writeln!(writer, "\nOscilloscope:")?;
        writeln!(writer, "  Sample rate: {} Hz", scope.sample_rate_hz())?;
        for ch in scope.channels() {
            let unit = ch.unit.as_deref().unwrap_or("?");
            writeln!(
                writer,
                "  Channel {}: {} (scale={} offset={} unit={unit})",
                ch.id.0, ch.name, ch.format.scale, ch.format.offset
            )?;
        }
    }

    if let Some(la) = device.as_logic_analyzer() {
        writeln!(writer, "\nLogic Analyzer:")?;
        writeln!(writer, "  Sample rate: {} Hz", la.sample_rate_hz())?;
        for ch in la.channels() {
            writeln!(
                writer,
                "  Channel {}: {} (bit {})",
                ch.id.0, ch.name, ch.bit
            )?;
        }
    }

    Ok(())
}

/// Connects to `address`, acquires `opts.samples` samples, and writes the
/// capture to `writer` in the requested format.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or if the
/// requested format is incompatible with the device's capabilities.
pub fn run_acquire(opts: AcquireOpts, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let device = find_and_connect(&opts.address)?;
    let mut handle = DeviceHandle::new(device);

    if let Some(rate) = opts.rate {
        block_on(handle.apply(AcquisitionCommand::SetSampleRate(rate)))?;
    }

    // Validate format before starting acquisition.
    if opts.format == OutputFormat::Vcd && handle.digital_trace().is_none() {
        anyhow::bail!(
            "VCD format requires a device with digital channels, but {} has none",
            opts.address
        );
    }

    if opts.samples > 0 {
        block_on(handle.apply(AcquisitionCommand::Start))?;
        while handle.sample_count() < opts.samples {
            let remaining = opts.samples - handle.sample_count();
            let pumped = handle.pump(remaining.min(4096));
            if pumped == 0 {
                break; // source exhausted (unexpected for demo; guard for real devices)
            }
        }
        block_on(handle.apply(AcquisitionCommand::Stop))?;
    }

    match opts.format {
        OutputFormat::Csv => write_csv(&handle, writer),
        OutputFormat::Vcd => write_vcd(&handle, writer),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Scans for `address` across all registered drivers and connects to it.
fn find_and_connect(address: &str) -> anyhow::Result<Box<dyn Device>> {
    let registry = DriverRegistry::with_default_factories();
    let results = block_on(registry.scan_all())?;
    let result = results
        .into_iter()
        .find(|r| r.candidate.address == address)
        .with_context(|| format!("no device found at address: {address}"))?;
    let device = block_on(registry.connect(&result.driver, &result.candidate))?;
    Ok(device)
}

/// Writes all acquired samples as CSV.
///
/// Header: `sample_index,time_s,<analog_names...>,<digital_names...>`
/// Each data row: sample index, time in seconds (9 dp), analog physical values,
/// digital channel bit values (0/1).
fn write_csv(handle: &DeviceHandle, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    // Derive the per-sample time step from the first available timebase.
    let sample_period_s = handle
        .analog_traces()
        .first()
        .map(|t| t.timebase().sample_period_s())
        .or_else(|| {
            handle
                .digital_trace()
                .map(|t| t.timebase().sample_period_s())
        })
        .unwrap_or(1.0);

    // Header row.
    write!(writer, "sample_index,time_s")?;
    for trace in handle.analog_traces() {
        write!(writer, ",{}", trace.channel().name)?;
    }
    if let Some(dt) = handle.digital_trace() {
        for ch in dt.channels() {
            write!(writer, ",{}", ch.name)?;
        }
    }
    writeln!(writer)?;

    // Data rows.
    let n = handle.sample_count();
    for i in 0..n {
        let t = i as f64 * sample_period_s;
        write!(writer, "{i},{t:.9}")?;

        for trace in handle.analog_traces() {
            let raw = trace.store().raw()[i];
            let phys = trace.to_physical(raw);
            write!(writer, ",{phys:.9}")?;
        }
        if let Some(dt) = handle.digital_trace() {
            let word = dt.store().words()[i];
            for ch in dt.channels() {
                let bit = (word >> ch.bit) & 1;
                write!(writer, ",{bit}")?;
            }
        }
        writeln!(writer)?;
    }

    Ok(())
}

/// Writes all acquired digital samples as VCD (Value Change Dump, IEEE 1364).
///
/// Only changes are emitted; timestamps are in nanoseconds.  Analog channels
/// are omitted (VCD is a digital format).
fn write_vcd(handle: &DeviceHandle, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let dt = handle
        .digital_trace()
        .ok_or_else(|| anyhow::anyhow!("device has no digital channels"))?;

    let rate = dt.timebase().sample_rate_hz();
    let period_ns = (1e9 / rate).round() as u64;
    let channels = dt.channels();

    // VCD identifiers: printable ASCII starting at '!' (33).
    let ids: Vec<char> = (0..channels.len())
        .map(|i| char::from_u32(33 + i as u32).expect("at most 94 digital channels in VCD"))
        .collect();

    // Header.
    writeln!(writer, "$timescale 1 ns $end")?;
    writeln!(writer, "$version RustyBench-CLI $end")?;
    writeln!(writer, "$scope module top $end")?;
    for (ch, id) in channels.iter().zip(ids.iter()) {
        writeln!(writer, "$var wire 1 {id} {} $end", ch.name)?;
    }
    writeln!(writer, "$upscope $end")?;
    writeln!(writer, "$enddefinitions $end")?;

    // Initial values at t=0.
    let words = dt.store().words();
    writeln!(writer, "#0")?;
    writeln!(writer, "$dumpvars")?;
    if !words.is_empty() {
        for (ch, id) in channels.iter().zip(ids.iter()) {
            let bit = (words[0] >> ch.bit) & 1;
            writeln!(writer, "{bit}{id}")?;
        }
    }
    writeln!(writer, "$end")?;

    // Value changes (only emit when a channel transitions).
    let mut prev = words.first().copied().unwrap_or(0);
    for (i, &word) in words.iter().enumerate().skip(1) {
        if word != prev {
            writeln!(writer, "#{}", i as u64 * period_ns)?;
            for (ch, id) in channels.iter().zip(ids.iter()) {
                let prev_bit = (prev >> ch.bit) & 1;
                let curr_bit = (word >> ch.bit) & 1;
                if prev_bit != curr_bit {
                    writeln!(writer, "{curr_bit}{id}")?;
                }
            }
            prev = word;
        }
    }

    Ok(())
}
