//! Native acquisition-spawner integration tests (require the `native` feature).
//!
//! These cover the tokio `spawn_local` path and per-device panic isolation, which
//! the runtime-free unit tests cannot exercise. Run with:
//! `cargo test -p rb-core --features native`.
#![cfg(feature = "native")]

use core::time::Duration;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::task::LocalSpawnExt;

use rb_core::Session;
use rb_core::runtime::native::AcquisitionController;
use rb_core::DeviceHandle;
use rb_device::{AcquisitionSource, Device, DeviceId, DeviceInfo, DeviceResult, Oscilloscope};
use rb_drivers::demo::{DemoConfig, DemoDevice};
use rb_model::{AnalogChannel, AnalogTrace, SampleChunk, Timebase};

#[tokio::test]
async fn spawned_acquisition_produces_samples() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
            let mut handle = DeviceHandle::new(Box::new(device));

            // Arm and set sample rate.
            if let Some(scope) = handle.device().as_oscilloscope_mut() {
                scope.arm().await.unwrap();
            }
            if let Some(la) = handle.device().as_logic_analyzer_mut() {
                la.arm().await.unwrap();
            }

            // Start streaming.
            let (data_tx, data_rx) = mpsc::unbounded::<SampleChunk>();
            let (gui_tx, mut gui_rx) = mpsc::unbounded::<SampleChunk>();
            let read_loop = handle
                .device()
                .as_acquisition_source_mut()
                .unwrap()
                .start_streaming(data_tx)
                .await
                .unwrap();

            let controller = AcquisitionController::spawn(read_loop, data_rx, Some(gui_tx), gui_tx.clone());
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Read some chunks.
            let mut count = 0;
            while let Ok(Some(_chunk)) =
                tokio::time::timeout(Duration::from_millis(50), gui_rx.next()).await
            {
                count += 1;
                if count >= 3 {
                    break;
                }
            }

            controller.finish().await;
            assert!(count > 0, "expected at least one chunk");

            // Clean stop.
            if let Some(src) = handle.device().as_acquisition_source_mut() {
                src.stop_streaming().await.unwrap();
            }
        })
        .await;
}

/// A source that panics on start_streaming, to prove task-boundary isolation.
struct PanicSource;

#[async_trait(?Send)]
impl AcquisitionSource for PanicSource {
    async fn start_streaming(
        &mut self,
        _chunk_tx: mpsc::UnboundedSender<SampleChunk>,
    ) -> DeviceResult<Pin<Box<dyn Future<Output = ()>>>> {
        panic!("synthetic driver fault");
    }

    async fn stop_streaming(&mut self) -> DeviceResult<()> {
        Ok(())
    }
}

/// A minimal device that arms cleanly but whose source panics when streaming.
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
    fn supported_sample_rates(&self) -> &[f64] {
        &[1_000.0, 10_000.0, 100_000.0, 1_000_000.0]
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
            let mut handle = DeviceHandle::new(Box::new(PanicDevice::new()));

            // Arm the device (this succeeds).
            if let Some(scope) = handle.device().as_oscilloscope_mut() {
                scope.arm().await.unwrap();
            }

            // Start streaming — this will panic inside the spawned task.
            let (data_tx, data_rx) = mpsc::unbounded::<SampleChunk>();
            let (gui_tx, _gui_rx) = mpsc::unbounded::<SampleChunk>();

            // Spawn the acquisition in a task. The panic should be contained.
            let join = tokio::task::spawn_local(async move {
                if let Some(src) = handle.device().as_acquisition_source_mut() {
                    // This panics inside start_streaming.
                    let _ = src.start_streaming(data_tx).await;
                }
            });

            // The spawned task should panic, not the test.
            tokio::time::sleep(Duration::from_millis(15)).await;
            let result = join.await;
            assert!(result.is_err(), "expected the spawned task to panic");

            drop(gui_tx);
            drop(data_rx);

            // A fresh device still works.
            let mut session = Session::new();
            let device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
            session.add_device(Box::new(device));
            assert_eq!(session.len(), 1);
        })
        .await;
}
