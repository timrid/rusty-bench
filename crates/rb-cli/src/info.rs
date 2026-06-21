//! Info subcommand — show full device capability details.

use std::io;

use rb_device::Device;
use serde::Serialize;

use crate::util::{find_and_connect, write_json};

/// Connects to `address` and prints full capability details.
///
/// # Errors
/// Returns an error if the device cannot be found or connected.
pub fn run_info(address: &str, writer: &mut dyn io::Write, json: bool) -> anyhow::Result<()> {
    let device = find_and_connect(address)?;
    let info = device.info();

    if json {
        let info_json = DeviceInfoJson {
            vendor: info.vendor.clone(),
            model: info.model.clone(),
            serial: info.serial.clone(),
            address: address.to_string(),
            classes: device.classes().iter().map(|c| format!("{c:?}")).collect(),
            capabilities: build_capabilities_json(device.as_ref()),
        };
        return write_json(&info_json, writer);
    }

    writeln!(writer, "Vendor:  {}", info.vendor)?;
    writeln!(writer, "Model:   {}", info.model)?;
    if let Some(serial) = &info.serial {
        writeln!(writer, "Serial:  {serial}")?;
    }
    writeln!(writer, "Address: {address}")?;
    let class_names: Vec<String> = device.classes().iter().map(|c| format!("{c:?}")).collect();
    writeln!(writer, "Classes: {}", class_names.join(", "))?;

    write_oscilloscope_info(device.as_ref(), writer)?;
    write_logic_analyzer_info(device.as_ref(), writer)?;
    write_multimeter_info(device.as_ref(), writer)?;
    write_power_supply_info(device.as_ref(), writer)?;
    write_waveform_gen_info(device.as_ref(), writer)?;
    write_sdr_receiver_info(device.as_ref(), writer)?;
    write_spectrum_analyzer_info(device.as_ref(), writer)?;
    write_electronic_load_info(device.as_ref(), writer)?;

    Ok(())
}

// ── JSON types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct DeviceInfoJson {
    vendor: String,
    model: String,
    serial: Option<String>,
    address: String,
    classes: Vec<String>,
    capabilities: CapabilitiesJson,
}

#[derive(Serialize)]
struct CapabilitiesJson {
    oscilloscope: Option<OscilloscopeJson>,
    logic_analyzer: Option<LogicAnalyzerJson>,
    multimeter: Option<MultimeterJson>,
    power_supply: Option<PowerSupplyJson>,
    waveform_generator: Option<WaveformGenJson>,
    sdr_receiver: Option<SdrReceiverJson>,
    spectrum_analyzer: Option<SpectrumAnalyzerJson>,
    electronic_load: Option<ElectronicLoadJson>,
}

#[derive(Serialize)]
struct OscilloscopeJson {
    sample_rate_hz: f64,
    channels: Vec<AnalogChannelJson>,
}

#[derive(Serialize)]
struct AnalogChannelJson {
    id: String,
    name: String,
    scale: f64,
    offset: f64,
    unit: String,
}

#[derive(Serialize)]
struct LogicAnalyzerJson {
    sample_rate_hz: f64,
    channels: Vec<DigitalChannelJson>,
}

#[derive(Serialize)]
struct DigitalChannelJson {
    id: String,
    name: String,
    bit: u8,
}

#[derive(Serialize)]
struct MultimeterJson {
    unit: String,
}

#[derive(Serialize)]
struct PowerSupplyJson {
    output_enabled: bool,
}

#[derive(Serialize)]
struct WaveformGenJson {
    output_enabled: bool,
}

#[derive(Serialize)]
struct SdrReceiverJson {
    sample_rate_hz: f64,
}

#[derive(Serialize)]
struct SpectrumAnalyzerJson {}

#[derive(Serialize)]
struct ElectronicLoadJson {
    mode: String,
}

// ── JSON builder ──────────────────────────────────────────────────────────────

