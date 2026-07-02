//! Native USB transport via the [`nusb`] crate.
//!
//! Wraps a [`nusb::Interface`] behind the [`UsbTransport`] trait.
//! Available only with the `usb` feature.

use std::time::Duration;

use async_trait::async_trait;
use nusb::{
    Interface,
    transfer::{Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient},
};

use crate::error::{TransportError, TransportResult};
use crate::transport::UsbTransport;

/// Timeout for USB control transfers.
const CONTROL_TIMEOUT: Duration = Duration::from_millis(1000);

/// A native USB bulk endpoint pair behind [`UsbTransport`].
///
/// Bulk transfers use the nusb 0.2 [`Endpoint`](nusb::Endpoint) API:
/// [`submit`](nusb::Endpoint::submit) + [`next_complete`](nusb::Endpoint::next_complete).
pub struct NusbTransport {
    interface: Interface,
    ep_in: Option<nusb::Endpoint<Bulk, In>>,
    ep_out: Option<nusb::Endpoint<Bulk, Out>>,
}

impl NusbTransport {
    /// Wraps an opened [`nusb::Interface`] as a bulk-pair transport.
    ///
    /// Pass `0x00` for endpoints that are not used (fx2lafw has no bulk OUT,
    /// for example).
    ///
    /// # Errors
    /// Returns an error if a non-zero endpoint address cannot be opened.
    pub fn new(
        interface: Interface,
        bulk_in_ep: u8,
        bulk_out_ep: u8,
    ) -> TransportResult<Self> {
        let ep_in = if bulk_in_ep != 0x00 {
            Some(
                interface
                    .endpoint::<Bulk, In>(bulk_in_ep)
                    .map_err(|e| {
                        TransportError::Io(format!(
                            "bulk IN endpoint 0x{bulk_in_ep:02X} not found: {e}"
                        ))
                    })?,
            )
        } else {
            None
        };
        let ep_out = if bulk_out_ep != 0x00 {
            Some(
                interface
                    .endpoint::<Bulk, Out>(bulk_out_ep)
                    .map_err(|e| {
                        TransportError::Io(format!(
                            "bulk OUT endpoint 0x{bulk_out_ep:02X} not found: {e}"
                        ))
                    })?,
            )
        } else {
            None
        };
        Ok(Self {
            interface,
            ep_in,
            ep_out,
        })
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
impl UsbTransport for NusbTransport {
    // ── Bulk IN ────────────────────────────────────────────────────────────

    fn submit_bulk_in(&mut self, buf: Vec<u8>) {
        self.ep_in
            .as_mut()
            .expect("submit_bulk_in: no bulk IN endpoint")
            .submit(buf.into());
    }

    fn pending_bulk_in(&self) -> usize {
        self.ep_in
            .as_ref()
            .map_or(0, |ep| ep.pending())
    }

    async fn next_bulk_in(&mut self) -> TransportResult<Vec<u8>> {
        let ep = self
            .ep_in
            .as_mut()
            .ok_or_else(|| TransportError::Io("no bulk IN endpoint".into()))?;
        let completed = ep.next_complete().await;
        completed
            .status
            .map_err(|e| TransportError::Io(format!("USB bulk IN: {e}")))?;
        log::debug!("nusb: bulk IN completed, len={}", completed.buffer.len());
        Ok(completed.buffer.to_vec())
    }

    // ── Bulk OUT ───────────────────────────────────────────────────────────

    fn submit_bulk_out(&mut self, data: Vec<u8>) {
        self.ep_out
            .as_mut()
            .expect("submit_bulk_out: no bulk OUT endpoint")
            .submit(data.into());
    }

    fn pending_bulk_out(&self) -> usize {
        self.ep_out
            .as_ref()
            .map_or(0, |ep| ep.pending())
    }

    async fn next_bulk_out(&mut self) -> TransportResult<Vec<u8>> {
        let ep = self
            .ep_out
            .as_mut()
            .ok_or_else(|| TransportError::Io("no bulk OUT endpoint".into()))?;
        let completed = ep.next_complete().await;
        completed
            .status
            .map_err(|e| TransportError::Io(format!("USB bulk OUT: {e}")))?;
        Ok(completed.buffer.to_vec())
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
                    .await
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
                    .await
                    .map_err(|e| TransportError::Io(format!("USB control OUT: {e}")))?;
                Ok(Vec::new())
            }
        }
    }

    // ── Misc ───────────────────────────────────────────────────────────────

    async fn close(&mut self) -> TransportResult<()> {
        Ok(())
    }

    async fn clear_in_halt(&mut self) -> TransportResult<()> {
        if let Some(ref mut ep) = self.ep_in {
            ep.clear_halt()
                .await
                .map_err(|e| TransportError::Io(format!("clear_in_halt: {e}")))?;
        }
        Ok(())
    }
}
