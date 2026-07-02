//! [`DeviceHandle`]: one connected device plus its stores and acquisition state.

use std::future::Future;
use std::pin::Pin;

use futures::channel::mpsc;

use rb_device::{Device, DeviceId};
use rb_model::{AnalogTrace, DigitalTrace, SampleChunk, Timebase};

use crate::error::SessionError;

/// Where a device is in its acquisition lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquisitionState {
    /// Connected but not yet armed.
    Idle,
    /// Armed and streaming samples into its stores.
    Running,
    /// Acquisition was stopped cleanly.
    Stopped,
    /// Acquisition ended in error (e.g. a driver fault or a caught panic). The
    /// device is isolated; the rest of the session is unaffected.
    Error(String),
}

/// A control message for a device's acquisition, delivered over a channel by the
/// runtime glue (or applied directly in tests).
#[derive(Clone, Debug, PartialEq)]
pub enum AcquisitionCommand {
    /// Arm the device and begin filling its stores.
    Start,
    /// Stop an in-progress acquisition.
    Stop,
    /// Change the acquisition sample rate, in hertz.
    SetSampleRate(f64),
}

/// One connected device together with the per-channel stores its samples flow
/// into and its current [`AcquisitionState`].
///
/// The handle owns the device and its stores.  Control commands flow through
/// [`apply`](Self::apply).  The push-based bulk path works through
/// [`start_streaming`](Self::start_streaming) → read-loop future (spawned by the
/// runtime) → [`ingest_chunk`](Self::ingest_chunk) fed from the data channel.
pub struct DeviceHandle {
    id: DeviceId,
    device: Box<dyn Device>,
    analog: Vec<AnalogTrace>,
    digital: Option<DigitalTrace>,
    state: AcquisitionState,
}

impl DeviceHandle {
    /// Wraps a connected device, building one [`AnalogTrace`] per oscilloscope
    /// channel and a [`DigitalTrace`] for the logic group, sized from the
    /// device's advertised capabilities.
    #[must_use]
    pub fn new(device: Box<dyn Device>) -> Self {
        let id = device.id().clone();
        let (analog, digital) = Self::build_traces(device.as_ref());
        Self {
            id,
            device,
            analog,
            digital,
            state: AcquisitionState::Idle,
        }
    }

    fn build_traces(device: &dyn Device) -> (Vec<AnalogTrace>, Option<DigitalTrace>) {
        let analog = device
            .as_oscilloscope()
            .map(|scope| {
                let timebase = Timebase::new(positive_rate(scope.sample_rate_hz()), 0.0);
                scope
                    .channels()
                    .iter()
                    .map(|channel| AnalogTrace::new(channel.clone(), timebase))
                    .collect()
            })
            .unwrap_or_default();

        let digital = device
            .as_logic_analyzer()
            .filter(|la| !la.channels().is_empty())
            .map(|la| {
                let timebase = Timebase::new(positive_rate(la.sample_rate_hz()), 0.0);
                DigitalTrace::new(la.channels().to_vec(), timebase)
            });

        (analog, digital)
    }

    fn rebuild_traces(&mut self) {
        let (analog, digital) = Self::build_traces(self.device.as_ref());
        self.analog = analog;
        self.digital = digital;
    }

    /// Replaces the internal traces (for matching an [`AcquisitionConfig`]).
    pub fn set_traces(&mut self, analog: Vec<AnalogTrace>, digital: Option<DigitalTrace>) {
        self.analog = analog;
        self.digital = digital;
    }

    /// Discards all acquired samples by recreating traces with the same
    /// channels and timebase. Called at the start of every acquisition so
    /// that Stop → Start begins a fresh capture.
    pub fn discard_samples(&mut self) {
        for trace in &mut self.analog {
            let tb = *trace.timebase();
            let ch = trace.channel().clone();
            *trace = AnalogTrace::new(ch, tb);
        }
        if let Some(digital) = &mut self.digital {
            let tb = *digital.timebase();
            let channels = digital.channels().to_vec();
            *digital = DigitalTrace::new(channels, tb);
        }
    }

