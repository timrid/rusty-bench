//! The bulk-data seam: [`AcquisitionSource`].
//!
//! Control-plane operations (arming, configuring) flow through the `async`
//! capability traits, but **bulk samples never do**. Instead, a device that is
//! capturing exposes a synchronous, pull-based [`AcquisitionSource`]: the session's
//! acquisition loop repeatedly asks it for the next [`SampleChunk`] and appends the
//! result to the per-device stores. Keeping this path synchronous and free of
//! `async-trait` boxing matches the decision that mass data rides the sample-store
//! pipeline rather than per-call futures, and keeps it trivially testable.

use rb_model::SampleChunk;

/// A pull-based source of freshly-acquired samples.
///
/// Implementors return up to `max_samples` new samples per call, advancing their
/// own internal position. A live device produces real samples here; a synthetic
/// device (e.g. the demo driver) generates them. The returned chunk should be
/// [consistent](SampleChunk::is_consistent) and carry at most `max_samples`
/// samples; an exhausted or idle source returns an empty chunk.
pub trait AcquisitionSource {
    /// Produces the next block of up to `max_samples` samples.
    fn next_chunk(&mut self, max_samples: usize) -> SampleChunk;
}
