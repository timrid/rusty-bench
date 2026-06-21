//! An in-memory [`Transport`] for tests.
//!
//! `MockTransport` is the test backbone for every higher layer: it **replays** a
//! queue of synthetic or recorded bytes back to a driver's [`read`](Transport::read)
//! calls and **captures** everything the driver [`write`](Transport::write)s for
//! later assertions. Because it is pure (no I/O, no runtime) it compiles to wasm
//! and runs in plain unit tests.

use std::collections::VecDeque;

use async_trait::async_trait;

use crate::error::{TransportError, TransportResult};
use crate::transport::{Transport, TransportCapabilities, TransportKind};

/// A recorded control transfer for test assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlTransferRecord {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub data: Vec<u8>,
}

/// In-memory transport that replays queued reads and records writes.
#[derive(Debug, Clone)]
pub struct MockTransport {
    caps: TransportCapabilities,
    to_read: VecDeque<u8>,
    written: Vec<u8>,
    closed: bool,
    /// Queue of canned responses for [`control_transfer`](Transport::control_transfer).
    control_responses: VecDeque<Vec<u8>>,
    /// All control transfers issued so far.
    control_transfers: Vec<ControlTransferRecord>,
    /// Queued read errors (each consumed by one read call).
    read_errors: VecDeque<String>,
}

impl MockTransport {
    /// Creates a byte-stream mock with no transfer-size limit.
    #[must_use]
    pub fn new() -> Self {
        Self {
            caps: TransportCapabilities {
                kind: TransportKind::Mock,
                packet_oriented: false,
                max_transfer: None,
            },
            to_read: VecDeque::new(),
            written: Vec::new(),
            closed: false,
            control_responses: VecDeque::new(),
            control_transfers: Vec::new(),
            read_errors: VecDeque::new(),
        }
    }

    /// Creates a packet-oriented mock that splits reads and writes at
    /// `max_transfer` bytes, modelling e.g. a USB bulk endpoint.
    ///
    /// # Panics
    /// Panics if `max_transfer` is `0`.
    #[must_use]
    pub fn packet(max_transfer: usize) -> Self {
        assert!(max_transfer > 0, "max_transfer must be > 0");
        let mut t = Self::new();
        t.caps.packet_oriented = true;
        t.caps.max_transfer = Some(max_transfer);
        t
    }

    /// Builder: pre-loads bytes to be replayed by [`read`](Transport::read).
    #[must_use]
    pub fn with_read_data(mut self, data: impl AsRef<[u8]>) -> Self {
        self.queue_read(data);
        self
    }

    /// Appends more bytes to the replay queue.
    pub fn queue_read(&mut self, data: impl AsRef<[u8]>) {
        self.to_read.extend(data.as_ref().iter().copied());
    }

    /// Queues an I/O error for the next [`read`](Transport::read) call.
    /// The error message is preserved in the returned [`TransportError::Io`].
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

    /// Whether the transport has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queues a canned response for the next [`control_transfer`](Transport::control_transfer).
    /// Each call to `control_transfer` consumes one queued response.
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

    /// The per-call transfer cap, if any.
    fn transfer_cap(&self) -> usize {
        self.caps.max_transfer.unwrap_or(usize::MAX)
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl Transport for MockTransport {
    fn capabilities(&self) -> TransportCapabilities {
        self.caps
    }

    async fn write(&mut self, data: &[u8]) -> TransportResult<usize> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let n = data.len().min(self.transfer_cap());
        self.written.extend_from_slice(&data[..n]);
        Ok(n)
    }

    async fn read(&mut self, buf: &mut [u8]) -> TransportResult<usize> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        // Drain queued data first; only report error when out of data.
        let n = buf.len().min(self.to_read.len()).min(self.transfer_cap());
        if n > 0 {
            for slot in buf.iter_mut().take(n) {
                *slot = self.to_read.pop_front().expect("checked length above");
            }
            return Ok(n);
        }
        if let Some(msg) = self.read_errors.pop_front() {
            return Err(TransportError::Io(msg));
        }
        Ok(0)
    }

