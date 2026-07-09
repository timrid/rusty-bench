//! Record subcommand — acquire samples from a device and write a capture.

use std::io;

use futures::StreamExt;
use futures::executor::block_on;
use futures::task::LocalSpawnExt;
use rb_core::DeviceHandle;
use rb_io::{AnalogCapture, DeviceCapture, DigitalCapture};
use rb_model::{AnalogTrace, DigitalTrace, Timebase};

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

    // Resolve sample rate from device capabilities.
    let sample_rate = resolve_sample_rate(&handle);
    let rate = opts.rate.unwrap_or(sample_rate);

    // Apply sample rate.
    if let Some(la) = handle.device_mut().as_logic_analyzer_mut() {
        block_on(la.set_sample_rate_hz(rate))?;
    }
    if let Some(scope) = handle.device_mut().as_oscilloscope_mut() {
        block_on(scope.set_sample_rate_hz(rate))?;
    }

    // Build traces on the CLI side.
    let timebase = Timebase::new(if rate > 0.0 { rate } else { 1.0 }, 0.0);
    let mut analog: Vec<AnalogTrace> = handle
        .device()
        .as_oscilloscope()
        .map(|s| {
            s.channels()
                .iter()
                .map(|ch| AnalogTrace::new(ch.clone(), timebase))
                .collect()
        })
        .unwrap_or_default();
    let mut digital: Option<DigitalTrace> = handle
        .device()
        .as_logic_analyzer()
        .filter(|la| !la.channels().is_empty())
        .map(|la| DigitalTrace::new(la.channels().to_vec(), timebase));

    // Validate format before starting acquisition.
    if opts.format == OutputFormat::Vcd && digital.is_none() {
        anyhow::bail!(
            "VCD format requires a device with digital channels, but {} has none",
            opts.address
        );
    }

    // Compute effective sample target.
    let mut target_samples = match &opts.bounds {
        RecordBounds::Finite { samples, .. } => samples.unwrap_or(usize::MAX),
        RecordBounds::Continuous => {
            anyhow::bail!("--continuous is not yet implemented");
        }
    };
    if let RecordBounds::Finite { time: Some(t), .. } = &opts.bounds {
        let from_time = (rate * t).round() as usize;
        target_samples = target_samples.min(from_time);
    }

    let mut sample_count = 0usize;

    if target_samples > 0 {
        // Arm the device.
        if let Some(la) = handle.device_mut().as_logic_analyzer_mut() {
            block_on(la.arm())?;
        }
        if let Some(scope) = handle.device_mut().as_oscilloscope_mut() {
            block_on(scope.arm())?;
        }

        // Start streaming.
        let (data_tx, data_rx) = futures::channel::mpsc::unbounded();
        let (gui_tx, mut gui_rx) = futures::channel::mpsc::unbounded();
        let read_loop = block_on(
            handle
                .device_mut()
                .as_acquisition_source_mut()
                .ok_or_else(|| anyhow::anyhow!("device has no acquisition source"))?
                .start_streaming(data_tx),
        )?;

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner()
            .spawn_local(rb_core::run_acquisition(read_loop, data_rx, Some(gui_tx)))
            .map_err(|e| anyhow::anyhow!("failed to spawn read loop: {e}"))?;

        // Drain data into local traces.
        while sample_count < target_samples {
            let chunk = pool.run_until(gui_rx.next());
            match chunk {
                Some(chunk) => {
                    for (i, trace) in analog.iter_mut().enumerate() {
                        if let Some(samples) = chunk.analog_channel(i) {
                            trace.push_raw(samples);
                        }
                    }
                    if let Some(ref mut dt) = digital {
                        if !chunk.logic().is_empty() {
                            dt.push_words(chunk.logic());
                        }
                    }
                    sample_count += chunk.sample_count();
                }
                None => break,
            }
        }

        // Stop streaming.
        drop(gui_rx);
        if let Some(src) = handle.device_mut().as_acquisition_source_mut() {
            block_on(src.stop_streaming())?;
        }
        if let Some(la) = handle.device_mut().as_logic_analyzer_mut() {
            block_on(la.stop())?;
        }
        if let Some(scope) = handle.device_mut().as_oscilloscope_mut() {
            block_on(scope.stop())?;
        }
        pool.run_until_stalled();
    }

    // Build capture from local traces.
    let info = handle.device().info().clone();
    let analog_captures: Vec<AnalogCapture> = analog
        .iter()
        .map(|t| AnalogCapture {
            channel: t.channel().clone(),
            timebase: *t.timebase(),
            samples: t.store().raw().to_vec(),
        })
        .collect();
    let digital_capture = digital.map(|dt| DigitalCapture {
        channels: dt.channels().to_vec(),
        timebase: *dt.timebase(),
        words: dt.store().words().to_vec(),
    });

    let capture = DeviceCapture {
        info,
        analog: analog_captures,
        digital: digital_capture,
    };

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

/// Returns the device's current sample rate, reading from whichever acquirable
/// capability is available.
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
