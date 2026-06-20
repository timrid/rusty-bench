//! Core data model for RustyBench: sample types, channels, timebase, the sample
//! stores and their multi-resolution mip-maps.
//!
//! This crate is intentionally free of I/O and async-runtime dependencies so it
//! compiles unchanged to native and `wasm32-unknown-unknown`.
//!
//! # Layers
//! - [`Timebase`] maps base-sample indices to time.
//! - [`AnalogChannel`] / [`DigitalChannel`] carry per-channel metadata.
//! - [`AnalogStore`] / [`DigitalStore`] hold append-only base samples.
//! - [`AnalogMipMap`] (min/max pyramid) and [`DigitalMipMap`] (transition index)
//!   provide constant-cost reads at any zoom.
//! - [`AnalogTrace`] / [`DigitalTrace`] bundle the above into a display-facing
//!   surface: push samples, then request draw [`Bucket`]s or edges over a range.

#![forbid(unsafe_code)]

mod analog;
mod channel;
mod chunk;
mod digital;
mod timebase;

pub use analog::{AnalogMipMap, AnalogStore, AnalogTrace, Bucket, DEFAULT_RADIX, MinMax};
pub use channel::{AnalogChannel, AnalogFormat, ChannelId, DigitalChannel};
pub use chunk::SampleChunk;
pub use digital::{DigitalMipMap, DigitalStore, DigitalTrace, LogicWord, MAX_DIGITAL_CHANNELS};
pub use timebase::Timebase;
