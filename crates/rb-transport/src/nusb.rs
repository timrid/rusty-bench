//! Native USB transport via the [`nusb`] crate.
//!
//! Wraps a [`nusb::Interface`] behind the [`Transport`] trait.
//! Available only with the `usb` feature.

use std::time::Duration;

use async_trait::async_trait;
use nusb::{
    Interface, MaybeFuture,
    transfer::{
        Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient,
    },
};

use crate::error::{TransportError, TransportResult};
use crate::transport::{Transport, TransportCapabilities, TransportKind};

/// Timeout for USB control transfers.
const CONTROL_TIMEOUT: Duration = Duration::from_millis(1000);

/// A [`Transport`] backed by a native USB bulk endpoint pair.
///
/// Bulk transfers use the nusb 0.2 [`Endpoint`](nusb::Endpoint) API:
/// [`submit`](nusb::Endpoint::submit) + [`next_complete`](nusb::Endpoint::next_complete).
pub struct NusbTransport {
    interface: Interface,
    caps: TransportCapabilities,
    ep_in: Option<nusb::Endpoint<Bulk, In>>,
    ep_out: Option<nusb::Endpoint<Bulk, Out>>,
}

impl NusbTransport {
    /// Wraps an opened [`nusb::Interface`] as a bulk-pair transport.
    ///
    /// Pass `0x00` for endpoints that are not used (fx2lafw has no bulk OUT,
    /// for example).  Opening a non-zero endpoint address that does not exist
    /// on the interface will panic.
    ///
    /// # Panics
    /// Panics if a non-zero endpoint address cannot be opened.
    #[must_use]
    pub fn new(interface: Interface, bulk_in_ep: u8, bulk_out_ep: u8) -> Self {
        let caps = TransportCapabilities {
            kind: TransportKind::Usb,
            packet_oriented: true,
            max_transfer: Some(512),
        };
        let ep_in = if bulk_in_ep != 0x00 {
            Some(
                interface
                    .endpoint::<Bulk, In>(bulk_in_ep)
                    .expect("bulk IN endpoint not found on interface"),
            )
        } else {
            None
        };
        let ep_out = if bulk_out_ep != 0x00 {
            Some(
                interface
                    .endpoint::<Bulk, Out>(bulk_out_ep)
                    .expect("bulk OUT endpoint not found on interface"),
            )
        } else {
            None
        };
        Self {
            interface,
            caps,
            ep_in,
            ep_out,
        }
    }
}

fn decode_bmrequest_type(bm: u8) -> (ControlType, Direction, Recipient) {
    let dir = if bm & 0x80 != 0 {
        Direction::In
    } else {
        Direction::Out
    };
    let ctrl = match (bm >> 5) & 0x03 {
        0 => ControlType::Standard,
        1 => ControlType::Class,
        _ => ControlType::Vendor,
    };
    let rec = match bm & 0x1F {
        0 => Recipient::Device,
        1 => Recipient::Interface,
        2 => Recipient::Endpoint,
        _ => Recipient::Other,
    };
    (ctrl, dir, rec)
}

#[async_trait(?Send)]
impl Transport for NusbTransport {
    fn capabilities(&self) -> TransportCapabilities {
        self.caps
    }

    async fn write(&mut self, data: &[u8]) -> TransportResult<usize> {
        let ep = self
            .ep_out
            .as_mut()
            .ok_or_else(|| TransportError::Io("no bulk OUT endpoint".into()))?;
        ep.submit(data.to_vec().into());
        let completed = ep.next_complete().await;
        completed
            .status
            .map_err(|e| TransportError::Io(format!("USB bulk OUT: {e}")))?;
        Ok(data.len())
    }

    async fn read(&mut self, buf: &mut [u8]) -> TransportResult<usize> {
        let ep = self
            .ep_in
            .as_mut()
            .ok_or_else(|| TransportError::Io("no bulk IN endpoint".into()))?;
        ep.submit(vec![0u8; buf.len()].into());
        let completed = ep.next_complete().await;
        completed
            .status
            .map_err(|e| TransportError::Io(format!("USB bulk IN: {e}")))?;
        let n = completed.buffer.len().min(buf.len());
        buf[..n].copy_from_slice(&completed.buffer[..n]);
        Ok(n)
    }

    async fn close(&mut self) -> TransportResult<()> {
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
        let (ctrl, dir, rec) = decode_bmrequest_type(request_type);
        match dir {
            Direction::In => {
                let len = if data.is_empty() {
                    64
                } else {
                    data.len() as u16
                };
                self.interface
                    .control_in(
                        ControlIn {
                            control_type: ctrl,
                            recipient: rec,
                            request,
                            value,
                            index,
                            length: len,
                        },
                        CONTROL_TIMEOUT,
                    )
                    .wait()
                    .map_err(|e| TransportError::Io(format!("USB control IN: {e}")))
            }
            Direction::Out => {
                self.interface
                    .control_out(
                        ControlOut {
                            control_type: ctrl,
                            recipient: rec,
                            request,
                            value,
                            index,
                            data,
                        },
                        CONTROL_TIMEOUT,
                    )
                    .wait()
                    .map_err(|e| TransportError::Io(format!("USB control OUT: {e}")))?;
                Ok(Vec::new())
            }
        }
    }
}
