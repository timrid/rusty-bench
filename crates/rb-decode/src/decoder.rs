//! The [`Decoder`] and [`ByteDecoder`] traits plus [`DecodedByte`].

use core::ops::Range;

use crate::annotation::Annotation;

/// A streaming, resettable protocol decoder over packed logic samples.
///
/// Implementations are pure state machines: no I/O, no allocation beyond
/// the returned `Vec<Annotation>`. They compile unchanged to
/// `wasm32-unknown-unknown`.
///
/// # Contract
/// Callers must supply **consecutive, non-overlapping** chunks: if call N
/// covers `from_sample..from_sample + words.len()`, call N+1 must start at
/// `from_sample + words.len()`.
///
/// The decoder is not required to emit annotations immediately; a partial
/// frame may be buffered until its trailing bytes arrive in a later call.
pub trait Decoder: Send + 'static {
    /// Human-readable decoder name, e.g. `"UART"`, `"I²C"`.
    fn name(&self) -> &str;

    /// Push a slice of packed logic words into the decoder.
    ///
    /// - `words[0]` corresponds to sample index `from_sample`.
    /// - `rate_hz` is the capture's sample rate in hertz (used by time-based
    ///   decoders such as UART; ignored by edge-triggered decoders).
    ///
    /// Returns [`Annotation`]s produced by this chunk.
    fn feed(&mut self, words: &[u64], from_sample: usize, rate_hz: f64) -> Vec<Annotation>;

    /// Reset the decoder to its initial state, discarding any in-progress frame.
    fn reset(&mut self);
}

/// A decoded byte with its sample range, used as currency between stacked decoders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedByte {
    /// The decoded byte value.
    pub byte: u8,
    /// Half-open sample-index range `[start, end)` of the byte in the capture.
    pub range: Range<usize>,
}

/// A higher-level decoder that consumes decoded bytes from a lower [`Decoder`].
///
/// Used by [`crate::StackedDecoder`] to implement protocol layering (e.g.
/// UART decoded bytes fed into an ASCII or Modbus decoder).
pub trait ByteDecoder: Send + 'static {
    /// Human-readable decoder name.
    fn name(&self) -> &str;

    /// Process a batch of decoded bytes, returning higher-level annotations.
    fn feed_bytes(&mut self, bytes: &[DecodedByte]) -> Vec<Annotation>;

    /// Reset to initial state.
    fn reset(&mut self);
}
