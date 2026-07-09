//! Runtime-agnostic acquisition glue.
//!
//! [`run_acquisition`] polls a pre-armed device's read-loop future and its
//! data receiver concurrently via [`select!`]. Each [`SampleChunk`] is
//! forwarded to the optional GUI sender. The function binds **no** runtime —
//! it only uses `futures`, so it is wasm‑safe and testable with a plain executor.
//!
//! The runtime‑specific *spawners* live in the [`native`] (tokio) and [`web`]
//! (`wasm-bindgen-futures`) submodules, each enabled by its own Cargo feature.
//!
//! # Usage
//!
//! The consumer (GUI Tab, CLI) is responsible for:
//! 1. Obtaining a [`DeviceHandle`] from the [`DeviceManager`].
//! 2. Calling capability methods (`arm()`, `start_streaming()`) directly.
//! 3. Spawning `run_acquisition` with the resulting read-loop and data receiver.
//! 4. Stopping by dropping the data sender; calling `stop()` / `stop_streaming()`.
//! 5. Returning the [`DeviceHandle`] when done.

use std::future::Future;
use std::pin::Pin;

use futures::channel::mpsc;
use futures::FutureExt;
use futures::StreamExt;
use log::debug;
use rb_model::SampleChunk;

/// Polls a pre-armed device's read-loop future and data receiver concurrently.
///
/// Each [`SampleChunk`] from `data_rx` is forwarded to `gui_tx` (if provided).
/// The function returns when either the data receiver is exhausted or the
/// read-loop future completes. Dropping the sender end of `data_rx` is the
/// signal to stop.
pub async fn run_acquisition(
    read_loop: Pin<Box<dyn Future<Output = ()>>>,
    data_rx: mpsc::UnboundedReceiver<SampleChunk>,
    gui_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
) {
    let mut data_rx = data_rx.fuse();
    let mut read_loop = read_loop.fuse();

    loop {
        futures::select! {
            chunk = data_rx.next() => {
                match chunk {
                    Some(chunk) => {
                        if let Some(ref tx) = gui_tx {
                            let _ = tx.unbounded_send(chunk);
                        }
                    }
                    None => {
                        debug!("data receiver closed, exiting");
                        return;
                    }
                }
            }
            _ = read_loop => {
                debug!("read loop exited, draining remaining data");
                while let Some(Some(chunk)) = data_rx.next().now_or_never() {
                    if let Some(ref tx) = gui_tx {
                        let _ = tx.unbounded_send(chunk);
                    }
                }
                return;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Native spawner (tokio)
// ═══════════════════════════════════════════════════════════════════════════════

/// Native acquisition spawner: a current-thread tokio task running the
/// continuous acquisition loop.
///
/// The device capability futures are `?Send`, so they cannot ride
/// `tokio::spawn`; the spawner uses [`tokio::task::spawn_local`] and must run
/// inside a [`tokio::task::LocalSet`].
#[cfg(feature = "native")]
pub mod native {
    use std::future::Future;
    use std::pin::Pin;

    use futures::channel::mpsc;
    use rb_model::SampleChunk;

    /// Handle to a spawned acquisition task.
    ///
    /// Drop `data_tx` to signal stop. Call `finish()` to await completion.
    pub struct AcquisitionController {
        /// Drop this to stop streaming.
        pub data_tx: mpsc::UnboundedSender<SampleChunk>,
        join: tokio::task::JoinHandle<()>,
    }

    impl AcquisitionController {
        /// Spawns an acquisition task. Must be called inside a
        /// [`tokio::task::LocalSet`].
        ///
        /// The caller is responsible for arming the device and starting
        /// streaming before calling this. The returned `data_tx` should
        /// be connected to the device's streaming source.
        #[must_use]
        pub fn spawn(
            read_loop: Pin<Box<dyn Future<Output = ()>>>,
            data_rx: mpsc::UnboundedReceiver<SampleChunk>,
            gui_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
            data_tx: mpsc::UnboundedSender<SampleChunk>,
        ) -> Self {
            let join = tokio::task::spawn_local(super::run_acquisition(
                read_loop, data_rx, gui_tx,
            ));
            Self { data_tx, join }
        }

        /// Stops the task (by dropping the data sender) and awaits completion.
        pub async fn finish(self) {
            drop(self.data_tx);
            let _ = self.join.await;
        }
    }
}

/// Web acquisition spawner: runs the continuous loop on `wasm-bindgen-futures`.
#[cfg(feature = "web")]
pub mod web {
    use std::future::Future;
    use std::pin::Pin;

    use futures::channel::mpsc;
    use rb_model::SampleChunk;

    /// Handle for a web-spawned acquisition.
    ///
    /// Drop `data_tx` to stop. The task detaches and completes independently.
    pub struct WebAcquisitionHandle {
        /// Drop this to stop streaming.
        pub data_tx: mpsc::UnboundedSender<SampleChunk>,
    }

    /// Spawns the acquisition loop as a local (non-`Send`) task and returns a
    /// [`WebAcquisitionHandle`]. Dropping `data_tx` stops the task.
    pub fn spawn_local(
        read_loop: Pin<Box<dyn Future<Output = ()>>>,
        data_rx: mpsc::UnboundedReceiver<SampleChunk>,
        gui_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
        data_tx: mpsc::UnboundedSender<SampleChunk>,
    ) -> WebAcquisitionHandle {
        let h = WebAcquisitionHandle {
            data_tx: data_tx.clone(),
        };
        wasm_bindgen_futures::spawn_local(async move {
            super::run_acquisition(read_loop, data_rx, gui_tx).await;
        });
        h
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;

    #[test]
    fn data_rx_closure_ends_the_loop() {
        let (_data_tx, data_rx) = mpsc::unbounded::<SampleChunk>();
        let (gui_tx, _gui_rx) = mpsc::unbounded::<SampleChunk>();
        let read_loop = Box::pin(futures::future::pending::<()>());

        drop(_data_tx);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(run_acquisition(read_loop, data_rx, Some(gui_tx)));
    }

    #[test]
    fn read_loop_completion_ends_the_loop() {
        let (data_tx, data_rx) = mpsc::unbounded::<SampleChunk>();
        let (gui_tx, _gui_rx) = mpsc::unbounded::<SampleChunk>();

        let read_loop = Box::pin(futures::future::ready(()));

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(run_acquisition(read_loop, data_rx, Some(gui_tx)));
        drop(data_tx);
    }
}
