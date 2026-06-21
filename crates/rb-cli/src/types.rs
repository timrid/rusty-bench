//! Shared type definitions for the RustyBench CLI.
//!
//! These types are used by the clap argument parser (`main.rs`), the command
//! implementations (`scan.rs`, `info.rs`, `record.rs`, etc.), and by integration
//! tests.  They are re-exported from [`crate`] (i.e. `rb_cli::*`).

/// Output format for the `record` subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Comma-separated values — analog and digital, one row per sample.
    Csv,
    /// Value Change Dump (IEEE 1364) — digital channels only.
    Vcd,
    /// Native RustyBench capture (`.rbc`) — versioned ZIP archive.
    Native,
}

/// A channel selection spec parsed from the `--channels` argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelSpec {
    /// A single channel (e.g. `"A0"`).
    Single(String),
    /// An expanded channel range (e.g. `["D0","D1","D2","D3"]`).
    Range(Vec<String>),
    /// A channel with a user-assigned label (e.g. channel `"D7"`, label `"CLK"`).
    Named {
        /// Channel name on the device.
        channel: String,
        /// User-assigned label.
        label: String,
    },
}

/// How long to record.
#[derive(Clone, Debug)]
pub enum RecordBounds {
    /// Stop when a limit is reached (first of samples or time, if set).
    Finite {
        /// Max samples. `None` means unlimited.
        samples: Option<usize>,
        /// Max duration in seconds. `None` means unlimited.
        time: Option<f64>,
    },
    /// Record until interrupted.
    Continuous,
}

/// Options forwarded from the `record` subcommand.
pub struct RecordOpts {
    /// Opaque device address (e.g. `"demo:0"`).
    pub address: String,
    /// Recording bounds.
    pub bounds: RecordBounds,
    /// Override the device's default sample rate, in hertz.
    pub rate: Option<f64>,
    /// Channel selection.
    pub channels: Vec<ChannelSpec>,
    /// Device-specific configuration (`key=value` pairs).
    pub config: Vec<String>,
    /// Output format.
    pub format: OutputFormat,
}

/// Electronic load regulation mode (re-exported from `rb-device`).
pub use rb_device::LoadMode as LoadModeArg;
