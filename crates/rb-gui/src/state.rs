//! Platform-neutral RustyBench application state.
//!
//! [`AppState`] is framework-agnostic: it has no dependency on egui, Dioxus, or
//! any other GUI toolkit.  All device lifecycle, acquisition spawning, pending
//! actions, and executor management lives here so it can be tested without a
//! display and reused by any GUI front-end.

use std::collections::HashMap;

use futures::channel::mpsc;
use rb_core::{
    run_acquisition, AcquisitionCommand, AcquisitionState, DeviceHandle, DriverRegistry, ScanResult,
    Session,
};
use rb_device::DeviceId;
use rb_model::{AnalogChannel, AnalogTrace, DigitalChannel, DigitalTrace, SampleChunk, Timebase};

use crate::waveform_state::WaveformView;

// Executors: tokio LocalSet when native feature is enabled (non-wasm),
// LocalPool otherwise (non-wasm tests only; WASM uses wasm-bindgen-futures).
#[cfg(not(any(feature = "native", target_arch = "wasm32")))]
use {futures::executor::LocalPool, futures::executor::LocalSpawner, futures::task::LocalSpawnExt};
#[cfg(feature = "native")]
use tokio::task::LocalSet;

// ── Acquisition configuration ─────────────────────────────────────────────────

/// User-facing configuration for the next acquisition run.
///
/// Persists across Stop → Start cycles so that channel selection and sample
/// rate survive a re-run.  Built once at [`spawn_acquisition`] time from the
/// device's channel list (all channels enabled, device's native sample rate).
#[derive(Clone, Debug)]
pub struct AcquisitionConfig {
    /// Desired sample rate in Hz.  Sent via [`SetSampleRate`](AcquisitionCommand::SetSampleRate)
    /// before every [`Start`](AcquisitionCommand::Start).
    pub sample_rate_hz: f64,
    /// Per-channel enable flags, in device channel order.  A disabled channel
    /// is still present in [`DeviceAcquisition::analog`] (so labels render) but
    /// [`drain`](DeviceAcquisition::drain) skips pushing samples into it.
    pub analog_enabled: Vec<bool>,
    /// Per-digital-channel enable flags, in device channel order.
    pub digital_enabled: Vec<bool>,
}

// ── Per-device acquisition state ──────────────────────────────────────────────

/// Bundles a background acquisition future with the local display stores.
///
/// The acquisition future runs continuously on the platform's native spawner
/// (tokio `LocalSet` for native, `wasm-bindgen-futures` for web). Data flows
/// back through `data_rx` into the local stores.
pub struct DeviceAcquisition {
    pub analog: Vec<AnalogTrace>,
    pub digital: Option<DigitalTrace>,
    pub state: AcquisitionState,
    pub sample_count: usize,
    /// Sends [`AcquisitionCommand`]s to the spawned future.
    pub cmd_tx: mpsc::UnboundedSender<AcquisitionCommand>,
    /// Receives [`SampleChunk`]s from the spawned future.
    pub data_rx: mpsc::UnboundedReceiver<SampleChunk>,
    /// User-editable acquisition configuration.
    pub config: AcquisitionConfig,
}

impl DeviceAcquisition {
    pub fn drain(&mut self) {
        let chunks: Vec<SampleChunk> =
            std::iter::from_fn(|| self.data_rx.try_recv().ok()).collect();
        for chunk in &chunks {
            for (index, trace) in self.analog.iter_mut().enumerate() {
                if !self.config.analog_enabled.get(index).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(samples) = chunk.analog_channel(index) {
                    trace.push_raw(samples);
                }
            }
            if let Some(ref mut digital) = self.digital {
                if self.config.digital_enabled.iter().any(|e| *e) && !chunk.logic().is_empty() {
                    digital.push_words(chunk.logic());
                }
            }
            self.sample_count += chunk.sample_count();
        }
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }
    pub fn analog_traces(&self) -> &[AnalogTrace] {
        &self.analog
    }
    pub fn digital_trace(&self) -> Option<&DigitalTrace> {
        self.digital.as_ref()
    }
    pub fn state(&self) -> &AcquisitionState {
        &self.state
    }

