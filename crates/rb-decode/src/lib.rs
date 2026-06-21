//! Protocol decoders for RustyBench logic-analyzer captures.
//!
//! Decoders are streaming, stacking state machines written clean-room from open
//! protocol specifications (no GPLv3 sigrok source). They compile unchanged to
//! `wasm32-unknown-unknown`.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use rb_decode::{Decoder, UartDecoder, UartConfig};
//!
//! let mut dec = UartDecoder::new(UartConfig { baud_rate: 115_200, ..Default::default() });
//! let words: Vec<u64> = Vec::new(); // packed logic words from a capture
//! let anns = dec.feed(&words, 0, 1_000_000.0);
//! for ann in &anns {
//!     println!("{:?}: {}", ann.range, ann.label);
//! }
//! ```

#![forbid(unsafe_code)]

pub mod annotation;
pub mod decoder;
pub mod i2c;
pub mod spi;
pub mod stack;
pub mod uart;

pub use annotation::{Annotation, AnnotationKind};
pub use decoder::{ByteDecoder, DecodedByte, Decoder};
pub use i2c::{I2cConfig, I2cDecoder};
pub use spi::{SpiConfig, SpiDecoder};
pub use stack::StackedDecoder;
pub use uart::{Parity, StopBits, UartConfig, UartDecoder};
