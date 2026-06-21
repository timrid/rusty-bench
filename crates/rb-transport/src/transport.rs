//! The [`Transport`] trait and its capability description.
//!
//! A transport is the byte/packet link a driver speaks over, independent of the
//! concrete medium. Drivers are written against this trait only ("I need a
//! byte stream" / "I need bulk packets"), never against a concrete library, so
//! the same driver runs over a native USB stack or WebUSB unchanged.

use async_trait::async_trait;

use crate::error::{TransportError, TransportResult};

/// The kind of physical/logical medium a transport rides on.
///
/// Used together with build-target gating so a [`DriverFactory`] can be filtered
/// out on platforms where its transport does not exist (e.g. raw TCP on the web).
///
/// [`DriverFactory`]: crate::DriverFactory
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportKind {
    /// USB (native libusb-style stacks, or WebUSB in the browser).
    Usb,
    /// Serial / UART (native serial ports, or Web Serial).
    Serial,
    /// Bluetooth (classic or LE).
    Bluetooth,
    /// Ethernet / IP sockets.
    Ethernet,
    /// In-memory transport for tests ([`MockTransport`](crate::MockTransport)).
    Mock,
}

/// Static description of what a transport can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportCapabilities {
    /// Which medium this transport rides on.
    pub kind: TransportKind,
    /// `true` for message/packet links (e.g. USB bulk), `false` for raw byte
    /// streams (e.g. serial). Packet links never coalesce or split a logical
    /// message beyond [`max_transfer`](Self::max_transfer).
    pub packet_oriented: bool,
    /// Largest single transfer in bytes, if the medium imposes one.
    pub max_transfer: Option<usize>,
}

/// A byte/packet-oriented link a driver communicates over.
///
/// Methods are `async` via `async-trait` in `?Send` mode so a single trait works
/// for native runtimes and for non-`Send` web futures (WebUSB/Web Serial).
#[async_trait(?Send)]
pub trait Transport {
    /// Static description of this transport's medium and limits.
    fn capabilities(&self) -> TransportCapabilities;

    /// Writes bytes to the peer, returning the number actually accepted.
    ///
    /// On a packet-oriented transport a single call corresponds to at most one
    /// transfer of up to [`max_transfer`](TransportCapabilities::max_transfer)
    /// bytes; callers must be prepared for a short write.
    async fn write(&mut self, data: &[u8]) -> TransportResult<usize>;

    /// Reads bytes from the peer into `buf`, returning the number read.
    ///
    /// A return value of `0` means end-of-stream (no more data will arrive).
    async fn read(&mut self, buf: &mut [u8]) -> TransportResult<usize>;

    /// Flushes any buffered outbound bytes. Defaults to a no-op.
    async fn flush(&mut self) -> TransportResult<()> {
        Ok(())
    }

    /// Closes the transport. Further reads/writes must fail.
    async fn close(&mut self) -> TransportResult<()> {
        Ok(())
    }

    /// Performs a USB control transfer (vendor/class requests).
    ///
    /// - `request_type`: bmRequestType byte (direction + recipient + type).
    /// - `request`: bRequest byte.
    /// - `value`: wValue field.
    /// - `index`: wIndex field.
    /// - `data`: payload for host-to-device transfers; ignored for device-to-host.
    ///
    /// Returns the data phase payload (empty for host-to-device).
    ///
    /// The default implementation returns [`TransportError::Unsupported`].
    async fn control_transfer(
        &mut self,
        _request_type: u8,
        _request: u8,
        _value: u16,
        _index: u16,
        _data: &[u8],
    ) -> TransportResult<Vec<u8>> {
        Err(TransportError::Unsupported)
    }
}
