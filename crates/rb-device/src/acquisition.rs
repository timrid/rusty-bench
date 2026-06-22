//! The bulk-data seam: [`AcquisitionSource`].
//!
//! The control plane (arming, configuring) flows through the `async` capability
//! traits ([`LogicAnalyzer::arm`], …).  The bulk path is **push-based**:
//! [`start_streaming`](AcquisitionSource::start_streaming) arms the device and
//! returns a read‑loop future that the runtime polls via [`select!`]
//! concurrently with command processing — no explicit pump step.  The read loop
//! keeps the transport saturated by reading in a tight loop and pushing
//! [`SampleChunk`]s through a channel.

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use futures::channel::mpsc;

use crate::DeviceResult;
use rb_model::SampleChunk;

/// A push-based source of freshly-acquired samples.
///
/// [`start_streaming`] arms the device and returns a read‑loop future.  The
/// runtime polls the read‑loop future alongside the command stream via
/// [`select!`], so the transport stays saturated without explicit pump/yield
/// cycles.
///
/// For synthetic / rate‑limited devices the read loop can use timers to
/// control throughput — the runtime is unaware of the device's pacing.
#[async_trait(?Send)]
pub trait AcquisitionSource {
    /// Arm the device and return a future that implements the read loop.
    ///
    /// The returned future must read samples and push [`SampleChunk`]s through
    /// `chunk_tx`.  The runtime polls this future concurrently with command
    /// processing.  The future should exit when [`stop_streaming`] signals stop
    /// or when the receiver is dropped.
    async fn start_streaming(
        &mut self,
        chunk_tx: mpsc::UnboundedSender<SampleChunk>,
    ) -> DeviceResult<Pin<Box<dyn Future<Output = ()>>>>;

    /// Signal the read loop to stop.  Any remaining buffered samples should be
    /// flushed to the sender before the future exits.
    async fn stop_streaming(&mut self) -> DeviceResult<()>;
}
