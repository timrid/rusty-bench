//! Native USB transport via the [`nusb`] crate.
//!
//! Wraps a [`nusb::Interface`] behind the [`Transport`] trait.
//! Available only with the `usb` feature.

use async_trait::async_trait;
use nusb::{
    Interface,
    transfer::{ControlIn, ControlOut, ControlType, Direction, Recipient, RequestBuffer},
};

use crate::error::{TransportError, TransportResult};
use crate::transport::{Transport, TransportCapabilities, TransportKind};

/// A [`Transport`] backed by a native USB bulk endpoint pair.
pub struct NusbTransport {
    interface: Interface,
    caps: TransportCapabilities,
    bulk_in_ep: u8,
    bulk_out_ep: u8,
}

impl NusbTransport {
    /// Wraps an opened [`nusb::Interface`] as a bulk-pair transport.
    #[must_use]
    pub fn new(interface: Interface, bulk_in_ep: u8, bulk_out_ep: u8) -> Self {
        let caps = TransportCapabilities {
            kind: TransportKind::Usb,
            packet_oriented: true,
            max_transfer: Some(512),
        };
        Self {
            interface,
            caps,
            bulk_in_ep,
            bulk_out_ep,
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
        self.interface
            .bulk_out(self.bulk_out_ep, data.to_vec())
            .await
            .into_result()
            .map_err(|e| TransportError::Io(format!("USB bulk OUT: {e}")))?;
        Ok(data.len())
    }

    async fn read(&mut self, buf: &mut [u8]) -> TransportResult<usize> {
        let data = self
            .interface
            .bulk_in(self.bulk_in_ep, RequestBuffer::new(buf.len()))
            .await
            .into_result()
            .map_err(|e| TransportError::Io(format!("USB bulk IN: {e}")))?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
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
                    .control_in(ControlIn {
                        control_type: ctrl,
                        recipient: rec,
                        request,
                        value,
                        index,
                        length: len,
                    })
                    .await
                    .into_result()
                    .map_err(|e| TransportError::Io(format!("USB control IN: {e}")))
            }
            Direction::Out => {
                self.interface
                    .control_out(ControlOut {
                        control_type: ctrl,
                        recipient: rec,
                        request,
                        value,
                        index,
                        data,
                    })
                    .await
                    .into_result()
                    .map_err(|e| TransportError::Io(format!("USB control OUT: {e}")))?;
                Ok(Vec::new())
            }
        }
    }
}