    /// This device's stable identifier.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Read-only access to the wrapped device (identity, capabilities).
    #[must_use]
    pub fn device(&self) -> &dyn Device {
        self.device.as_ref()
    }

    /// The current acquisition state.
    #[must_use]
    pub fn state(&self) -> &AcquisitionState {
        &self.state
    }

    /// The analog traces, one per oscilloscope channel in channel order.
    #[must_use]
    pub fn analog_traces(&self) -> &[AnalogTrace] {
        &self.analog
    }

    /// The logic trace, if the device has digital channels.
    pub fn digital_trace(&self) -> Option<&DigitalTrace> {
        self.digital.as_ref()
    }

    /// Number of samples acquired so far (all present series share this count).
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.analog
            .first()
            .map(AnalogTrace::len)
            .or_else(|| self.digital.as_ref().map(DigitalTrace::len))
            .unwrap_or(0)
    }

    /// Applies a control [`AcquisitionCommand`].
    ///
    /// # Errors
    /// Returns a [`SessionError`] if the device rejects the command or exposes no
    /// acquirable capability.
    pub async fn apply(&mut self, command: AcquisitionCommand) -> Result<(), SessionError> {
        match command {
            AcquisitionCommand::Start => self.arm().await,
            AcquisitionCommand::Stop => self.stop().await,
            AcquisitionCommand::SetSampleRate(hz) => self.set_sample_rate_hz(hz).await,
        }
    }

    /// Arms every acquirable capability and enters [`Running`](AcquisitionState::Running).
    ///
    /// Clears any previously acquired trace data so that re-arm (Stop → Start)
    /// begins a fresh capture rather than appending to the prior run.
    ///
    /// Does NOT start the bulk data streaming — call [`start_streaming`] after
    /// this to begin the push-based data flow.
    ///
    /// # Errors
    /// Returns [`SessionError::NotAcquirable`] if the device streams no samples,
    /// or a [`SessionError::Device`] if arming fails.
    pub async fn arm(&mut self) -> Result<(), SessionError> {
        self.discard_samples();
        let mut armed_any = false;
        if let Some(scope) = self.device.as_oscilloscope_mut() {
            scope.arm().await?;
            armed_any = true;
        }
        if let Some(la) = self.device.as_logic_analyzer_mut() {
            la.arm().await?;
            armed_any = true;
        }
        if !armed_any {
            return Err(SessionError::NotAcquirable);
        }
        self.state = AcquisitionState::Running;
        Ok(())
    }

    /// Stops every acquirable capability and enters [`Stopped`](AcquisitionState::Stopped).
    ///
    /// # Errors
    /// Returns a [`SessionError::Device`] if the device reports a fault while
    /// stopping.
    pub async fn stop(&mut self) -> Result<(), SessionError> {
        if let Some(source) = self.device.as_acquisition_source_mut() {
            source.stop_streaming().await?;
        }
        if let Some(scope) = self.device.as_oscilloscope_mut() {
            scope.stop().await?;
        }
        if let Some(la) = self.device.as_logic_analyzer_mut() {
            la.stop().await?;
        }
        self.state = AcquisitionState::Stopped;
        Ok(())
    }

    /// Arms the device AND starts push-based streaming.
    ///
    /// Returns a read-loop future plus a receiver for the [`SampleChunk`]s.
    ///
    /// # Errors
    /// Returns [`SessionError::NotAcquirable`] if the device has no bulk source.
    pub async fn start_streaming(
        &mut self,
    ) -> Result<
        (
            Pin<Box<dyn Future<Output = ()>>>,
            mpsc::UnboundedReceiver<SampleChunk>,
        ),
        SessionError,
    > {
        self.arm().await?;

        let (data_tx, data_rx) = mpsc::unbounded();
        let read_loop = self
            .device
            .as_acquisition_source_mut()
            .ok_or(SessionError::NotAcquirable)?
            .start_streaming(data_tx)
            .await
            .map_err(SessionError::Device)?;

        Ok((read_loop, data_rx))
    }

    /// Feeds a [`SampleChunk`] into the per-channel stores.
    ///
    /// Called by the runtime for each chunk received from the streaming source.
    /// Returns the number of samples appended.
    pub fn ingest_chunk(&mut self, chunk: &SampleChunk) -> usize {
        let count = chunk.sample_count();
        for (index, trace) in self.analog.iter_mut().enumerate() {
            if let Some(samples) = chunk.analog_channel(index) {
                trace.push_raw(samples);
            }
        }
        if let Some(digital) = self.digital.as_mut() {
            if !chunk.logic().is_empty() {
                digital.push_words(chunk.logic());
            }
        }
        count
    }

    /// Sets the acquisition sample rate and resizes the (empty) stores' timebase.
    ///
    /// # Errors
    /// Returns a [`SessionError::Device`] if the device rejects the rate.
    pub async fn set_sample_rate_hz(&mut self, hz: f64) -> Result<(), SessionError> {
        if let Some(scope) = self.device.as_oscilloscope_mut() {
            scope.set_sample_rate_hz(hz).await?;
        }
        if let Some(la) = self.device.as_logic_analyzer_mut() {
            la.set_sample_rate_hz(hz).await?;
        }
        self.rebuild_traces();
        Ok(())
    }

    /// Forces the device into the [`Error`](AcquisitionState::Error) state. Used
    /// by the runtime glue to surface a caught panic or fault.
    pub fn mark_error(&mut self, message: impl Into<String>) {
        self.state = AcquisitionState::Error(message.into());
    }
}

