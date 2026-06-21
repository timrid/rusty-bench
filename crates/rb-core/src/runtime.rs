//! Runtime-agnostic acquisition glue.
//!
//! [`run_acquisition`] is the heart of the acquisition loop and binds **no**
//! runtime: it consumes a stream of [`AcquisitionCommand`]s and a stream of
//! ticks, applying commands and pumping the device's stores on every tick. It
//! only uses `futures`, so it is wasm-safe and can be tested with a plain
//! executor.
//!
//! The runtime-specific *spawners* live in the [`native`] (tokio) and [`web`]
//! (`wasm-bindgen-futures`) submodules, each enabled by its own Cargo feature.
//! Both just provide a tick source and a place to run the same loop.

use futures::FutureExt;
use futures::select_biased;
use futures::stream::{Stream, StreamExt};

use crate::handle::{AcquisitionCommand, DeviceHandle};

/// Drives a device's acquisition: applies commands and pumps on every tick.
///
/// Commands take priority over ticks, so a `Start` queued before the first tick
/// is always honoured first. The loop ends — returning the (owned) handle — when
/// either stream finishes: dropping the command sender stops and collects the
/// device, and a finite tick stream models a bounded capture.
///
/// A command that fails does not abort the loop; the device is moved to the
/// [`Error`](crate::AcquisitionState::Error) state and acquisition continues for
/// the rest of the session.
pub async fn run_acquisition<C, T>(
    mut handle: DeviceHandle,
    mut commands: C,
    mut ticks: T,
    chunk_samples: usize,
) -> DeviceHandle
where
    C: Stream<Item = AcquisitionCommand> + Unpin,
    T: Stream<Item = ()> + Unpin,
{
    loop {
        select_biased! {
            command = commands.next().fuse() => match command {
                Some(command) => {
                    if let Err(error) = handle.apply(command).await {
                        handle.mark_error(error.to_string());
                    }
                }
                None => break,
            },
            tick = ticks.next().fuse() => match tick {
                Some(()) => {
                    handle.pump(chunk_samples).await;
                }
                None => break,
            },
        }
    }
    handle
}

/// Native acquisition spawner: a current-thread tokio task plus interval ticks.
///
/// The device capability futures are `?Send`, so they cannot ride
/// `tokio::spawn`; the spawner uses [`tokio::task::spawn_local`] and must run
/// inside a [`tokio::task::LocalSet`]. A panicking source is isolated at the task
/// boundary: the process survives and [`AcquisitionController::finish`] reports
/// [`SessionError::AcquisitionPanicked`](crate::SessionError::AcquisitionPanicked).
#[cfg(feature = "native")]
pub mod native {
    use core::time::Duration;

    use futures::channel::mpsc;
    use futures::stream::Stream;

    use super::run_acquisition;
    use crate::error::SessionError;
    use crate::handle::{AcquisitionCommand, DeviceHandle};

    /// A stream that yields `()` every `period`, driven by `tokio::time::sleep`.
    pub fn interval_ticks(period: Duration) -> impl Stream<Item = ()> {
        futures::stream::unfold((), move |()| async move {
            tokio::time::sleep(period).await;
            Some(((), ()))
        })
    }

    /// Handle to a spawned acquisition task: send commands, then `finish` to stop
    /// and recover the device.
    pub struct AcquisitionController {
        commands: mpsc::UnboundedSender<AcquisitionCommand>,
        join: tokio::task::JoinHandle<DeviceHandle>,
    }

    impl AcquisitionController {
        /// Spawns an acquisition task pumping `chunk_samples` every `period`.
        /// Must be called inside a [`tokio::task::LocalSet`].
        #[must_use]
        pub fn spawn(handle: DeviceHandle, period: Duration, chunk_samples: usize) -> Self {
            let (commands, command_rx) = mpsc::unbounded();
            let ticks = Box::pin(interval_ticks(period));
            let join =
                tokio::task::spawn_local(run_acquisition(handle, command_rx, ticks, chunk_samples));
            Self { commands, join }
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

        /// Stops the task (by closing the command channel) and recovers the
        /// device handle with its filled stores.
        ///
        /// # Errors
        /// Returns [`SessionError::AcquisitionPanicked`] if the task panicked, or
        /// [`SessionError::TaskClosed`] if it was cancelled.
        pub async fn finish(self) -> Result<DeviceHandle, SessionError> {
            drop(self.commands);
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

/// Web acquisition spawner: runs the loop on `wasm-bindgen-futures`.
///
/// The browser supplies the tick source (e.g. `requestAnimationFrame` or a
/// timer, wired up by the GUI in a later milestone), so the spawner is generic
/// over the tick stream. The returned sender drives the detached task.
#[cfg(feature = "web")]
pub mod web {
    use futures::channel::mpsc;
    use futures::stream::Stream;

    use super::run_acquisition;
    use crate::handle::{AcquisitionCommand, DeviceHandle};

    /// Spawns the acquisition loop as a local (non-`Send`) task and returns a
    /// command sender. Dropping the sender stops the task.
    pub fn spawn_local<T>(
        handle: DeviceHandle,
        ticks: T,
        chunk_samples: usize,
    ) -> mpsc::UnboundedSender<AcquisitionCommand>
    where
        T: Stream<Item = ()> + Unpin + 'static,
    {
        let (commands, command_rx) = mpsc::unbounded();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = run_acquisition(handle, command_rx, ticks, chunk_samples).await;
        });
        commands
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

    fn demo_handle() -> DeviceHandle {
        let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        DeviceHandle::new(Box::new(device))
    }

    #[test]
    fn ticks_pump_after_a_start_command() {
        let handle = demo_handle();
        let (commands, command_rx) = mpsc::unbounded();
        commands.unbounded_send(AcquisitionCommand::Start).unwrap();
        let ticks = futures::stream::iter(core::iter::repeat_n((), 4));

        let handle = block_on(run_acquisition(handle, command_rx, ticks, 16));

        assert_eq!(handle.state(), &AcquisitionState::Running);
        assert_eq!(handle.sample_count(), 64);
        drop(commands);
    }

    #[test]
    fn closing_the_command_channel_ends_the_loop() {
        let handle = demo_handle();
        let (commands, command_rx) = mpsc::unbounded::<AcquisitionCommand>();
        drop(commands);
        // Infinite ticks: the loop must still end because commands closed.
        let ticks = futures::stream::repeat(());

        let handle = block_on(run_acquisition(handle, command_rx, ticks, 16));
        assert_eq!(handle.sample_count(), 0);
    }
}
