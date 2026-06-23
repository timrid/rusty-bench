//! Synthetic **Demo Device** driver.
//!
//! The demo device needs no hardware: it synthesises a sine wave on its analog
//! channel(s) and a free-running binary counter on its logic channels. It exists
//! to prove the whole pipeline — driver → [`AcquisitionSource`] → session store →
//! CLI/GUI — end to end, and to give every higher layer a device to test against.
//!
//! Signal definition (deterministic, so tests can assert exact values):
//! - analog channel `j`, sample `i`:
//!   `round(amplitude * sin(2π · f · i / rate + j · π/2))`
//! - logic word, sample `i`: `i & ((1 << digital_channels) - 1)` — i.e. `D0`
//!   toggles every sample, `D1` every two, and so on.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::channel::mpsc;

use rb_device::{
    AcquisitionSource, Device, DeviceClass, DeviceError, DeviceId, DeviceInfo, DeviceResult,
    LogicAnalyzer, Oscilloscope,
};
use rb_model::{AnalogChannel, AnalogFormat, ChannelId, DigitalChannel, SampleChunk};
use rb_transport::{DeviceCandidate, DriverError, DriverFactory, DriverResult};

/// Configuration of a synthetic [`DemoDevice`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DemoConfig {
    /// Acquisition sample rate, in hertz.
    pub sample_rate_hz: f64,
    /// Number of synthesised analog channels.
    pub analog_channels: usize,
    /// Number of synthesised digital channels (`0..=64`).
    pub digital_channels: u8,
    /// Frequency of the synthesised sine wave, in hertz.
    pub analog_frequency_hz: f64,
    /// Peak amplitude of the sine wave, in raw counts.
    pub analog_amplitude: i32,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 1_000_000.0,
            analog_channels: 1,
            digital_channels: 4,
            analog_frequency_hz: 1_000.0,
            analog_amplitude: 30_000,
        }
    }
}

/// The signal generator behind a [`DemoDevice`].
///
/// The `running` flag is shared with the read-loop future spawned by
/// [`start_streaming`](AcquisitionSource::start_streaming).
#[derive(Debug)]
pub struct DemoSource {
    config: DemoConfig,
    produced: u64,
    running: Arc<AtomicBool>,
}

