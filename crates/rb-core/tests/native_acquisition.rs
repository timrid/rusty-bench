//! Native acquisition-spawner integration tests (require the `native` feature).
//!
//! These cover the tokio `spawn_local` path and per-device panic isolation, which
//! the runtime-free unit tests cannot exercise. Run with:
//! `cargo test -p rb-core --features native`.
#![cfg(feature = "native")]

use core::time::Duration;

use async_trait::async_trait;

use rb_core::runtime::native::AcquisitionController;
use rb_core::{AcquisitionCommand, AcquisitionState, DeviceHandle, Session};
use rb_device::{AcquisitionSource, Device, DeviceId, DeviceInfo, DeviceResult, Oscilloscope};
use rb_drivers::demo::{DemoConfig, DemoDevice};
use rb_model::{AnalogChannel, SampleChunk};

#[tokio::test]
async fn spawned_task_acquires_via_command_channel() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
            let handle = DeviceHandle::new(Box::new(device));

            let controller = AcquisitionController::spawn(handle, Duration::from_millis(1), 64);
            controller.send(AcquisitionCommand::Start).unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;

            let handle = controller.finish().await.unwrap();
            assert_eq!(handle.state(), &AcquisitionState::Running);
            assert!(handle.sample_count() > 0, "expected the store to fill");
        })
        .await;
}

/// A source that panics on first pull, to prove task-boundary isolation.
struct PanicSource;

impl AcquisitionSource for PanicSource {
    fn next_chunk(&mut self, _max_samples: usize) -> SampleChunk {
        panic!("synthetic driver fault");
    }
}

/// A minimal device that arms cleanly but whose source panics when pumped.
struct PanicDevice {
    id: DeviceId,
    info: DeviceInfo,
    channels: Vec<AnalogChannel>,
    source: PanicSource,
}

impl PanicDevice {
    fn new() -> Self {
        Self {
            id: DeviceId::new("panic:0"),
            info: DeviceInfo::new("Test", "Panic"),
            channels: Vec::new(),
            source: PanicSource,
        }
    }
}

#[async_trait(?Send)]
impl Oscilloscope for PanicDevice {
    fn channels(&self) -> &[AnalogChannel] {
        &self.channels
    }
    fn sample_rate_hz(&self) -> f64 {
        1.0
    }
    async fn set_sample_rate_hz(&mut self, _hz: f64) -> DeviceResult<()> {
        Ok(())
    }
    async fn arm(&mut self) -> DeviceResult<()> {
        Ok(())
    }
    async fn stop(&mut self) -> DeviceResult<()> {
        Ok(())
    }
}

#[async_trait(?Send)]
impl Device for PanicDevice {
    fn id(&self) -> &DeviceId {
        &self.id
    }
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
    fn as_oscilloscope(&self) -> Option<&dyn Oscilloscope> {
        Some(self)
    }
    fn as_oscilloscope_mut(&mut self) -> Option<&mut dyn Oscilloscope> {
        Some(self)
    }
    fn as_acquisition_source_mut(&mut self) -> Option<&mut dyn AcquisitionSource> {
        Some(&mut self.source)
    }
}

#[tokio::test]
async fn a_panicking_device_is_isolated_from_the_session() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let handle = DeviceHandle::new(Box::new(PanicDevice::new()));
            let controller = AcquisitionController::spawn(handle, Duration::from_millis(1), 32);
            controller.send(AcquisitionCommand::Start).unwrap();
            tokio::time::sleep(Duration::from_millis(15)).await;

            // The task panicked, but the process survived and the error is surfaced.
            let outcome = controller.finish().await;
            assert!(matches!(
                outcome,
                Err(rb_core::SessionError::AcquisitionPanicked)
            ));

            // The session is still fully usable: a fresh device works.
            let mut session = Session::new();
            let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
            let id = session.add_device(Box::new(device));
            session.device_mut(&id).unwrap().start().await.unwrap();
            assert_eq!(session.pump_all(16), 16);
        })
        .await;
}
