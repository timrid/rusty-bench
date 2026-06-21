//! The bulk-data seam: [`AcquisitionSource`].
//!
//! Control-plane operations (arming, configuring) flow through the `async`
//! capability traits.  The bulk path ([`next_chunk`](AcquisitionSource::next_chunk))
//! is also `async` so that drivers can await USB / network I/O directly without
//! `block_on` or manual polling — the acquisition loop is already async and
//! simply `await`s each pump.

use async_trait::async_trait;

use rb_model::SampleChunk;

/// A pull-based source of freshly-acquired samples.
///
/// Implementors return up to `max_samples` new samples per call, advancing their
/// own internal position. A live device produces real samples here; a synthetic
/// device (e.g. the demo driver) generates them. The returned chunk should be
/// [consistent](SampleChunk::is_consistent) and carry at most `max_samples`
/// samples; an exhausted or idle source returns an empty chunk.
#[async_trait(?Send)]
pub trait AcquisitionSource {
    /// Produces the next block of up to `max_samples` samples.
    async fn next_chunk(&mut self, max_samples: usize) -> SampleChunk;
}
