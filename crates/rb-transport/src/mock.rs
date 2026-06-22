//! An in-memory [`UsbTransport`] for tests.
//!
//! `MockUsbTransport` is the test backbone for every higher layer: it **replays** a
//! queue of synthetic or recorded bytes back to a driver's
//! [`next_bulk_in`](UsbTransport::next_bulk_in) calls and **captures** everything
//! the driver [`submit_bulk_out`](UsbTransport::submit_bulk_out)s for later
//! assertions. Because it is pure (no I/O, no runtime) it compiles to wasm
//! and runs in plain unit tests.

use std::collections::VecDeque;

use async_trait::async_trait;

use crate::error::{TransportError, TransportResult};
use crate::transport::UsbTransport;

/// A recorded control transfer for test assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlTransferRecord {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub data: Vec<u8>,
}

/// In-memory USB transport that replays queued reads and records writes.
#[derive(Debug, Clone)]
pub struct MockUsbTransport {
    to_read: VecDeque<u8>,
    written: Vec<u8>,
    closed: bool,
    /// Whether `clear_in_halt` has been called.
    halt_cleared: bool,
    /// Per-call transfer cap (0 = no cap).
    transfer_cap: usize,
    /// Number of bulk IN transfers in flight.
    pending_in: usize,
    /// Number of bulk OUT transfers in flight.
    pending_out: usize,
    /// Canned responses for [`control_transfer`](UsbTransport::control_transfer).
    control_responses: VecDeque<Vec<u8>>,
    /// All control transfers issued so far.
    control_transfers: Vec<ControlTransferRecord>,
    /// Queued read errors (each consumed by one `next_bulk_in` call).
    read_errors: VecDeque<String>,
}

impl MockUsbTransport {
    /// Creates a byte-stream mock with no transfer-size limit.
    #[must_use]
    pub fn new() -> Self {
        Self {
            to_read: VecDeque::new(),
            written: Vec::new(),
            closed: false,
            halt_cleared: false,
            transfer_cap: 0,
            pending_in: 0,
            pending_out: 0,
            control_responses: VecDeque::new(),
            control_transfers: Vec::new(),
            read_errors: VecDeque::new(),
        }
    }

    /// Creates a packet-oriented mock that splits reads at
    /// `max_transfer` bytes, modelling a USB bulk endpoint.
    #[must_use]
    pub fn packet(max_transfer: usize) -> Self {
        assert!(max_transfer > 0, "max_transfer must be > 0");
        let mut t = Self::new();
        t.transfer_cap = max_transfer;
        t
    }

    /// Appends more bytes to the replay queue.
    pub fn queue_read(&mut self, data: impl AsRef<[u8]>) {
        self.to_read.extend(data.as_ref().iter().copied());
    }

    /// Builder: pre-loads bytes to be replayed by [`next_bulk_in`](UsbTransport::next_bulk_in).
    #[must_use]
    pub fn with_read_data(mut self, data: impl AsRef<[u8]>) -> Self {
        self.queue_read(data);
        self
    }

    /// Queues an I/O error for the next [`next_bulk_in`](UsbTransport::next_bulk_in) call.
    pub fn queue_read_error(&mut self, msg: impl Into<String>) {
        self.read_errors.push_back(msg.into());
    }

    /// All bytes written so far, in order.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        &self.written
    }

    /// Drains and returns all captured writes, clearing the buffer.
    pub fn take_written(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.written)
    }

    /// Number of bytes still queued for replay.
    #[must_use]
    pub fn remaining_read(&self) -> usize {
        self.to_read.len()
    }

    /// Whether `clear_in_halt` has been called on this transport.
    #[must_use]
    pub fn halt_was_cleared(&self) -> bool {
        self.halt_cleared
    }

    /// Whether the transport has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queues a canned response for the next [`control_transfer`](UsbTransport::control_transfer).
    pub fn queue_control_response(&mut self, data: impl AsRef<[u8]>) {
        self.control_responses.push_back(data.as_ref().to_vec());
    }

    /// Builder: pre-loads a control-transfer response.
    #[must_use]
    pub fn with_control_response(mut self, data: impl AsRef<[u8]>) -> Self {
        self.queue_control_response(data);
        self
    }

    /// All control transfers issued so far (for test assertions).
    #[must_use]
    pub fn control_transfers(&self) -> &[ControlTransferRecord] {
        &self.control_transfers
    }

    /// Drains and returns all recorded control transfers.
    pub fn take_control_transfers(&mut self) -> Vec<ControlTransferRecord> {
        core::mem::take(&mut self.control_transfers)
    }
}

impl Default for MockUsbTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl UsbTransport for MockUsbTransport {
    // ── Bulk IN ────────────────────────────────────────────────────────────

    fn submit_bulk_in(&mut self, _buf: Vec<u8>) {
        self.pending_in += 1;
    }

    fn pending_bulk_in(&self) -> usize {
        self.pending_in
    }

    async fn next_bulk_in(&mut self) -> TransportResult<Vec<u8>> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if self.pending_in == 0 {
            return Err(TransportError::Io(
                "next_bulk_in called with no transfers in flight".into(),
            ));
        }
        self.pending_in -= 1;

