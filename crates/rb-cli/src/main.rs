//! RustyBench command-line interface — a headless, scriptable single-shot tool.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rb_cli::{AcquireOpts, OutputFormat};

/// RustyBench CLI.
#[derive(Parser, Debug)]
#[command(name = "rusty-bench", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scan all registered drivers and list reachable devices.
    Scan,

    /// Connect to a device and confirm the connection.
    Connect {
        /// Opaque device address (e.g. `demo:0`).
        address: String,
    },

    /// Connect to a device and show its full capability details.
    Info {
        /// Opaque device address (e.g. `demo:0`).
        address: String,
    },

    /// Connect to a device, acquire samples, and write the capture.
    Acquire {
        /// Opaque device address (e.g. `demo:0`).
        address: String,

        /// Number of samples to acquire.
        #[arg(short, long, default_value = "1000")]
        samples: usize,

        /// Override the device's default sample rate, in hertz.
        #[arg(short, long)]
        rate: Option<f64>,

        /// Output format.
        #[arg(long, value_enum, default_value = "csv")]
        format: OutputFormat,

        /// Write output to a file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let stdout = std::io::stdout();

    match cli.command {
        Command::Scan => rb_cli::run_scan(&mut stdout.lock()),

        Command::Connect { address } => rb_cli::run_connect(&address, &mut stdout.lock()),

        Command::Info { address } => rb_cli::run_info(&address, &mut stdout.lock()),

        Command::Acquire {
            address,
            samples,
            rate,
            format,
            output,
        } => {
            let opts = AcquireOpts {
                address,
                samples,
                rate,
                format,
            };
            match output {
                Some(ref path) => {
                    let mut file = std::fs::File::create(path)?;
                    rb_cli::run_acquire(opts, &mut file)
                }
                None => rb_cli::run_acquire(opts, &mut stdout.lock()),
            }
        }
    }
}