    pub fn send_command(&self, cmd: AcquisitionCommand) {
        let _ = self.cmd_tx.unbounded_send(cmd);
    }

    /// Resets all traces to empty, preserving channel configuration.
    /// Called on re-run so old data is discarded before a fresh acquisition.
    pub fn reset_traces(&mut self) {
        // Read channel metadata from current traces BEFORE replacing them.
        let analog_channels: Vec<AnalogChannel> =
            self.analog.iter().map(|t| t.channel().clone()).collect();
        let digital_channels: Option<Vec<DigitalChannel>> =
            self.digital.as_ref().map(|t| t.channels().to_vec());
        let rate = self.config.sample_rate_hz;
        let timebase = Timebase::new(rate, 0.0);
        self.analog = analog_channels
            .iter()
            .map(|ch| AnalogTrace::new(ch.clone(), timebase))
            .collect();
        self.digital = digital_channels
            .as_ref()
            .map(|chs| DigitalTrace::new(chs.clone(), timebase));
        self.sample_count = 0;
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    pub session: Session,
    pub registry: DriverRegistry,
    pub scan_results: Vec<ScanResult>,
    pub scan_error: Option<String>,
    pub connect_error: Option<String>,
    pub selected_device: Option<DeviceId>,
    pub views: HashMap<DeviceId, WaveformView>,
    pub acquisitions: HashMap<DeviceId, DeviceAcquisition>,
    /// Executor for background acquisition futures (non-WASM, non-native tests).
    #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
    #[allow(dead_code)]
    pub pool: LocalPool,
    #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
    #[allow(dead_code)]
    pub spawner: LocalSpawner,
    /// Tokio LocalSet for native builds (proper I/O integration).
    /// Uses the existing Dioxus tokio runtime; no nested Runtime created.
    #[cfg(feature = "native")]
    pub local_set: LocalSet,
    // Deferred actions.
    pub pending_connect: Option<ScanResult>,
    pub pending_disconnect: Option<DeviceId>,
    pub pending_start: Option<DeviceId>,
    pub pending_stop: Option<DeviceId>,
    /// Receiver for a pending WASM scan (spawned via `spawn_local`).
    #[cfg(target_arch = "wasm32")]
    pub pending_wasm_scan: Option<futures::channel::oneshot::Receiver<Result<Vec<ScanResult>, String>>>,
    /// Receiver for a pending WASM connect (spawned via `spawn_local`).
    #[cfg(target_arch = "wasm32")]
    pub pending_wasm_connect: Option<futures::channel::oneshot::Receiver<Result<Box<dyn rb_device::Device>, String>>>,
}

impl AppState {
    /// Creates an app state with a pre-populated [`Session`], for integration tests
    /// that need to inject a mock device without going through the driver
    /// registry.
    ///
    /// The session's devices appear as connected in the UI and are ready for
    /// acquisition — no scan or connect step needed.
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
        let (pool, spawner) = {
            let p = LocalPool::new();
            let s = p.spawner();
            (p, s)
        };
        #[cfg(feature = "native")]
        let local_set = LocalSet::new();
        let ids: Vec<DeviceId> = session.device_ids();
        let selected_device = ids.into_iter().next();
        Self {
            session,
            registry: DriverRegistry::with_default_factories(),
            scan_results: Vec::new(),
            scan_error: None,
            connect_error: None,
            selected_device,
            views: HashMap::new(),
            acquisitions: HashMap::new(),
            #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
            pool,
            #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
            spawner,
            #[cfg(feature = "native")]
            local_set,
            pending_connect: None,
            pending_disconnect: None,
            pending_start: None,
            pending_stop: None,
            #[cfg(target_arch = "wasm32")]
            pending_wasm_scan: None,
            #[cfg(target_arch = "wasm32")]
            pending_wasm_connect: None,
        }
    }

    pub fn new() -> Self {
        Self::from_session(Session::new())
    }

