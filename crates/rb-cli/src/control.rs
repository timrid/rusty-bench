//! Control subcommands — multimeter, power-supply, waveform-gen, electronic-load.

use std::io;

use anyhow::Context as _;
use futures::executor::block_on;
use serde::Serialize;

use crate::types::LoadModeArg;
use crate::util::{find_and_connect, write_json};

// ── Multimeter ────────────────────────────────────────────────────────────────

/// Takes a single measurement from a multimeter and prints the value.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or if it
/// does not support the multimeter capability.
pub fn run_multimeter(
    address: &str,
    writer: &mut dyn io::Write,
    json: bool,
) -> anyhow::Result<()> {
    let mut device = find_and_connect(address)?;
    let dmm = device
        .as_multimeter_mut()
        .context("device does not support multimeter measurements")?;

    let value = block_on(dmm.measure())?;
    let unit = dmm.unit().unwrap_or("?");

    if json {
        #[derive(Serialize)]
        struct Measurement {
            value: f64,
            unit: String,
        }
        return write_json(
            &Measurement {
                value,
                unit: unit.to_string(),
            },
            writer,
        );
    }

    writeln!(writer, "{value} {unit}")?;
    Ok(())
}

// ── Power Supply ──────────────────────────────────────────────────────────────

/// Configures or queries a power supply.
///
/// Without any set-flags, shows the current status.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or does not
/// support the power-supply capability.
pub fn run_power_supply(
    address: &str,
    voltage: Option<f64>,
    current_limit: Option<f64>,
    output_enabled: Option<bool>,
    writer: &mut dyn io::Write,
    json: bool,
) -> anyhow::Result<()> {
    let mut device = find_and_connect(address)?;
    let ps = device
        .as_power_supply_mut()
        .context("device does not support power-supply capability")?;

    // Determine if we're setting or just getting.
    let is_set = voltage.is_some() || current_limit.is_some() || output_enabled.is_some();

    if let Some(v) = voltage {
        block_on(ps.set_voltage(v))?;
    }
    if let Some(cl) = current_limit {
        block_on(ps.set_current_limit(cl))?;
    }
    if let Some(on) = output_enabled {
        block_on(ps.set_output_enabled(on))?;
    }

    if is_set {
        // After setting, show new status.
        let state = if ps.is_output_enabled() { "on" } else { "off" };
        if json {
            #[derive(Serialize)]
            struct PsStatus {
                output: String,
            }
            return write_json(&PsStatus { output: state.into() }, writer);
        }
        writeln!(writer, "Power supply output: {state}")?;
    } else {
        // Just query.
        let state = if ps.is_output_enabled() { "on" } else { "off" };
        if json {
            #[derive(Serialize)]
            struct PsStatus {
                output: String,
            }
            return write_json(&PsStatus { output: state.into() }, writer);
        }
        writeln!(writer, "Power supply output: {state}")?;
    }
    Ok(())
}

// ── Waveform Generator ────────────────────────────────────────────────────────

/// Configures or queries a waveform generator.
///
/// Without any set-flags, shows the current status.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or does not
/// support the waveform-generator capability.
pub fn run_waveform_gen(
    address: &str,
    frequency: Option<f64>,
    amplitude: Option<f64>,
    output_enabled: Option<bool>,
    writer: &mut dyn io::Write,
    json: bool,
) -> anyhow::Result<()> {
    let mut device = find_and_connect(address)?;
    let wg = device
        .as_waveform_generator_mut()
        .context("device does not support waveform-generator capability")?;

    let _is_set = frequency.is_some() || amplitude.is_some() || output_enabled.is_some();

    if let Some(f) = frequency {
        block_on(wg.set_frequency_hz(f))?;
    }
    if let Some(a) = amplitude {
        block_on(wg.set_amplitude_v(a))?;
    }
    if let Some(on) = output_enabled {
        block_on(wg.set_output_enabled(on))?;
    }

    let state = if wg.is_output_enabled() { "on" } else { "off" };
    if json {
        #[derive(Serialize)]
        struct WgStatus {
            output: String,
        }
        return write_json(&WgStatus { output: state.into() }, writer);
    }
    writeln!(writer, "Waveform generator output: {state}")?;
    Ok(())
}

// ── Electronic Load ───────────────────────────────────────────────────────────

/// Configures or queries an electronic load.
///
/// Without any set-flags, shows the current status.
///
/// # Errors
/// Returns an error if the device cannot be found, connected, or does not
/// support the electronic-load capability.
pub fn run_electronic_load(
    address: &str,
    mode: Option<LoadModeArg>,
    setpoint: Option<f64>,
    input_enabled: Option<bool>,
    writer: &mut dyn io::Write,
    json: bool,
) -> anyhow::Result<()> {
    let mut device = find_and_connect(address)?;
    let el = device
        .as_electronic_load_mut()
        .context("device does not support electronic-load capability")?;

    if let Some(m) = mode {
        block_on(el.set_mode(m))?;
    }
    if let Some(sp) = setpoint {
        block_on(el.set_setpoint(sp))?;
    }
    if let Some(on) = input_enabled {
        block_on(el.set_input_enabled(on))?;
    }

    if json {
        #[derive(Serialize)]
        struct ElStatus {
            mode: String,
        }
        return write_json(
            &ElStatus {
                mode: format!("{:?}", el.mode()),
            },
            writer,
        );
    }

    writeln!(writer, "Electronic load mode: {:?}", el.mode())?;
    Ok(())
}