/// Clamps a possibly-zero/negative advertised rate to a strictly positive value
/// so the [`Timebase`] invariant holds even for a misbehaving device.
fn positive_rate(hz: f64) -> f64 {
    if hz.is_finite() && hz > 0.0 { hz } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::task::LocalSpawnExt;
    use rb_device::DeviceId;
    use rb_drivers::demo::{DemoConfig, DemoDevice};

    fn demo_handle() -> DeviceHandle {
        let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        DeviceHandle::new(Box::new(device))
    }

    #[test]
    fn new_handle_builds_traces_from_capabilities() {
        let handle = demo_handle();
        assert_eq!(handle.state(), &AcquisitionState::Idle);
        assert_eq!(handle.analog_traces().len(), 1);
        assert_eq!(handle.digital_trace().unwrap().channels().len(), 4);
        assert_eq!(handle.sample_count(), 0);
    }

    #[test]
    fn start_streaming_fills_stores() {
        let mut handle = demo_handle();
        let (read_loop, mut data_rx) = block_on(handle.start_streaming()).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Running);

        // Spawn the read loop on a LocalPool.
        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        // Read one chunk and ingest it.
        let chunk = pool.run_until(data_rx.next()).unwrap();
        handle.ingest_chunk(&chunk);
        assert!(handle.sample_count() >= 256);
        assert_eq!(handle.analog_traces()[0].len(), handle.sample_count());
        assert_eq!(handle.digital_trace().unwrap().len(), handle.sample_count());

        // Stop streaming.
        block_on(handle.apply(AcquisitionCommand::Stop)).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Stopped);

        drop(data_rx);
        pool.run_until_stalled();
    }

    #[test]
    fn stop_halts_acquisition() {
        let mut handle = demo_handle();
        let (read_loop, mut data_rx) = block_on(handle.start_streaming()).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(data_rx.next()).unwrap();
        handle.ingest_chunk(&chunk);
        let count_before = handle.sample_count();
        assert!(count_before > 0);

        block_on(handle.apply(AcquisitionCommand::Stop)).unwrap();
        assert_eq!(handle.state(), &AcquisitionState::Stopped);

        drop(data_rx);
        pool.run_until_stalled();

        // No new samples ingested after stop.
        assert_eq!(handle.sample_count(), count_before);
    }

    #[test]
    fn set_sample_rate_updates_the_timebase() {
        let mut handle = demo_handle();
        block_on(handle.apply(AcquisitionCommand::SetSampleRate(2_000_000.0))).unwrap();
        assert_eq!(
            handle.analog_traces()[0].timebase().sample_rate_hz(),
            2_000_000.0
        );
    }
}