    /// Connect to a device candidate (blocking, for use in event handlers).
    /// On native, wraps in block_in_place to work within Dioxus's tokio context.
    pub fn connect_blocking(&mut self, result: &ScanResult) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            #[cfg(feature = "native")]
            let connect_result = tokio::task::block_in_place(|| {
                futures::executor::block_on(
                    self.registry.connect(&result.driver, &result.candidate),
                )
            });
            #[cfg(not(feature = "native"))]
            let connect_result = futures::executor::block_on(
                self.registry.connect(&result.driver, &result.candidate),
            );
            match connect_result {
                Ok(device) => {
                    let id = self.session.add_device(device);
                    self.selected_device = Some(id);
                    self.connect_error = None;
                }
                Err(e) => self.connect_error = Some(e.to_string()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.pending_connect = Some(result.clone());
        }
    }

    /// Disconnect a device (synchronous, no I/O needed).
    pub fn disconnect_blocking(&mut self, id: &DeviceId) {
        self.acquisitions.remove(id);
        let _ = self.session.remove(id);
        self.views.remove(id);
        if self.selected_device.as_ref() == Some(id) {
            self.selected_device = self.connected_device_ids().into_iter().next();
        }
    }

    /// Start acquisition for a device (blocking, for use in event handlers).
    pub fn start_blocking(&mut self, id: &DeviceId) {
        if let Some(acq) = self.acquisitions.get_mut(id) {
            // Re-run: apply config sample rate, clear old data, then send Start.
            let rate = acq.config.sample_rate_hz;
            acq.send_command(AcquisitionCommand::SetSampleRate(rate));
            acq.reset_traces();
            acq.send_command(AcquisitionCommand::Start);
            acq.state = AcquisitionState::Running;
        } else if let Some(handle) = self.session.remove(id) {
            let acq = self.spawn_acquisition(handle);
            self.acquisitions.insert(id.clone(), acq);
        }
    }

    /// Stop acquisition for a device (blocking, for use in event handlers).
    pub fn stop_blocking(&mut self, id: &DeviceId) {
        if let Some(acq) = self.acquisitions.get_mut(id) {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        } else if let Some(handle) = self.session.device_mut(id) {
            #[cfg(not(target_arch = "wasm32"))]
            let _ = futures::executor::block_on(handle.apply(AcquisitionCommand::Stop));
        }
    }

