//! RustyBench CLI — command implementations and public API.
//!
//! This is a facade module that re-exports the public API from focused
//! sub-modules. All command logic lives in the corresponding module:
//!
//! | Module       | Commands                          |
//! |--------------|-----------------------------------|
//! | [`scan`]     | `run_scan`                        |
//! | [`info`]     | `run_info`                        |
//! | [`record`]   | `run_record`                      |
//! | [`control`]  | `run_multimeter`, `run_power_supply`, `run_waveform_gen`, `run_electronic_load` |
//! | [`decode`]   | `run_decode`                      |
//!
//! Shared types are defined in [`types`]; shared internal helpers in [`util`].

#![forbid(unsafe_code)]

mod types;
pub use types::*;

mod util;
mod scan;
mod info;
mod record;
mod control;
mod decode;

pub use scan::run_scan;
pub use info::run_info;
pub use record::run_record;
pub use control::{run_electronic_load, run_multimeter, run_power_supply, run_waveform_gen};
pub use decode::run_decode;

