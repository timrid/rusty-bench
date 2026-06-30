//! Runtime-agnostic acquisition glue.
//!
//! [`run_acquisition`] is the heart of the acquisition loop and binds **no**
//! runtime: it consumes a stream of [`AcquisitionCommand`]s and drives a
//! push‑based data flow.  It only uses `futures`, so it is wasm‑safe and can be
//! tested with a plain executor.
//!
//! On [`Start`](AcquisitionCommand::Start), the device's [`AcquisitionSource`]
//! is armed and its read‑loop future is polled concurrently with the command
//! stream via [`select!`].  Data chunks are ingested into the device's stores
//! and forwarded to the optional GUI sender.
//!
//! The runtime‑specific *spawners* live in the [`native`] (tokio) and [`web`]
//! (`wasm-bindgen-futures`) submodules, each enabled by its own Cargo feature.

use std::future::Future;
use std::pin::Pin;

use futures::FutureExt;
use futures::channel::mpsc;
use futures::stream::{Stream, StreamExt};
use log::{debug, info};

use crate::handle::{AcquisitionCommand, DeviceHandle};
use rb_model::SampleChunk;

/// Drives a device's acquisition: on [`Start`](AcquisitionCommand::Start), arms
/// the device and begins concurrent polling of commands and data via
/// [`select!`].  No explicit pump — the device's read‑loop future is polled
/// directly, keeping the transport saturated.
///
/// The loop ends when the command stream closes.
pub async fn run_acquisition<C>(
    mut handle: DeviceHandle,
    mut commands: C,
    gui_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
) -> DeviceHandle
where
    C: Stream<Item = AcquisitionCommand> + Unpin,
{
    // Phase 1: wait for Start command.
    loop {
        match commands.next().await {
            Some(AcquisitionCommand::Start) => {
                info!("start command received, arming device");
                match handle.start_streaming().await {
                    Ok((read_loop, data_rx)) => {
                        debug!("streaming started, entering select! loop");
                        return run_active(handle, commands, read_loop, data_rx, gui_tx).await;
                    }
                    Err(e) => {
                        handle.mark_error(e.to_string());
                        return handle;
                    }
                }
            }
            Some(cmd) => {
                info!("command: {cmd:?}");
                if let Err(e) = handle.apply(cmd).await {
                    handle.mark_error(e.to_string());
                }
            }
            None => {
                debug!("channel closed before start");
                return handle;
            }
        }
    }
}