    pub fn apply_pending_actions(&mut self) {
        if let Some(result) = self.pending_connect.take() {
            log::debug!("apply_pending_actions: connecting to {}", result.candidate.address);
            #[cfg(not(target_arch = "wasm32"))]
            {
                let connect_result = futures::executor::block_on(
                    self.registry.connect(&result.driver, &result.candidate),
                );
                log::debug!("apply_pending_actions: connect result={:?}", connect_result.as_ref().map(|_| "Ok").map_err(|e| e.to_string()));
                Self::apply_connect_result(
                    connect_result,
                    &mut self.session,
                    &mut self.selected_device,
                    &mut self.connect_error,
                );
            }
            #[cfg(target_arch = "wasm32")]
            {
                // Spawn async connect via wasm-bindgen-futures.
                let registry = self.registry.clone();
                let driver = result.driver.clone();
                let candidate = result.candidate.clone();
                let (tx, rx) = futures::channel::oneshot::channel();
                wasm_bindgen_futures::spawn_local(async move {
                    let r = registry
                        .connect(&driver, &candidate)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(r);
                });
                self.pending_wasm_connect = Some(rx);
                return; // Result arrives next frame.
            }
        }

        if let Some(id) = self.pending_disconnect.take() {
            self.acquisitions.remove(&id);
            let _ = self.session.remove(&id);
            self.views.remove(&id);
            if self.selected_device.as_ref() == Some(&id) {
                self.selected_device = self.connected_device_ids().into_iter().next();
            }
        }

        if let Some(id) = self.pending_start.take() {
            if let Some(acq) = self.acquisitions.get_mut(&id) {
                // Re-run: apply config sample rate, clear old data, then send Start.
                let rate = acq.config.sample_rate_hz;
                acq.send_command(AcquisitionCommand::SetSampleRate(rate));
                acq.reset_traces();
                acq.send_command(AcquisitionCommand::Start);
                acq.state = AcquisitionState::Running;
            } else if let Some(handle) = self.session.remove(&id) {
                // First start: spawn_acquisition arms and starts streaming
                // immediately — no separate Start command needed.
                let acq = self.spawn_acquisition(handle);
                self.acquisitions.insert(id, acq);
            }
        }

        if let Some(id) = self.pending_stop.take() {
            if let Some(acq) = self.acquisitions.get_mut(&id) {
                acq.send_command(AcquisitionCommand::Stop);
                acq.state = AcquisitionState::Stopped;
            } else if let Some(handle) = self.session.device_mut(&id) {
                #[cfg(not(target_arch = "wasm32"))]
                let _ = futures::executor::block_on(handle.apply(AcquisitionCommand::Stop));
            }
        }

        // Check for completed WASM scan.
        #[cfg(target_arch = "wasm32")]
        if let Some(mut rx) = self.pending_wasm_scan.take() {
            if let Ok(Some(result)) = rx.try_recv() {
                match result {
                    Ok(results) => {
                        self.scan_results = results;
                        self.scan_error = None;
                    }
                    Err(e) => {
                        self.scan_results.clear();
                        self.scan_error = Some(e);
                    }
                }
            } else {
                // Not yet ready; put it back for the next frame.
                self.pending_wasm_scan = Some(rx);
            }
        }
        // Check for completed WASM connect.
        #[cfg(target_arch = "wasm32")]
        if let Some(mut rx) = self.pending_wasm_connect.take() {
            if let Ok(Some(result)) = rx.try_recv() {
                Self::apply_connect_result(
                    result,
                    &mut self.session,
                    &mut self.selected_device,
                    &mut self.connect_error,
                );
            } else {
                self.pending_wasm_connect = Some(rx);
            }
        }
    }

    /// Apply a connect result (shared by sync and async paths).
    fn apply_connect_result(
        result: Result<Box<dyn rb_device::Device>, impl ToString>,
        session: &mut Session,
        selected_device: &mut Option<DeviceId>,
        connect_error: &mut Option<String>,
    ) {
        match result {
            Ok(device) => {
                let id = session.add_device(device);
                *selected_device = Some(id);
                *connect_error = None;
            }
            Err(e) => *connect_error = Some(e.to_string()),
        }
    }