fn build_capabilities_json(device: &dyn Device) -> CapabilitiesJson {
    CapabilitiesJson {
        oscilloscope: device.as_oscilloscope().map(|s| OscilloscopeJson {
            sample_rate_hz: s.sample_rate_hz(),
            channels: s
                .channels()
                .iter()
                .map(|ch| AnalogChannelJson {
                    id: ch.id.0.to_string(),
                    name: ch.name.clone(),
                    scale: ch.format.scale,
                    offset: ch.format.offset,
                    unit: ch.unit.clone().unwrap_or_else(|| "?".into()),
                })
                .collect(),
        }),
        logic_analyzer: device.as_logic_analyzer().map(|la| LogicAnalyzerJson {
            sample_rate_hz: la.sample_rate_hz(),
            channels: la
                .channels()
                .iter()
                .map(|ch| DigitalChannelJson {
                    id: ch.id.0.to_string(),
                    name: ch.name.clone(),
                    bit: ch.bit,
                })
                .collect(),
        }),
        multimeter: device.as_multimeter().map(|m| MultimeterJson {
            unit: m.unit().unwrap_or("?").to_string(),
        }),
        power_supply: device.as_power_supply().map(|ps| PowerSupplyJson {
            output_enabled: ps.is_output_enabled(),
        }),
        waveform_generator: device.as_waveform_generator().map(|wg| WaveformGenJson {
            output_enabled: wg.is_output_enabled(),
        }),
        sdr_receiver: device.as_sdr_receiver().map(|sdr| SdrReceiverJson {
            sample_rate_hz: sdr.sample_rate_hz(),
        }),
        spectrum_analyzer: device.as_spectrum_analyzer().map(|_| SpectrumAnalyzerJson {}),
        electronic_load: device.as_electronic_load().map(|el| ElectronicLoadJson {
            mode: format!("{:?}", el.mode()),
        }),
    }
}

// ── Human-readable info helpers ──────────────────────────────────────────────

fn write_oscilloscope_info(device: &dyn Device, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let Some(scope) = device.as_oscilloscope() else {
        return Ok(());
    };
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
    Ok(())
}

fn write_logic_analyzer_info(
    device: &dyn Device,
    writer: &mut dyn io::Write,
) -> anyhow::Result<()> {
    let Some(la) = device.as_logic_analyzer() else {
        return Ok(());
    };
    writeln!(writer, "\nLogic Analyzer:")?;
    writeln!(writer, "  Sample rate: {} Hz", la.sample_rate_hz())?;
    for ch in la.channels() {
        writeln!(
            writer,
            "  Channel {}: {} (bit {})",
            ch.id.0, ch.name, ch.bit
        )?;
    }
    Ok(())
}

fn write_multimeter_info(device: &dyn Device, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let Some(dmm) = device.as_multimeter() else {
        return Ok(());
    };
    let unit = dmm.unit().unwrap_or("?");
    writeln!(writer, "\nMultimeter:")?;
    writeln!(writer, "  Unit: {unit}")?;
    Ok(())
}

fn write_power_supply_info(device: &dyn Device, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let Some(ps) = device.as_power_supply() else {
        return Ok(());
    };
    let state = if ps.is_output_enabled() { "on" } else { "off" };
    writeln!(writer, "\nPower Supply:")?;
    writeln!(writer, "  Output: {state}")?;
    Ok(())
}

fn write_waveform_gen_info(device: &dyn Device, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let Some(wg) = device.as_waveform_generator() else {
        return Ok(());
    };
    let state = if wg.is_output_enabled() { "on" } else { "off" };
    writeln!(writer, "\nWaveform Generator:")?;
    writeln!(writer, "  Output: {state}")?;
    Ok(())
}

fn write_sdr_receiver_info(device: &dyn Device, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let Some(sdr) = device.as_sdr_receiver() else {
        return Ok(());
    };
    writeln!(writer, "\nSDR Receiver:")?;
    writeln!(writer, "  Sample rate: {} Hz", sdr.sample_rate_hz())?;
    Ok(())
}

fn write_spectrum_analyzer_info(
    device: &dyn Device,
    writer: &mut dyn io::Write,
) -> anyhow::Result<()> {
    if device.as_spectrum_analyzer().is_some() {
        writeln!(writer, "\nSpectrum Analyzer: supported")?;
    }
    Ok(())
}

fn write_electronic_load_info(
    device: &dyn Device,
    writer: &mut dyn io::Write,
) -> anyhow::Result<()> {
    let Some(el) = device.as_electronic_load() else {
        return Ok(());
    };
    writeln!(writer, "\nElectronic Load:")?;
    writeln!(writer, "  Mode: {:?}", el.mode())?;
    Ok(())
}
