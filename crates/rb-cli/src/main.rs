//! RustyBench command-line interface — a headless, scriptable single-shot tool.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use rb_cli::{ChannelSpec, OutputFormat, RecordBounds, RecordOpts};

/// RustyBench CLI — stateless, single-shot bench-tool driver.
#[derive(Parser, Debug)]
#[command(name = "rusty-bench", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Whether to enable or disable an output.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// Electronic load regulation mode.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum LoadModeArg {
    /// Constant current.
    Cc,
    /// Constant voltage.
    Cv,
    /// Constant resistance.
    Cr,
    /// Constant power.
    Cp,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scan all registered drivers and list reachable devices.
    Scan {
        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Connect to a device and show its full capability details.
    Info {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Record samples from a device.
    Record {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Number of samples to record.  Supports k, m, g suffixes.
        #[arg(short, long, value_parser = parse_sample_count)]
        samples: Option<usize>,

        /// Recording duration (e.g. `100ms`, `2s`, `1min`).
        #[arg(short, long, value_parser = parse_duration)]
        time: Option<f64>,

        /// Record continuously until interrupted (Ctrl+C).
        /// Incompatible with --samples and --time.
        #[arg(short = 'C', long, conflicts_with = "samples", conflicts_with = "time")]
        continuous: bool,

        /// Override the device's default sample rate, in hertz.
        /// Supports k, m, g suffixes.
        #[arg(short, long, value_parser = parse_rate)]
        rate: Option<f64>,

        /// Channel selection (e.g. `A0,D0-D3,D7=CLK`).
        #[arg(short = 'c', long, value_parser = parse_channels)]
        channels: Option<Vec<ChannelSpec>>,

        /// Device-specific configuration (repeatable).
        /// Format: `key=value`.
        #[arg(long = "config", value_name = "KEY=VALUE")]
        config: Vec<String>,

        /// Output format.
        #[arg(long = "output-format", value_enum, default_value = "csv")]
        format: OutputFormat,

        /// Write output to a file instead of stdout.
        #[arg(short = 'o', long = "output-file")]
        output: Option<PathBuf>,
    },

    /// Take a single measurement from a multimeter.
    Multimeter {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Configure a programmable DC power supply.
    /// Without any set-flags, shows the current status.
    PowerSupply {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Set the target output voltage, in volts.
        #[arg(long)]
        voltage: Option<f64>,

        /// Set the current limit, in amperes.
        #[arg(long = "current-limit")]
        current_limit: Option<f64>,

        /// Enable or disable the output.
        #[arg(long)]
        output: Option<OnOff>,

        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Configure a waveform / function generator.
    /// Without any set-flags, shows the current status.
    WaveformGen {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Set the output frequency, in hertz.
        #[arg(long, value_parser = parse_rate)]
        frequency: Option<f64>,

        /// Set the output amplitude, in volts.
        #[arg(long)]
        amplitude: Option<f64>,

        /// Enable or disable the output.
        #[arg(long)]
        output: Option<OnOff>,

        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Configure a programmable electronic load.
    /// Without any set-flags, shows the current status.
    ElectronicLoad {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Set the regulation mode.
        #[arg(long)]
        mode: Option<LoadModeArg>,

        /// Set the regulation setpoint (A, V, Ω, or W depending on mode).
        #[arg(long)]
        setpoint: Option<f64>,

        /// Enable or disable the load input.
        #[arg(long)]
        input: Option<OnOff>,

        /// Output machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Decode a previously recorded capture file.
    Decode {
        /// Input capture file.
        #[arg(short = 'i', long = "input-file")]
        input_file: PathBuf,

        /// Input format. If omitted, auto-detected from file extension.
        #[arg(short = 'I', long = "input-format")]
        input_format: Option<String>,

        /// Protocol decoders to apply (comma-separated).
        #[arg(short = 'P', long = "decoders")]
        decoders: Option<String>,

        /// Output format for decoder annotations.
        #[arg(long = "output-format", default_value = "text")]
        output_format: String,

        /// Write output to a file instead of stdout.
        #[arg(short = 'o', long = "output-file")]
        output_file: Option<PathBuf>,
    },
}

// ── Custom parsers ──────────────────────────────────────────────────────────

/// Parse `--samples` with optional k, m, g suffix.
fn parse_sample_count(raw: &str) -> Result<usize, String> {
    parse_usize_with_si(raw, &['k', 'm', 'g'], &[1_000, 1_000_000, 1_000_000_000])
}

/// Parse `--rate` (Hz) with optional k, m, g suffix.
fn parse_rate(raw: &str) -> Result<f64, String> {
    parse_f64_with_si(raw, &['k', 'm', 'g'], &[1e3, 1e6, 1e9]).map(|(v, _)| v)
}

/// Parse `--time` with ms, s, min suffix.
fn parse_duration(raw: &str) -> Result<f64, String> {
    if let Some(rest) = raw.strip_suffix("min") {
        return rest
            .trim()
            .parse::<f64>()
            .map(|v| v * 60.0)
            .map_err(|_| format!("invalid duration: '{raw}'"));
    }
    if let Some(rest) = raw.strip_suffix("ms") {
        return rest
            .trim()
            .parse::<f64>()
            .map(|v| v / 1000.0)
            .map_err(|_| format!("invalid duration: '{raw}'"));
    }
    parse_f64_with_si(raw, &['s'], &[1.0]).map(|(v, _)| v)
}

/// Parse `--channels` list like `A0,D0-D3,D7=CLK`.
fn parse_channels(raw: &str) -> Result<Vec<ChannelSpec>, String> {
    let mut specs = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        specs.push(parse_one_channel(part)?);
    }
    Ok(specs)
}

fn parse_one_channel(raw: &str) -> Result<ChannelSpec, String> {
    // "D7=CLK" → name mapping
    if let Some((channel, label)) = raw.split_once('=') {
        return Ok(ChannelSpec::Named {
            channel: channel.trim().to_string(),
            label: label.trim().to_string(),
        });
    }
    // "D0-D3" → range
    if raw.contains('-') {
        return parse_channel_range(raw);
    }
    // "A0" → single channel
    Ok(ChannelSpec::Single(raw.to_string()))
}

fn parse_channel_range(raw: &str) -> Result<ChannelSpec, String> {
    let (prefix_start, end_str) = raw
        .split_once('-')
        .ok_or_else(|| format!("invalid channel range: '{raw}'"))?;
    let end_num: usize = end_str
        .parse()
        .map_err(|_| format!("invalid channel range end: '{end_str}'"))?;

    let digit_pos = prefix_start
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|p| p + 1)
        .unwrap_or(0);

    if digit_pos >= prefix_start.len() {
        return Err(format!("invalid channel range: '{raw}'"));
    }

    let prefix = &prefix_start[..digit_pos];
    let start_num: usize = prefix_start[digit_pos..]
        .parse()
        .map_err(|_| format!("invalid channel range start: '{prefix_start}'"))?;

    let channels: Vec<String> = (start_num..=end_num).map(|n| format!("{prefix}{n}")).collect();
    Ok(ChannelSpec::Range(channels))
}

/// Parse an integer with SI suffix.
fn parse_usize_with_si(
    raw: &str,
    suffixes: &[char],
    multipliers: &[usize],
) -> Result<usize, String> {
    let raw_lower = raw.trim().to_lowercase();
    for (&suffix, &multiplier) in suffixes.iter().zip(multipliers) {
        if let Some(num_str) = raw_lower.strip_suffix(suffix) {
            let base: usize = num_str
                .trim()
                .parse()
                .map_err(|_| format!("invalid number: '{raw}'"))?;
            return Ok(base * multiplier);
        }
    }
    raw_lower
        .parse()
        .map_err(|_| format!("invalid number: '{raw}'"))
}

/// Parse a float with optional SI suffix.
fn parse_f64_with_si(
    raw: &str,
    suffixes: &[char],
    multipliers: &[f64],
) -> Result<(f64, Option<char>), String> {
    let raw_lower = raw.trim().to_lowercase();
    for (&suffix, &multiplier) in suffixes.iter().zip(multipliers) {
        if let Some(num_str) = raw_lower.strip_suffix(suffix) {
            let base: f64 = num_str
                .trim()
                .parse()
                .map_err(|_| format!("invalid number: '{raw}'"))?;
            return Ok((base * multiplier, Some(suffix)));
        }
    }
    raw_lower
        .parse()
        .map(|v| (v, None))
        .map_err(|_| format!("invalid number: '{raw}'"))
}

// ── main ────────────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let stdout = std::io::stdout();

    match cli.command {
        Command::Scan { json } => rb_cli::run_scan(&mut stdout.lock(), json),

        Command::Info { address, json } => rb_cli::run_info(&address, &mut stdout.lock(), json),

        Command::Record {
            address,
            samples,
            time,
            continuous,
            rate,
            channels,
            config,
            format,
            output,
        } => {
            let bounds = if continuous {
                RecordBounds::Continuous
            } else {
                RecordBounds::Finite { samples, time }
            };

            let opts = RecordOpts {
                address,
                bounds,
                rate,
                channels: channels.unwrap_or_default(),
                config,
                format,
            };
            match output {
                Some(ref path) => {
                    let mut file = std::fs::File::create(path)?;
                    rb_cli::run_record(opts, &mut file)
                }
                None => rb_cli::run_record(opts, &mut stdout.lock()),
            }
        }

        Command::Multimeter { address, json } => {
            rb_cli::run_multimeter(&address, &mut stdout.lock(), json)
        }

        Command::PowerSupply {
            address,
            voltage,
            current_limit,
            output,
            json,
        } => {
            let output_enabled = output.map(|o| matches!(o, OnOff::On));
            rb_cli::run_power_supply(
                &address,
                voltage,
                current_limit,
                output_enabled,
                &mut stdout.lock(),
                json,
            )
        }

        Command::WaveformGen {
            address,
            frequency,
            amplitude,
            output,
            json,
        } => {
            let output_enabled = output.map(|o| matches!(o, OnOff::On));
            rb_cli::run_waveform_gen(
                &address,
                frequency,
                amplitude,
                output_enabled,
                &mut stdout.lock(),
                json,
            )
        }

        Command::ElectronicLoad {
            address,
            mode,
            setpoint,
            input,
            json,
        } => {
            let input_enabled = input.map(|o| matches!(o, OnOff::On));
            let load_mode = mode.map(|m| match m {
                LoadModeArg::Cc => rb_cli::LoadModeArg::ConstantCurrent,
                LoadModeArg::Cv => rb_cli::LoadModeArg::ConstantVoltage,
                LoadModeArg::Cr => rb_cli::LoadModeArg::ConstantResistance,
                LoadModeArg::Cp => rb_cli::LoadModeArg::ConstantPower,
            });
            rb_cli::run_electronic_load(
                &address,
                load_mode,
                setpoint,
                input_enabled,
                &mut stdout.lock(),
                json,
            )
        }

        Command::Decode {
            input_file,
            input_format,
            decoders,
            output_format,
            output_file,
        } => match output_file {
            Some(ref path) => {
                let mut file = std::fs::File::create(path)?;
                rb_cli::run_decode(
                    &input_file,
                    input_format.as_deref(),
                    decoders.as_deref(),
                    &output_format,
                    &mut file,
                )
            }
            None => rb_cli::run_decode(
                &input_file,
                input_format.as_deref(),
                decoders.as_deref(),
                &output_format,
                &mut stdout.lock(),
            ),
        },
    }
}