    /// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
    ///
    /// All paths now use [`run_acquisition`] so that Stop → Start re-arm is
    /// handled correctly (re-calls `start_streaming()`).
    /// The old native custom-spawn path that bypassed `run_acquisition` could
    /// not recover from a `Stop` because the data-pipe task exited permanently.
    #[allow(unused_mut)]
    pub fn spawn_acquisition(&mut self, mut handle: DeviceHandle) -> DeviceAcquisition {
        let analog = handle.analog_traces().to_vec();
        let digital = handle.digital_trace().cloned();

        // Build initial config from device capabilities — all channels enabled.
        let analog_enabled = vec![true; analog.len()];
        let digital_enabled = digital
            .as_ref()
            .map(|dt| vec![true; dt.channels().len()])
            .unwrap_or_default();
        let sample_rate_hz = analog
            .first()
            .map(|t| t.timebase().sample_rate_hz())
            .unwrap_or(1.0);
        let config = AcquisitionConfig {
            sample_rate_hz,
            analog_enabled,
            digital_enabled,
        };

        #[cfg(target_arch = "wasm32")]
        {
            let web_handle = rb_core::runtime::web::spawn_local(handle);
            // The spawned run_acquisition future waits for a Start command
            // before arming the device — send it now so streaming begins.
            // Apply sample rate, then start streaming.
            let _ = web_handle
                .commands
                .unbounded_send(AcquisitionCommand::SetSampleRate(config.sample_rate_hz));
            let _ = web_handle.commands.unbounded_send(AcquisitionCommand::Start);
            return DeviceAcquisition {
                analog,
                digital,
                state: AcquisitionState::Running,
                sample_count: 0,
                cmd_tx: web_handle.commands,
                data_rx: web_handle.data,
                config,
            };
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (cmd_tx, cmd_rx) = mpsc::unbounded::<AcquisitionCommand>();
            let (data_tx, data_rx) = mpsc::unbounded();

            // Spawn run_acquisition — it handles Start/Stop/restart
            // internally via select!, correctly calling start_streaming()
            // on each re-arm.
            let fut = run_acquisition(handle, cmd_rx, Some(data_tx));
            #[cfg(feature = "native")]
            {
                self.local_set.spawn_local(async move {
                    let _handle = fut.await;
                });
            }
            #[cfg(not(feature = "native"))]
            {
                self.spawner
                    .spawn_local(async move {
                        let _handle = fut.await;
                    })
                    .expect("spawn");
            }

            // Apply sample rate, then start streaming.
            let _ = cmd_tx.unbounded_send(AcquisitionCommand::SetSampleRate(config.sample_rate_hz));
            let _ = cmd_tx.unbounded_send(AcquisitionCommand::Start);

            DeviceAcquisition {
                analog,
                digital,
                state: AcquisitionState::Running,
                sample_count: 0,
                cmd_tx,
                data_rx,
                config,
            }
        }
    }

    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        let mut ids: Vec<DeviceId> = self.session.device_ids();
        ids.extend(self.acquisitions.keys().cloned());
        ids
    }

    pub fn device_label(&self, id: &DeviceId) -> String {
        if let Some(handle) = self.session.device(id) {
            let info = handle.device().info();
            return format!("{}/{}", info.vendor, info.model);
        }
        id.to_string()
    }

    pub fn device_state(&self, id: &DeviceId) -> Option<AcquisitionState> {
        if let Some(acq) = self.acquisitions.get(id) {
            Some(acq.state().clone())
        } else {
            self.session.device(id).map(|handle| handle.state().clone())
        }
    }

    pub fn device_sample_count(&self, id: &DeviceId) -> usize {
        if let Some(acq) = self.acquisitions.get(id) {
            acq.sample_count()
        } else if let Some(handle) = self.session.device(id) {
            handle.sample_count()
        } else {
            0
        }
    }

    /// Triggers a scan (native: block_on; web: spawn_local + oneshot).
    pub fn trigger_scan(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let result = futures::executor::block_on(self.registry.scan_all());
            match result {
                Ok(results) => {
                    self.scan_results = results;
                    self.scan_error = None;
                }
                Err(e) => {
                    self.scan_results.clear();
                    self.scan_error = Some(e.to_string());
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let registry = self.registry.clone();
            let (tx, rx) = futures::channel::oneshot::channel();
            wasm_bindgen_futures::spawn_local(async move {
                // Request any known USB device so it appears in
                // subsequent getDevices() calls.
                let _ = request_supported_usb_devices().await;
                let result = registry.scan_all().await.map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
            self.pending_wasm_scan = Some(rx);
        }
    }

    /// Run the executor for one tick. Abstracts over native (tokio
    /// `LocalSet` with timeout), web (no-op — `wasm-bindgen-futures` drives
    /// acquisition tasks), and non-native (`LocalPool`).
    pub fn pump_once(&mut self) {
        // On WASM, acquisition futures run on `wasm-bindgen-futures`, not the
        // `LocalPool`. The browser's microtask queue drives them; we only need
        // to drain data here.
        #[cfg(target_arch = "wasm32")]
        return;

        #[cfg(feature = "native")]
        {
            // Drive tokio's LocalSet for a short window using block_in_place
            // to safely block within Dioxus's async context.
            let handle = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                handle.block_on(async {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_micros(500),
                        self.local_set
                            .run_until(futures::future::pending::<()>()),
                    )
                    .await;
                });
            });
        }
        #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
        self.pool.run_until_stalled();
    }

    /// Drains all acquisition data into local stores. Returns true if any
    /// new data arrived.
    pub fn drain_all(&mut self) -> bool {
        let mut had_data = false;
        for acq in self.acquisitions.values_mut() {
            let before = acq.sample_count();
            acq.drain();
            if acq.sample_count() > before {
                had_data = true;
            }
        }
        had_data
    }

    /// Returns true if any acquisition is currently running.
    pub fn any_running(&self) -> bool {
        self.acquisitions
            .values()
            .any(|a| a.state() == &AcquisitionState::Running)
    }

    /// Returns true if any WASM async operation is pending.
    #[cfg(target_arch = "wasm32")]
    pub fn wasm_pending(&self) -> bool {
        self.pending_wasm_scan.is_some() || self.pending_wasm_connect.is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wasm_pending(&self) -> bool {
        false
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::from_session(Session::new())
    }
}