    async fn close(&mut self) -> TransportResult<()> {
        self.closed = true;
        Ok(())
    }

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
            .ok_or(TransportError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn replays_queued_bytes_in_order_then_signals_eof() {
        let mut t = MockTransport::new().with_read_data([1, 2, 3]);
        let mut buf = [0u8; 8];

        let n = block_on(t.read(&mut buf)).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);

        // Queue is now empty: a further read reports end-of-stream.
        assert_eq!(block_on(t.read(&mut buf)).unwrap(), 0);
    }

    #[test]
    fn read_is_split_across_calls_by_buffer_size() {
        let mut t = MockTransport::new().with_read_data([10, 20, 30, 40]);
        let mut small = [0u8; 3];

        assert_eq!(block_on(t.read(&mut small)).unwrap(), 3);
        assert_eq!(small, [10, 20, 30]);

        let n = block_on(t.read(&mut small)).unwrap();
        assert_eq!(n, 1);
        assert_eq!(small[0], 40);
        assert_eq!(t.remaining_read(), 0);
    }

    #[test]
    fn captures_writes_in_order() {
        let mut t = MockTransport::new();
        assert_eq!(block_on(t.write(b"AT")).unwrap(), 2);
        assert_eq!(block_on(t.write(b"+RST\r")).unwrap(), 5);
        assert_eq!(t.written(), b"AT+RST\r");
    }

    #[test]
    fn take_written_drains_the_capture_buffer() {
        let mut t = MockTransport::new();
        block_on(t.write(b"hello")).unwrap();
        assert_eq!(t.take_written(), b"hello");
        assert!(t.written().is_empty());
    }

    #[test]
    fn packet_transport_caps_reads_at_max_transfer() {
        let mut t = MockTransport::packet(2).with_read_data([1, 2, 3, 4, 5]);
        let mut buf = [0u8; 8];

        assert_eq!(block_on(t.read(&mut buf)).unwrap(), 2);
        assert_eq!(&buf[..2], &[1, 2]);
        assert_eq!(block_on(t.read(&mut buf)).unwrap(), 2);
        assert_eq!(&buf[..2], &[3, 4]);
        assert_eq!(block_on(t.read(&mut buf)).unwrap(), 1);
        assert_eq!(buf[0], 5);
    }

    #[test]
    fn packet_transport_reports_short_writes() {
        let mut t = MockTransport::packet(4);
        // Only the first 4 bytes are accepted in one transfer.
        assert_eq!(block_on(t.write(b"abcdef")).unwrap(), 4);
        assert_eq!(t.written(), b"abcd");
    }

    #[test]
    fn capabilities_reflect_constructor() {
        assert_eq!(
            MockTransport::new().capabilities(),
            TransportCapabilities {
                kind: TransportKind::Mock,
                packet_oriented: false,
                max_transfer: None,
            }
        );
        assert_eq!(
            MockTransport::packet(64).capabilities(),
            TransportCapabilities {
                kind: TransportKind::Mock,
                packet_oriented: true,
                max_transfer: Some(64),
            }
        );
    }

    #[test]
    fn operations_fail_after_close() {
        let mut t = MockTransport::new().with_read_data([1]);
        block_on(t.close()).unwrap();
        assert!(t.is_closed());

        let mut buf = [0u8; 4];
        assert!(matches!(
            block_on(t.read(&mut buf)),
            Err(TransportError::Closed)
        ));
        assert!(matches!(
            block_on(t.write(b"x")),
            Err(TransportError::Closed)
        ));
    }

    #[test]
    fn usable_as_trait_object() {
        let mut t: Box<dyn Transport> = Box::new(MockTransport::new().with_read_data([7, 8]));
        let mut buf = [0u8; 2];
        assert_eq!(block_on(t.read(&mut buf)).unwrap(), 2);
        assert_eq!(buf, [7, 8]);
    }
}
