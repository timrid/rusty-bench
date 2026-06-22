//! The [`UsbTransport`] trait — a minimal async wrapper around nusb's USB
//! primitives that exists only so drivers can be tested via
//! [`MockUsbTransport`](crate::MockUsbTransport).
//!
//! The trait mirrors nusb's [`Endpoint`](nusb::Endpoint) API as closely as
//! possible: `submit` (non-async, `&self`) + `pending` + `next_complete`
//! (async, `&mut self`).  This gives drivers full control over transfer
//! queueing — essential for devices like fx2lafw that need pipe saturation.

use async_trait::async_trait;

use crate::error::TransportResult;

/// A USB bulk-pair link, mirroring nusb's submit/pending/next_complete API.
#[async_trait(?Send)]
pub trait UsbTransport {
    // ── Bulk IN (device → host) ────────────────────────────────────────────

    /// Submits a buffer for a bulk IN transfer.  The device fills the buffer;
    /// the host receives it via [`next_bulk_in`](Self::next_bulk_in).
    fn submit_bulk_in(&mut self, buf: Vec<u8>);

    /// Number of bulk IN transfers currently in flight.
    fn pending_bulk_in(&self) -> usize;

    /// Awaits the next completed bulk IN transfer, returning the
    /// device-filled buffer.
    async fn next_bulk_in(&mut self) -> TransportResult<Vec<u8>>;

    // ── Bulk OUT (host → device) ───────────────────────────────────────────

    /// Submits `data` for a bulk OUT transfer.
    fn submit_bulk_out(&mut self, data: Vec<u8>);

    /// Number of bulk OUT transfers currently in flight.
    fn pending_bulk_out(&self) -> usize;

    /// Awaits completion of the next bulk OUT transfer, returning the
    /// now-empty buffer.
    async fn next_bulk_out(&mut self) -> TransportResult<Vec<u8>>;

    // ── Control ────────────────────────────────────────────────────────────

    /// Performs a USB control transfer (vendor, class, or standard request).
    ///
    /// * `request_type` — `bmRequestType` byte (direction + type + recipient).
    /// * `request` — `bRequest` byte.
    /// * `value` — `wValue` field.
    /// * `index` — `wIndex` field.
    /// * `data` — payload for host-to-device transfers; ignored for
    ///   device-to-host.
    ///
    /// Returns the data-phase payload (empty `Vec` for host-to-device).
    async fn control_transfer(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> TransportResult<Vec<u8>>;

    // ── Misc ───────────────────────────────────────────────────────────────

    /// Clears a halted/stalled bulk IN endpoint
    /// (`ClearFeature(ENDPOINT_HALT)`).
    ///
    /// On fx2lafw devices this is needed after a fresh plug-in to reset the
    /// bulk-IN pipe before the first acquisition.
    async fn clear_in_halt(&mut self) -> TransportResult<()>;

    /// Closes the transport.  Further operations must fail with
    /// [`TransportError::Closed`].
    async fn close(&mut self) -> TransportResult<()>;
}
