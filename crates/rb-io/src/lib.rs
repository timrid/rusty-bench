//! RustyBench I/O: native capture format, sigrok `.sr` import, CSV and VCD
//! export.
//!
//! # Formats
//! - **Native** (`.rbc`): versioned ZIP archive — JSON manifest + raw binary
//!   blobs.  Written and read via [`DeviceCapture::write_to`] /
//!   [`DeviceCapture::read_from`].
//! - **CSV**: one row per sample, analog converted to physical units, digital as
//!   `0`/`1` bits.  Written with [`write_csv`].
//! - **VCD** (IEEE 1364): digital-only value-change dump with nanosecond
//!   timestamps.  Written with [`write_vcd`].
//! - **Sigrok `.sr`**: logic-only import via [`read_sr`].
//!
//! The crate is runtime-free and I/O-synchronous.  It depends on `rb-model`
//! and `rb-device` but not on `rb-core`, so it stays independent of the
//! acquisition runtime.

#![forbid(unsafe_code)]

mod capture;
mod csv;
mod sr;
mod vcd;

pub use capture::{AnalogCapture, DeviceCapture, DigitalCapture};
pub use csv::write_csv;
pub use sr::read_sr;
pub use vcd::write_vcd;

/// Error type for all rb-io operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// An I/O error during reading or writing.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// A ZIP archive error (bad signature, missing entry, …).
    #[error("ZIP error: {0}")]
    Zip(String),

    /// A JSON serialisation / deserialisation error in the manifest.
    #[error("JSON error")]
    Json(#[from] serde_json::Error),

    /// The capture data or file structure is malformed.
    #[error("malformed capture: {0}")]
    Format(String),
}