// ── WASM helpers ──────────────────────────────────────────────────────────────

/// Requests permission for all known USB devices via the WebUSB API.
///
/// Calls `navigator.usb.requestDevice()` with filters built from every
/// driver's [`rb_drivers::known_usb_vid_pids`] list. The browser will show a
/// permission dialog; once granted, the device will appear in subsequent
/// `nusb::list_devices()` calls.
#[cfg(target_arch = "wasm32")]
async fn request_supported_usb_devices() {
    use wasm_bindgen_futures::JsFuture;

    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let usb = window.navigator().usb();
    if usb.is_undefined() {
        return;
    }

    let known = ::rb_drivers::known_usb_vid_pids();
    if known.is_empty() {
        return;
    }
    let mut filters: Vec<web_sys::UsbDeviceFilter> = Vec::new();
    for (vid, pid) in &known {
        let filter = web_sys::UsbDeviceFilter::new();
        filter.set_vendor_id(*vid);
        filter.set_product_id(*pid);
        filters.push(filter);
    }

    let options = web_sys::UsbDeviceRequestOptions::new(&filters);

    // This triggers the browser permission dialog.
    let _ = JsFuture::from(usb.request_device(&options)).await;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Block on a future using a temporary tokio runtime (required by nusb).
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn app_initializes_empty() {
        let app = AppState::default();
        assert!(app.session.is_empty());
        assert!(app.acquisitions.is_empty());
        assert!(app.scan_results.is_empty());
        assert!(app.selected_device.is_none());
    }

    #[test]
    fn scan_populates_results() {
        let mut app = AppState::default();
        for r in block_on(app.registry.scan_all()).unwrap() {
            app.scan_results.push(r);
        }
        assert!(!app.scan_results.is_empty());
        assert!(app.scan_results.iter().any(|r| r.driver == "demo"));
    }

    #[test]
    fn connect_adds_device_to_session() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        assert_eq!(app.session.len(), 1);
        assert!(app.selected_device.is_some());
    }

    #[test]
    fn disconnect_removes_device() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        let id = app.selected_device.clone().unwrap();
        app.pending_disconnect = Some(id);
        app.apply_pending_actions();
        assert!(app.session.is_empty());
        assert!(app.acquisitions.is_empty());
        assert!(app.selected_device.is_none());
    }

    #[test]
    fn start_spawns_and_pumps_samples() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        let id = app.selected_device.clone().unwrap();

        // Start acquisition.
        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(app.acquisitions.contains_key(&id));

        // Drive the pool repeatedly, waiting between calls for
        // futures_timer delays to fire. The first iteration processes
        // Start and sleeps 50 ms (idle back-off); subsequent iterations
        // pump data every ~1 ms.
        for _ in 0..10 {
            app.pump_once();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Drain data.
        if let Some(acq) = app.acquisitions.get_mut(&id) {
            acq.drain();
        }

        let count = app.device_sample_count(&id);
        assert!(count > 0, "expected samples after pump, got {count}");
    }

    fn scan_for_demo(app: &AppState) -> ScanResult {
        block_on(app.registry.scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
