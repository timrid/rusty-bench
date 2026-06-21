//! RustyBench CLI core logic — scan, info, connect, acquire and output formatters.
//!
//! All public functions write their output to a `&mut dyn io::Write`, so tests
//! can capture the output in a `Vec<u8>` without spawning a subprocess.
//!
//! Output is delegated to [`rb_io`] for all capture formats; this module only
//! handles the CLI plumbing (scanning, connecting, acquisition loop).

#![forbid(unsafe_code)]

use std::io;

use anyhow::Context as _;
use futures::executor::block_on;
use rb_core::{AcquisitionCommand, DeviceHandle, DriverRegistry};
use rb_device::Device;
use rb_io::{AnalogCapture, DeviceCapture, DigitalCapture};

// ── Public types ──────────────────────────────────────────────────────────────

/// Output format for the `acquire` subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Comma-separated values — analog and digital, one row per sample.
    Csv,
    /// Value Change Dump (IEEE 1364) — digital channels only.
    Vcd,
    /// Native RustyBench capture (`.rbc`) — versioned ZIP archive.
    Native,
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
            let pumped = block_on(handle.pump(remaining.min(4096)));
            if pumped == 0 {
                break; // source exhausted (guard for real devices)
            }
        }
        block_on(handle.apply(AcquisitionCommand::Stop))?;
    }

    let capture = capture_from_handle(&handle);

    match opts.format {
        OutputFormat::Csv => rb_io::write_csv(&capture, writer)?,
        OutputFormat::Vcd => rb_io::write_vcd(&capture, writer)?,
        OutputFormat::Native => {
            // Native format (ZIP) requires Write + Seek; buffer to memory first
            // so we can stream to any writer including stdout.
            let mut buf = std::io::Cursor::new(Vec::new());
            capture.write_to(&mut buf)?;
            writer.write_all(&buf.into_inner())?;
        }
    }
    Ok(())
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

/// Snapshots a [`DeviceHandle`]'s stores into a [`DeviceCapture`] for export.
fn capture_from_handle(handle: &DeviceHandle) -> DeviceCapture {
    let info = handle.device().info().clone();

    let analog = handle
        .analog_traces()
        .iter()
        .map(|t| AnalogCapture {
            channel: t.channel().clone(),
            timebase: *t.timebase(),
            samples: t.store().raw().to_vec(),
        })
        .collect();

    let digital = handle.digital_trace().map(|dt| DigitalCapture {
        channels: dt.channels().to_vec(),
        timebase: *dt.timebase(),
        words: dt.store().words().to_vec(),
    });

    DeviceCapture {
        info,
        analog,
        digital,
    }
}