        let cap = if self.transfer_cap > 0 {
            self.transfer_cap
        } else {
            usize::MAX
        };
        let n = self.to_read.len().min(cap);
        if n > 0 {
            let data: Vec<u8> = self.to_read.drain(..n).collect();
            return Ok(data);
        }
        if let Some(msg) = self.read_errors.pop_front() {
            return Err(TransportError::Io(msg));
        }
        Ok(Vec::new()) // EOF
    }

    // ── Bulk OUT ───────────────────────────────────────────────────────────

    fn submit_bulk_out(&mut self, data: Vec<u8>) {
        self.written.extend_from_slice(&data);
        self.pending_out += 1;
    }

    fn pending_bulk_out(&self) -> usize {
        self.pending_out
    }

    async fn next_bulk_out(&mut self) -> TransportResult<Vec<u8>> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if self.pending_out == 0 {
            return Err(TransportError::Io(
                "next_bulk_out called with no transfers in flight".into(),
            ));
        }
        self.pending_out -= 1;
        Ok(Vec::new())
    }

    // ── Control ────────────────────────────────────────────────────────────

    async fn control_transfer(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> TransportResult<Vec<u8>> {
        self.control_transfers.push(ControlTransferRecord {
            request_type,
            request,
            value,
            index,
            data: data.to_vec(),
        });
        self.control_responses
            .pop_front()
            .ok_or_else(|| TransportError::Io("no queued control response".into()))
    }

    // ── Misc ───────────────────────────────────────────────────────────────

    async fn close(&mut self) -> TransportResult<()> {
        self.closed = true;
        Ok(())
    }

    async fn clear_in_halt(&mut self) -> TransportResult<()> {
        self.halt_cleared = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn replays_queued_bytes_in_order_then_signals_eof() {
        let mut t = MockUsbTransport::new().with_read_data([1, 2, 3]);
        t.submit_bulk_in(vec![0u8; 8]);
        assert_eq!(t.pending_bulk_in(), 1);

        let data = block_on(t.next_bulk_in()).unwrap();
        assert_eq!(data, vec![1, 2, 3]);

        // Queue is now empty: next read signals EOF.
        t.submit_bulk_in(vec![0u8; 8]);
        assert!(block_on(t.next_bulk_in()).unwrap().is_empty());
    }

    #[test]
    fn read_is_split_across_calls_by_transfer_cap() {
        let mut t = MockUsbTransport::packet(3).with_read_data([10, 20, 30, 40]);
        t.submit_bulk_in(vec![0u8; 3]);
        assert_eq!(block_on(t.next_bulk_in()).unwrap(), vec![10, 20, 30]);

        t.submit_bulk_in(vec![0u8; 3]);
        assert_eq!(block_on(t.next_bulk_in()).unwrap(), vec![40]);

        assert_eq!(t.remaining_read(), 0);
    }

    #[test]
    fn captures_writes_in_order() {
        let mut t = MockUsbTransport::new();
        t.submit_bulk_out(b"AT".to_vec());
        block_on(t.next_bulk_out()).unwrap();
        t.submit_bulk_out(b"+RST\r".to_vec());
        block_on(t.next_bulk_out()).unwrap();
        assert_eq!(t.written(), b"AT+RST\r");
    }

    #[test]
    fn take_written_drains_the_capture_buffer() {
        let mut t = MockUsbTransport::new();
        t.submit_bulk_out(b"hello".to_vec());
        block_on(t.next_bulk_out()).unwrap();
        assert_eq!(t.take_written(), b"hello");
        assert!(t.written().is_empty());
    }

    #[test]
    fn packet_transport_caps_reads_at_max_transfer() {
        let mut t = MockUsbTransport::packet(2).with_read_data([1, 2, 3, 4, 5]);

        t.submit_bulk_in(vec![0u8; 2]);
        assert_eq!(block_on(t.next_bulk_in()).unwrap(), vec![1, 2]);
        t.submit_bulk_in(vec![0u8; 2]);
        assert_eq!(block_on(t.next_bulk_in()).unwrap(), vec![3, 4]);
        t.submit_bulk_in(vec![0u8; 2]);
        assert_eq!(block_on(t.next_bulk_in()).unwrap(), vec![5]);
    }

    #[test]
    fn operations_fail_after_close() {
        let mut t = MockUsbTransport::new().with_read_data([1]);
        block_on(t.close()).unwrap();
        assert!(t.is_closed());

        t.submit_bulk_in(vec![0u8; 4]);
        assert!(matches!(
            block_on(t.next_bulk_in()),
            Err(TransportError::Closed)
        ));
        t.submit_bulk_out(b"x".to_vec());
        assert!(matches!(
            block_on(t.next_bulk_out()),
            Err(TransportError::Closed)
        ));
    }

    #[test]
    fn usable_as_trait_object() {
        let mut t: Box<dyn UsbTransport> =
            Box::new(MockUsbTransport::new().with_read_data([7, 8]));
        t.submit_bulk_in(vec![0u8; 2]);
        let data = block_on(t.next_bulk_in()).unwrap();
        assert_eq!(data, vec![7, 8]);
    }
}