/// Active streaming phase: polls the read‑loop future, data receiver, and
/// command stream concurrently via [`select!`].
async fn run_active<C>(
    mut handle: DeviceHandle,
    commands: C,
    read_loop: Pin<Box<dyn Future<Output = ()>>>,
    data_rx: mpsc::UnboundedReceiver<SampleChunk>,
    gui_tx: Option<mpsc::UnboundedSender<SampleChunk>>,
) -> DeviceHandle
where
    C: Stream<Item = AcquisitionCommand> + Unpin,
{
    let mut commands = commands.fuse();
    let mut data_rx = data_rx.fuse();
    let mut read_loop = read_loop.fuse();

    // Loop until the command channel closes.  Re-enters streaming on Stop → Start.
    loop {
        futures::select! {
            cmd = commands.next() => {
                match cmd {
                    Some(command) => {
                        info!("command: {command:?}");
                        if let Err(error) = handle.apply(command).await {
                            handle.mark_error(error.to_string());
                        }
                        // On Stop: drain remaining data, then wait for next Start or close.
                        if !matches!(handle.state(), &crate::handle::AcquisitionState::Running) {
                            debug!("device not running, draining data");
                            while let Some(Some(chunk)) = data_rx.next().now_or_never() {
                                handle.ingest_chunk(&chunk);
                                if let Some(ref tx) = gui_tx {
                                    let _ = tx.unbounded_send(chunk);
                                }
                            }
                            // Wait for re-arm, read-loop exit, or channel close.
                            // Must also poll read_loop so it can exit and return
                            // its transport (needed for re-arm on drivers like fx2lafw).
                            loop {
                                futures::select! {
                                    cmd = commands.next() => {
                                        match cmd {
                                            Some(AcquisitionCommand::Start) => {
                                                info!("re-arming device");
                                                match handle.start_streaming().await {
                                                    Ok((rl, rx)) => {
                                                        read_loop = rl.fuse();
                                                        data_rx = rx.fuse();
                                                        break; // back to streaming select!
                                                    }
                                                    Err(e) => {
                                                        handle.mark_error(e.to_string());
                                                        return handle;
                                                    }
                                                }
                                            }
                                            Some(cmd) => {
                                                if let Err(e) = handle.apply(cmd).await {
                                                    handle.mark_error(e.to_string());
                                                }
                                            }
                                            None => return handle,
                                        }
                                    }
                                    // Drain any late-arriving data while stopped.
                                    chunk = data_rx.next() => {
                                        if let Some(chunk) = chunk {
                                            handle.ingest_chunk(&chunk);
                                            if let Some(ref tx) = gui_tx {
                                                let _ = tx.unbounded_send(chunk);
                                            }
                                        }
                                    }
                                    // Read loop may exit after stop; let it complete.
                                    _ = read_loop => {
                                        debug!("read loop exited after stop");
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        debug!("channel closed, exiting");
                        let _ = handle.apply(AcquisitionCommand::Stop).await;
                        return handle;
                    }
                }
            }
            chunk = data_rx.next() => {
                if let Some(chunk) = chunk {
                    let _count = handle.ingest_chunk(&chunk);
                    if let Some(ref tx) = gui_tx {
                        let _ = tx.unbounded_send(chunk);
                    }
                }
            }
            _ = read_loop => {
                debug!("read loop exited");
                // Read loop ended — drain remaining chunks.
                while let Some(Some(chunk)) = data_rx.next().now_or_never() {
                    handle.ingest_chunk(&chunk);
                    if let Some(ref tx) = gui_tx {
                        let _ = tx.unbounded_send(chunk);
                    }
                }
                // Wait for re-arm or close.  Also drain any late data.
                loop {
                    futures::select! {
                        cmd = commands.next() => {
                            match cmd {
                                Some(AcquisitionCommand::Start) => {
                                    info!("re-arming after read loop exit");
                                    match handle.start_streaming().await {
                                        Ok((rl, rx)) => {
                                            read_loop = rl.fuse();
                                            data_rx = rx.fuse();
                                            break; // back to streaming select!
                                        }
                                        Err(e) => {
                                            handle.mark_error(e.to_string());
                                            return handle;
                                        }
                                    }
                                }
                                Some(cmd) => {
                                    if let Err(e) = handle.apply(cmd).await {
                                        handle.mark_error(e.to_string());
                                    }
                                }
                                None => return handle,
                            }
                        }
                        chunk = data_rx.next() => {
                            if let Some(chunk) = chunk {
                                handle.ingest_chunk(&chunk);
                                if let Some(ref tx) = gui_tx {
                                    let _ = tx.unbounded_send(chunk);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Native acquisition spawner: a current-thread tokio task running the
/// continuous acquisition loop.
///
/// The device capability futures are `?Send`, so they cannot ride
/// `tokio::spawn`; the spawner uses [`tokio::task::spawn_local`] and must run
/// inside a [`tokio::task::LocalSet`]. A panicking source is isolated at the task
/// boundary: the process survives and [`AcquisitionController::finish`] reports
/// [`SessionError::AcquisitionPanicked`](crate::SessionError::AcquisitionPanicked).
#[cfg(feature = "native")]
pub mod native {
    use futures::channel::mpsc;

    use super::run_acquisition;
    use crate::error::SessionError;
    use crate::handle::{AcquisitionCommand, DeviceHandle};
    use rb_model::SampleChunk;

    /// Handle to a spawned acquisition task: send commands, poll acquired data,
    /// then `finish` to stop and recover the device.
    pub struct AcquisitionController {
        commands: mpsc::UnboundedSender<AcquisitionCommand>,
        data: mpsc::UnboundedReceiver<SampleChunk>,
        join: tokio::task::JoinHandle<DeviceHandle>,
    }

    impl AcquisitionController {
        /// Spawns an acquisition task. Must be called inside a
        /// [`tokio::task::LocalSet`].
        #[must_use]
        pub fn spawn(handle: DeviceHandle) -> Self {
            let (commands, command_rx) = mpsc::unbounded();
            let (data_tx, data_rx) = mpsc::unbounded();
            let join = tokio::task::spawn_local(run_acquisition(handle, command_rx, Some(data_tx)));
            Self {
                commands,
                data: data_rx,
                join,
            }
        }

        /// Sends a control command to the running task.
        ///
        /// # Errors
        /// Returns [`SessionError::TaskClosed`] if the task has already finished.
        pub fn send(&self, command: AcquisitionCommand) -> Result<(), SessionError> {
            self.commands
                .unbounded_send(command)
                .map_err(|_| SessionError::TaskClosed)
        }

        /// Drains any [`SampleChunk`]s that the task has produced since the last
        /// call, in acquisition order.
        pub fn drain_data(&mut self) -> Vec<SampleChunk> {
            let mut chunks = Vec::new();
            while let Ok(chunk) = self.data.try_recv() {
                chunks.push(chunk);
            }
            chunks
        }

        /// Stops the task (by closing the command channel) and recovers the
        /// device handle with its filled stores.
        ///
        /// # Errors
        /// Returns [`SessionError::AcquisitionPanicked`] if the task panicked, or
        /// [`SessionError::TaskClosed`] if it was cancelled.
        pub async fn finish(self) -> Result<DeviceHandle, SessionError> {
            drop(self.commands);
            drop(self.data);
            self.join.await.map_err(|error| {
                if error.is_panic() {
                    SessionError::AcquisitionPanicked
                } else {
                    SessionError::TaskClosed
                }
            })
        }
    }
}

/// Web acquisition spawner: runs the continuous loop on `wasm-bindgen-futures`.
///
/// The returned sender drives the detached task, and the returned receiver
/// delivers acquired [`SampleChunk`]s.
#[cfg(feature = "web")]
pub mod web {
    use futures::channel::mpsc;

    use super::run_acquisition;
    use crate::handle::{AcquisitionCommand, DeviceHandle};
    use rb_model::SampleChunk;

    /// Handle for a web-spawned acquisition: command sender and data receiver.
    pub struct WebAcquisitionHandle {
        pub commands: mpsc::UnboundedSender<AcquisitionCommand>,
        pub data: mpsc::UnboundedReceiver<SampleChunk>,
    }

    /// Spawns the acquisition loop as a local (non-`Send`) task and returns a
    /// [`WebAcquisitionHandle`]. Dropping the command sender stops the task.
    pub fn spawn_local(handle: DeviceHandle) -> WebAcquisitionHandle {
        let (commands, command_rx) = mpsc::unbounded();
        let (data_tx, data_rx) = mpsc::unbounded();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = run_acquisition(handle, command_rx, Some(data_tx)).await;
        });
        WebAcquisitionHandle {
            commands,
            data: data_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::{AcquisitionState, DeviceHandle};
    use futures::channel::mpsc;
    use futures::executor::block_on;
    use rb_device::DeviceId;
    use rb_drivers::demo::{DemoConfig, DemoDevice};
    use std::time::Duration;

    fn demo_handle() -> DeviceHandle {
        let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        DeviceHandle::new(Box::new(device))
    }

    /// Builds a single-threaded tokio runtime (required by `LocalSet`).
    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    #[test]
    fn streaming_after_start_command() {
        let rt = rt();
        let handle = demo_handle();
        let (commands, command_rx) = mpsc::unbounded();

        let local = tokio::task::LocalSet::new();
        let (done_tx, done_rx) = futures::channel::oneshot::channel();

        local.spawn_local(async move {
            let h = run_acquisition(handle, command_rx, None).await;
            let _ = done_tx.send(h);
        });

        // Send Start after spawning so the loop is already polling.
        commands.unbounded_send(AcquisitionCommand::Start).unwrap();

        // Close the channel after a delay so samples accumulate.
        let closer = commands.clone();
        local.spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            drop(closer);
        });
        drop(commands);

        let handle = rt
            .block_on(local.run_until(done_rx))
            .expect("task panicked");

        assert!(handle.sample_count() > 0, "should have streamed samples");
    }

    #[test]
    fn closing_the_command_channel_ends_the_loop() {
        let handle = demo_handle();
        let (commands, command_rx) = mpsc::unbounded::<AcquisitionCommand>();
        drop(commands);

        let handle = block_on(run_acquisition(handle, command_rx, None));
        assert_eq!(handle.sample_count(), 0);
    }

    #[test]
    fn stop_command_halts_streaming() {
        let rt = rt();
        let handle = demo_handle();
        let (commands, command_rx) = mpsc::unbounded();

        let local = tokio::task::LocalSet::new();
        let (done_tx, done_rx) = futures::channel::oneshot::channel();

        local.spawn_local(async move {
            let h = run_acquisition(handle, command_rx, None).await;
            let _ = done_tx.send(h);
        });

        // Send Start.
        commands.unbounded_send(AcquisitionCommand::Start).unwrap();

        // Send Stop after 100 ms.
        let c = commands.clone();
        local.spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = c.unbounded_send(AcquisitionCommand::Stop);
        });

        // Close channel after 300 ms to end the loop.
        let c = commands.clone();
        local.spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            drop(c);
        });
        drop(commands);

        let handle = rt
            .block_on(local.run_until(done_rx))
            .expect("task panicked");

        assert_eq!(handle.state(), &AcquisitionState::Stopped);
        assert!(
            handle.sample_count() > 0,
            "should have samples from before stop"
        );
    }
}
