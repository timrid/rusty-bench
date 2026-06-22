//! Record subcommand — acquire samples from a device and write a capture.

use std::io;

use futures::StreamExt;
use futures::executor::block_on;
use futures::task::LocalSpawnExt;
use rb_core::{AcquisitionCommand, DeviceHandle};
use rb_io::{AnalogCapture, DeviceCapture, DigitalCapture};

use crate::types::{OutputFormat, RecordBounds, RecordOpts};
use crate::util::find_and_connect;

/// Records samples from the device at `opts.address` according to `opts.bounds`,
/// and writes the capture to `writer` in the requested format.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or if the
/// requested format is incompatible with the device's capabilities.
pub fn run_record(opts: RecordOpts, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let device = find_and_connect(&opts.address)?;
    let mut handle = DeviceHandle::new(device);

    if let Some(rate) = opts.rate {
        block_on(handle.apply(AcquisitionCommand::SetSampleRate(rate)))?;
    }

    // Apply device-specific config options.
    // For now, config is a forward-looking API; the demo driver ignores them.
    // Real drivers will parse their own config keys.
    for _cfg in &opts.config {
        // TODO: apply config to device capabilities
    }

    // Validate format before starting acquisition.
    if opts.format == OutputFormat::Vcd && handle.digital_trace().is_none() {
        anyhow::bail!(
            "VCD format requires a device with digital channels, but {} has none",
            opts.address
        );
    }

    // Compute effective sample target.
    // When `--time` is given, convert it to a sample count using the device's
    // sample rate so that the bound works even for instant (demo) sources.
    let sample_rate = resolve_sample_rate(&handle);
    let mut target_samples = match &opts.bounds {
        RecordBounds::Finite { samples, .. } => samples.unwrap_or(usize::MAX),
        RecordBounds::Continuous => {
            anyhow::bail!("--continuous is not yet implemented");
        }
    };
    if let RecordBounds::Finite { time: Some(t), .. } = &opts.bounds {
        let from_time = (sample_rate * t).round() as usize;
        target_samples = target_samples.min(from_time);
    }

    if target_samples > 0 {
        let (read_loop, mut data_rx) = block_on(handle.start_streaming())?;

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner()
            .spawn_local(read_loop)
            .map_err(|e| anyhow::anyhow!("failed to spawn read loop: {e}"))?;

        while handle.sample_count() < target_samples {
            let chunk = pool.run_until(data_rx.next());
            match chunk {
                Some(chunk) => {
                    handle.ingest_chunk(&chunk);
                }
                None => break, // streaming stopped
            }
        }
        block_on(handle.apply(AcquisitionCommand::Stop))?;
        drop(data_rx);
        pool.run_until_stalled();
    }

    let capture = capture_from_handle(&handle);

    match opts.format {
        OutputFormat::Csv => rb_io::write_csv(&capture, writer)?,
        OutputFormat::Vcd => rb_io::write_vcd(&capture, writer)?,
        OutputFormat::Native => {
            let mut buf = std::io::Cursor::new(Vec::new());
            capture.write_to(&mut buf)?;
            writer.write_all(&buf.into_inner())?;
        }
    }
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

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

/// Returns the device's current sample rate, reading from whichever acquirable
/// capability (oscilloscope, logic analyzer, SDR) is available.
fn resolve_sample_rate(handle: &DeviceHandle) -> f64 {
    if let Some(scope) = handle.device().as_oscilloscope() {
        return scope.sample_rate_hz();
    }
    if let Some(la) = handle.device().as_logic_analyzer() {
        return la.sample_rate_hz();
    }
    if let Some(sdr) = handle.device().as_sdr_receiver() {
        return sdr.sample_rate_hz();
    }
    1.0
}