impl DemoSource {
    /// Creates a source for the given configuration, positioned at sample `0`.
    #[must_use]
    pub fn new(config: DemoConfig) -> Self {
        Self {
            config,
            produced: 0,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Number of samples generated so far.
    #[must_use]
    pub fn produced(&self) -> u64 {
        self.produced
    }

    /// Whether the source is currently streaming.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[async_trait(?Send)]
impl AcquisitionSource for DemoSource {
    async fn start_streaming(
        &mut self,
        chunk_tx: mpsc::UnboundedSender<SampleChunk>,
    ) -> DeviceResult<Pin<Box<dyn Future<Output = ()>>>> {
        self.running.store(true, Ordering::SeqCst);

        let config = self.config;
        let produced = self.produced;
        let running = self.running.clone();
        const CHUNK_SIZE: usize = 256;

        let fut = async move {
            let mut state = ReadLoopState {
                config,
                produced,
                chunk_tx,
            };
            while running.load(Ordering::SeqCst) {
                let chunk = state.generate_chunk(CHUNK_SIZE);
                if state.chunk_tx.unbounded_send(chunk).is_err() {
                    break;
                }
                // Rate-limit so cooperative executors don't busy-loop on
                // pure-compute devices.  Real I/O devices suspend on I/O.
                futures_timer::Delay::new(core::time::Duration::from_millis(1)).await;
            }
            running.store(false, Ordering::SeqCst);
        };

        Ok(Box::pin(fut))
    }

    async fn stop_streaming(&mut self) -> DeviceResult<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Owned state for the demo read-loop future.
struct ReadLoopState {
    config: DemoConfig,
    produced: u64,
    chunk_tx: mpsc::UnboundedSender<SampleChunk>,
}

impl ReadLoopState {
    fn logic_mask(&self) -> u64 {
        match self.config.digital_channels {
            0 => 0,
            n if n >= 64 => u64::MAX,
            n => (1u64 << n) - 1,
        }
    }

    fn generate_chunk(&mut self, count: usize) -> SampleChunk {
        let cfg = self.config;
        let mask = self.logic_mask();
        let mut analog: Vec<Vec<i32>> = (0..cfg.analog_channels)
            .map(|_| Vec::with_capacity(count))
            .collect();
        let mut logic: Vec<u64> = if cfg.digital_channels > 0 {
            Vec::with_capacity(count)
        } else {
            Vec::new()
        };

        for s in 0..count {
            let idx = self.produced + s as u64;
            let t = idx as f64 / cfg.sample_rate_hz;
            for (j, analog_ch) in analog.iter_mut().enumerate() {
                let phase = j as f64 * core::f64::consts::FRAC_PI_2;
                let angle = core::f64::consts::TAU * cfg.analog_frequency_hz * t + phase;
                analog_ch.push((cfg.analog_amplitude as f64 * angle.sin()).round() as i32);
            }
            if cfg.digital_channels > 0 {
                logic.push(idx & mask);
            }
        }

        self.produced += count as u64;
        SampleChunk::new().with_analog(analog).with_logic(logic)
    }
}

/// A synthetic multi-class device: an [`Oscilloscope`] **and** a [`LogicAnalyzer`].
#[derive(Debug)]
pub struct DemoDevice {
    id: DeviceId,
    info: DeviceInfo,
    analog_channels: Vec<AnalogChannel>,
    digital_channels: Vec<DigitalChannel>,
    source: DemoSource,
}

impl DemoDevice {
    /// Builds a demo device with the given identifier and configuration.
    #[must_use]
    pub fn new(id: DeviceId, config: DemoConfig) -> Self {
        let scale = if config.analog_amplitude != 0 {
            1.0 / f64::from(config.analog_amplitude)
        } else {
            1.0
        };
        let analog_channels = (0..config.analog_channels)
            .map(|j| {
                AnalogChannel::new(
                    ChannelId(j as u32),
                    format!("A{j}"),
                    AnalogFormat::new(scale, 0.0),
                )
                .with_unit("V")
            })
            .collect();
        let digital_channels = (0..config.digital_channels)
            .map(|k| DigitalChannel::new(ChannelId(1000 + u32::from(k)), format!("D{k}"), k))
            .collect();
        Self {
            id,
            info: DeviceInfo::new("RustyBench", "Demo Device"),
            analog_channels,
            digital_channels,
            source: DemoSource::new(config),
        }
    }

    /// Whether the device is currently armed (producing samples).
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.source.is_running()
    }

    fn set_rate(&mut self, hz: f64) -> DeviceResult<()> {
        if !(hz.is_finite() && hz > 0.0) {
            return Err(DeviceError::InvalidParameter(format!(
                "sample rate must be finite and > 0, got {hz}"
            )));
        }
        self.source.config.sample_rate_hz = hz;
        Ok(())
    }
}

#[async_trait(?Send)]
impl Oscilloscope for DemoDevice {
    fn channels(&self) -> &[AnalogChannel] {
        &self.analog_channels
    }

    fn sample_rate_hz(&self) -> f64 {
        self.source.config.sample_rate_hz
    }

    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()> {
        self.set_rate(hz)
    }

    async fn arm(&mut self) -> DeviceResult<()> {
        self.source.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&mut self) -> DeviceResult<()> {
        self.source.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait(?Send)]
impl LogicAnalyzer for DemoDevice {
    fn channels(&self) -> &[DigitalChannel] {
        &self.digital_channels
    }

    fn sample_rate_hz(&self) -> f64 {
        self.source.config.sample_rate_hz
    }

    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()> {
        self.set_rate(hz)
    }

    async fn arm(&mut self) -> DeviceResult<()> {
        self.source.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&mut self) -> DeviceResult<()> {
        self.source.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait(?Send)]
impl Device for DemoDevice {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn as_logic_analyzer(&self) -> Option<&dyn LogicAnalyzer> {
        Some(self)
    }

    fn as_logic_analyzer_mut(&mut self) -> Option<&mut dyn LogicAnalyzer> {
        Some(self)
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

/// Discovers and connects [`DemoDevice`]s.
#[derive(Clone, Copy, Debug, Default)]
pub struct DemoFactory {
    config: DemoConfig,
}

impl DemoFactory {
    /// Creates a factory that hands out devices with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a factory that hands out devices with the given configuration.
    #[must_use]
    pub fn with_config(config: DemoConfig) -> Self {
        Self { config }
    }
}

const DEMO_CLASSES: &[DeviceClass] = &[DeviceClass::Oscilloscope, DeviceClass::LogicAnalyzer];

#[async_trait(?Send)]
impl DriverFactory for DemoFactory {
    fn name(&self) -> &str {
        "demo"
    }

    fn supported_classes(&self) -> &[DeviceClass] {
        DEMO_CLASSES
    }

    async fn scan(&self) -> DriverResult<Vec<DeviceCandidate>> {
        Ok(vec![DeviceCandidate::new(
            DeviceInfo::new("RustyBench", "Demo Device"),
            "demo:0",
        )])
    }

    async fn connect(&self, candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>> {
        if candidate.info.model != "Demo Device" {
            return Err(DriverError::NotFound);
        }
        Ok(Box::new(DemoDevice::new(
            DeviceId::new(candidate.address.clone()),
            self.config,
        )))
    }
}

/// The demo driver is synthetic (no USB hardware), so it contributes no
/// VID/PID pairs to the central WebUSB filter list.
#[must_use]
pub fn known_vid_pids() -> Vec<(u16, u16)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::task::LocalSpawnExt;

    #[test]
    fn source_generates_consistent_deterministic_chunks() {
        let mut source = DemoSource::new(DemoConfig::default());
        let (tx, mut rx) = mpsc::unbounded();

        let read_loop = block_on(source.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        // First chunk has 256 samples.
        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.sample_count(), 256);
        assert!(chunk.is_consistent());
        assert_eq!(chunk.analog_channel_count(), 1);
        // sin(0) == 0 at the very first sample.
        assert_eq!(chunk.analog_channel(0).unwrap()[0], 0);
        // D0..D3 counter: logic word == sample index & 0b1111.
        assert_eq!(chunk.logic()[0], 0);
        assert_eq!(chunk.logic()[1], 1);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn source_advances_position_across_chunks() {
        let mut source = DemoSource::new(DemoConfig::default());
        let (tx, mut rx) = mpsc::unbounded();

        let read_loop = block_on(source.start_streaming(tx)).unwrap();
        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let _ = pool.run_until(rx.next()).unwrap();
        let chunk2 = pool.run_until(rx.next()).unwrap();
        // Second chunk continues from sample 256.
        assert_eq!(chunk2.logic()[0], 256 & 0b1111);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn device_exposes_both_capabilities_and_a_source() {
        let mut device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        let mut classes = device.classes();
        classes.sort();
        assert_eq!(
            classes,
            vec![DeviceClass::LogicAnalyzer, DeviceClass::Oscilloscope]
        );
        assert_eq!(Oscilloscope::channels(&device).len(), 1);
        assert_eq!(LogicAnalyzer::channels(&device).len(), 4);
        assert!(device.as_acquisition_source_mut().is_some());
    }

    #[test]
    fn set_sample_rate_validates_and_applies() {
        let mut device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        assert!(block_on(Oscilloscope::set_sample_rate_hz(&mut device, 0.0)).is_err());
        assert!(block_on(Oscilloscope::set_sample_rate_hz(&mut device, -5.0)).is_err());
        block_on(Oscilloscope::set_sample_rate_hz(&mut device, 2_000_000.0)).unwrap();
        assert_eq!(Oscilloscope::sample_rate_hz(&device), 2_000_000.0);
        assert_eq!(LogicAnalyzer::sample_rate_hz(&device), 2_000_000.0);
    }

    #[test]
    fn arm_and_stop_toggle_armed_state() {
        let mut device = DemoDevice::new(DeviceId::new("demo:0"), DemoConfig::default());
        assert!(!device.is_armed());
        block_on(Oscilloscope::arm(&mut device)).unwrap();
        assert!(device.is_armed());
        block_on(LogicAnalyzer::stop(&mut device)).unwrap();
        assert!(!device.is_armed());
    }

    #[test]
    fn factory_scans_then_connects_a_live_device() {
        let factory = DemoFactory::new();
        let candidates = block_on(factory.scan()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].info.model, "Demo Device");

        let device = block_on(factory.connect(&candidates[0])).unwrap();
        assert_eq!(device.id().as_str(), "demo:0");
        assert!(device.as_oscilloscope().is_some());
        assert!(device.as_logic_analyzer().is_some());
    }

    #[test]
    fn factory_rejects_foreign_candidate() {
        let factory = DemoFactory::new();
        let foreign = DeviceCandidate::new(DeviceInfo::new("Acme", "Other"), "x:1");
        assert!(matches!(
            block_on(factory.connect(&foreign)),
            Err(DriverError::NotFound)
        ));
    }
}
