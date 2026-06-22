//! Platform-neutral RustyBench GUI built on [`eframe`]/egui.
//!
//! The same [`RustyBenchApp`] runs natively (via `rb-gui-native`) and in the
//! browser (via `rb-gui-web`).
//!
//! # Frame loop
//! 1. Deferred actions from the previous frame are applied.
//! 2. Data drained from background acquisition tasks into local stores.
//! 3. Sidebar + waveform panels are drawn.

#![forbid(unsafe_code)]

mod waveform_view;

use std::collections::{HashMap, HashSet};

use eframe::egui;
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::StreamExt;
use rb_core::{
    AcquisitionCommand, AcquisitionState, DeviceHandle, DriverRegistry, ScanResult, Session,
};

use rb_device::DeviceId;
use rb_model::{AnalogTrace, DigitalTrace, SampleChunk};
use waveform_view::WaveformView;
// Executors: tokio LocalSet when native feature is enabled (non-wasm),
// LocalPool otherwise (wasm + tests).
#[cfg(not(feature = "native"))]
use {futures::executor::LocalPool, futures::executor::LocalSpawner, futures::task::LocalSpawnExt};
#[cfg(feature = "native")]
use tokio::task::LocalSet;

// ── Per-device acquisition state ──────────────────────────────────────────────

/// Bundles a background acquisition future with the local display stores.
///
/// The acquisition future runs continuously on the platform's native spawner
/// (tokio `LocalSet` for native, `wasm-bindgen-futures` for web). Data flows
/// back through `data_rx` into the local stores.
struct DeviceAcquisition {
    analog: Vec<AnalogTrace>,
    digital: Option<DigitalTrace>,
    state: AcquisitionState,
    sample_count: usize,
    /// Sends [`AcquisitionCommand`]s to the spawned future.
    cmd_tx: mpsc::UnboundedSender<AcquisitionCommand>,
    /// Receives [`SampleChunk`]s from the spawned future.
    data_rx: mpsc::UnboundedReceiver<SampleChunk>,
}

impl DeviceAcquisition {
    fn drain(&mut self) {
        let chunks: Vec<SampleChunk> =
            std::iter::from_fn(|| self.data_rx.try_recv().ok()).collect();
        for chunk in &chunks {
            for (index, trace) in self.analog.iter_mut().enumerate() {
                if let Some(samples) = chunk.analog_channel(index) {
                    trace.push_raw(samples);
                }
            }
            if let Some(ref mut digital) = self.digital {
                if !chunk.logic().is_empty() {
                    digital.push_words(chunk.logic());
                }
            }
            self.sample_count += chunk.sample_count();
        }
    }

    fn sample_count(&self) -> usize {
        self.sample_count
    }
    fn analog_traces(&self) -> &[AnalogTrace] {
        &self.analog
    }
    fn digital_trace(&self) -> Option<&DigitalTrace> {
        self.digital.as_ref()
    }
    fn state(&self) -> &AcquisitionState {
        &self.state
    }

    fn send_command(&self, cmd: AcquisitionCommand) {
        let _ = self.cmd_tx.unbounded_send(cmd);
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct RustyBenchApp {
    session: Session,
    registry: DriverRegistry,
    scan_results: Vec<ScanResult>,
    scan_error: Option<String>,
    connect_error: Option<String>,
    selected_device: Option<DeviceId>,
    views: HashMap<DeviceId, WaveformView>,
    acquisitions: HashMap<DeviceId, DeviceAcquisition>,
    /// Executor for background acquisition futures.
    #[cfg(not(feature = "native"))]
    #[allow(dead_code)]
    pool: LocalPool,
    #[cfg(not(feature = "native"))]
    #[allow(dead_code)]
    spawner: LocalSpawner,
    /// Tokio runtime + LocalSet for native builds (proper I/O integration).
    #[cfg(feature = "native")]
    rt: tokio::runtime::Runtime,
    #[cfg(feature = "native")]
    local_set: LocalSet,
    // Deferred actions.
    pending_connect: Option<ScanResult>,
    pending_disconnect: Option<DeviceId>,
    pending_start: Option<DeviceId>,
    pending_stop: Option<DeviceId>,
}

impl RustyBenchApp {
    /// Creates an app with a pre-populated [`Session`], for integration tests
    /// that need to inject a mock device without going through the driver
    /// registry.
    ///
    /// The session's devices appear as connected in the UI and are ready for
    /// acquisition — no scan or connect step needed.
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        #[cfg(not(feature = "native"))]
        let (pool, spawner) = {
            let p = LocalPool::new();
            let s = p.spawner();
            (p, s)
        };
        #[cfg(feature = "native")]
        let (rt, local_set) = {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("tokio runtime");
            let ls = LocalSet::new();
            (rt, ls)
        };
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
            #[cfg(not(feature = "native"))]
            pool,
            #[cfg(not(feature = "native"))]
            spawner,
            #[cfg(feature = "native")]
            rt,
            #[cfg(feature = "native")]
            local_set,
            pending_connect: None,
            pending_disconnect: None,
            pending_start: None,
            pending_stop: None,
        }
    }
}

impl Default for RustyBenchApp {
    fn default() -> Self {
        Self::from_session(Session::new())
    }
}

impl RustyBenchApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn apply_pending_actions(&mut self) {
        if let Some(result) = self.pending_connect.take() {
            match block_on(self.registry.connect(&result.driver, &result.candidate)) {
                Ok(device) => {
                    let id = self.session.add_device(device);
                    self.selected_device = Some(id);
                    self.connect_error = None;
                }
                Err(e) => self.connect_error = Some(e.to_string()),
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
                // Re-arm after stop: just set state (the command handler
                // will re-arm via send_command if needed).
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
                let _ = block_on(handle.apply(AcquisitionCommand::Stop));
            }
        }
    }

    /// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
    ///
    /// On native: spawns the raw read-loop directly (matching the CLI's proven
    /// pattern), avoiding the `select!` wrapper in `run_acquisition`.
    /// On web: spawns via `wasm-bindgen-futures::spawn_local`.
    fn spawn_acquisition(&mut self, mut handle: DeviceHandle) -> DeviceAcquisition {
        let analog = handle.analog_traces().to_vec();
        let digital = handle.digital_trace().cloned();

        #[cfg(target_arch = "wasm32")]
        {
            let web_handle = rb_core::runtime::web::spawn_local(handle, 512);
            return DeviceAcquisition {
                analog,
                digital,
                state: AcquisitionState::Running,
                sample_count: 0,
                cmd_tx: web_handle.commands,
                data_rx: web_handle.data,
            };
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (cmd_tx, cmd_rx) = mpsc::unbounded::<AcquisitionCommand>();
            let (data_tx, data_rx) = mpsc::unbounded();

            let (read_loop, internal_rx) =
                match block_on(async { handle.start_streaming().await }) {
                    Ok((rl, rx)) => (rl, rx),
                    Err(e) => {
                        return DeviceAcquisition {
                            analog,
                            digital,
                            state: AcquisitionState::Error(e.to_string()),
                            sample_count: 0,
                            cmd_tx,
                            data_rx,
                        };
                    }
                };

            let handle = std::rc::Rc::new(std::cell::RefCell::new(handle));

            // Spawn on tokio LocalSet (native) or LocalPool (fallback/tests).
            #[cfg(feature = "native")]
            {
                self.local_set.spawn_local(read_loop);
                let h = handle.clone();
                self.local_set.spawn_local(async move {
                    use futures::StreamExt;
                    let mut rx = internal_rx;
                    while let Some(chunk) = rx.next().await {
                        h.borrow_mut().ingest_chunk(&chunk);
                        if data_tx.unbounded_send(chunk).is_err() {
                            break;
                        }
                    }
                });
                let h = handle.clone();
                self.local_set.spawn_local(async move {
                    use futures::StreamExt;
                    let mut rx = cmd_rx;
                    while let Some(cmd) = rx.next().await {
                        let _ = h.borrow_mut().apply(cmd).await;
                    }
                });
            }
            #[cfg(not(feature = "native"))]
            {
                self.spawner.spawn_local(read_loop).expect("spawn");
                let h = handle.clone();
                self.spawner.spawn_local(async move {
                    use futures::StreamExt;
                    let mut rx = internal_rx;
                    while let Some(chunk) = rx.next().await {
                        h.borrow_mut().ingest_chunk(&chunk);
                        if data_tx.unbounded_send(chunk).is_err() {
                            break;
                        }
                    }
                }).expect("spawn");
                let h = handle.clone();
                self.spawner.spawn_local(async move {
                    use futures::StreamExt;
                    let mut rx = cmd_rx;
                    while let Some(cmd) = rx.next().await {
                        let _ = h.borrow_mut().apply(cmd).await;
                    }
                }).expect("spawn");
            }

            DeviceAcquisition {
                analog,
                digital,
                state: AcquisitionState::Running,
                sample_count: 0,
                cmd_tx,
                data_rx,
            }
        }
    }

    fn connected_device_ids(&self) -> Vec<DeviceId> {
        let mut ids: Vec<DeviceId> = self.session.device_ids();
        ids.extend(self.acquisitions.keys().cloned());
        ids
    }

    fn device_label(&self, id: &DeviceId) -> String {
        if let Some(handle) = self.session.device(id) {
            let info = handle.device().info();
            return format!("{}/{}", info.vendor, info.model);
        }
        id.to_string()
    }

    fn device_state(&self, id: &DeviceId) -> Option<AcquisitionState> {
        if let Some(acq) = self.acquisitions.get(id) {
            Some(acq.state().clone())
        } else if let Some(handle) = self.session.device(id) {
            Some(handle.state().clone())
        } else {
            None
        }
    }

    fn device_sample_count(&self, id: &DeviceId) -> usize {
        if let Some(acq) = self.acquisitions.get(id) {
            acq.sample_count()
        } else if let Some(handle) = self.session.device(id) {
            handle.sample_count()
        } else {
            0
        }
    }

    // ── Sidebar ───────────────────────────────────────────────────────────

    fn draw_device_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Devices");
        ui.separator();

        if ui.button("\u{27F3} Scan").clicked() {
            match block_on(self.registry.scan_all()) {
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
        if let Some(err) = &self.scan_error {
            ui.colored_label(egui::Color32::RED, err.as_str());
        }
        if let Some(err) = &self.connect_error {
            ui.colored_label(egui::Color32::RED, err.as_str());
        }

        let connected: HashSet<String> = self
            .connected_device_ids()
            .iter()
            .map(|id| id.as_str().to_string())
            .collect();

        let available: Vec<ScanResult> = self
            .scan_results
            .iter()
            .filter(|r| !connected.contains(&r.candidate.address))
            .cloned()
            .collect();

        if !available.is_empty() {
            ui.separator();
            ui.label("Available:");
            for r in available {
                ui.horizontal(|ui| {
                    ui.label(&r.candidate.address);
                    if ui.small_button("Connect").clicked() {
                        self.pending_connect = Some(r);
                    }
                });
            }
        }

        ui.separator();
        ui.label("Connected:");
        let device_ids = self.connected_device_ids();
        if device_ids.is_empty() {
            ui.weak("(none)");
        }

        for id in &device_ids {
            let label = self.device_label(id);
            let is_selected = self.selected_device.as_ref() == Some(id);
            if ui.selectable_label(is_selected, &label).clicked() {
                self.selected_device = Some(id.clone());
            }

            ui.horizontal(|ui| {
                let device_state = self.device_state(id);
                match device_state {
                    Some(AcquisitionState::Running) => {
                        ui.colored_label(egui::Color32::GREEN, "\u{25CF}");
                        if ui.small_button("\u{23F9}").on_hover_text("Stop").clicked() {
                            self.pending_stop = Some(id.clone());
                        }
                    }
                    Some(AcquisitionState::Error(msg)) => {
                        ui.colored_label(egui::Color32::RED, "\u{26A0}")
                            .on_hover_text(&msg);
                    }
                    _ => {
                        if ui.small_button("\u{25B6}").on_hover_text("Start").clicked() {
                            self.pending_start = Some(id.clone());
                        }
                    }
                }
                if ui
                    .small_button("\u{2716}")
                    .on_hover_text("Disconnect")
                    .clicked()
                {
                    self.pending_disconnect = Some(id.clone());
                }
            });

            ui.weak(format!("{} samples", self.device_sample_count(id)));
            ui.add_space(4.0);
        }
    }

    // ── Central panel ─────────────────────────────────────────────────────

    fn draw_main_panel(&mut self, ui: &mut egui::Ui) {
        let device_ids = self.connected_device_ids();
        if device_ids.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No devices connected.\n\nUse the sidebar to scan and connect a device.");
            });
            return;
        }

        if self
            .selected_device
            .as_ref()
            .is_none_or(|id| !device_ids.contains(id))
        {
            self.selected_device = device_ids.first().cloned();
        }

        ui.horizontal(|ui| {
            for id in &device_ids {
                let label = self.device_label(id);
                let selected = self.selected_device.as_ref() == Some(id);
                if ui.selectable_label(selected, label).clicked() {
                    self.selected_device = Some(id.clone());
                }
            }
        });
        ui.separator();

        if let Some(id) = self.selected_device.clone() {
            self.draw_waveform_for(ui, &id);
        }
    }

    fn draw_waveform_for(&mut self, ui: &mut egui::Ui, id: &DeviceId) {
        let view = self.views.entry(id.clone()).or_default();
        if let Some(acq) = self.acquisitions.get(id) {
            view.draw_direct(
                ui,
                acq.state(),
                acq.analog_traces(),
                acq.digital_trace(),
                acq.sample_count(),
            );
        } else if let Some(handle) = self.session.device(id) {
            view.draw(ui, handle);
        }
    }

    /// Run the executor for one tick.  Abstracts over native (tokio
    /// `LocalSet` with timeout) and non-native (`LocalPool`).
    fn pump_once(&mut self) {
        #[cfg(feature = "native")]
        {
            // Drive tokio's LocalSet for a short window — matching what
            // LocalPool::run_until_stalled() does, but with proper I/O
            // integration (USB completions are handled by tokio's reactor).
            let _ = self.rt.block_on(async {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_micros(500),
                    self.local_set.run_until(futures::future::pending::<()>()),
                )
                .await;
            });
        }
        #[cfg(not(feature = "native"))]
        self.pool.run_until_stalled();
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RustyBenchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.apply_pending_actions();

        // Drive the executor in a tight loop while acquisitions are running.
        loop {
            self.pump_once();

            let mut had_data = false;
            for acq in self.acquisitions.values_mut() {
                let before = acq.sample_count();
                acq.drain();
                if acq.sample_count() > before {
                    had_data = true;
                }
            }
            if !had_data {
                break;
            }
        }

        // Keep requesting repaints while any acquisition is running, so the
        // pool stays alive and data flows continuously even without user input.
        let any_running = self
            .acquisitions
            .values()
            .any(|a| a.state() == &AcquisitionState::Running);
        if any_running {
            ui.ctx().request_repaint();
        }

        egui::Panel::left("device_panel")
            .resizable(true)
            .default_size(220.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.draw_device_panel(ui);
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.draw_main_panel(ui);
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_initializes_empty() {
        let app = RustyBenchApp::default();
        assert!(app.session.is_empty());
        assert!(app.acquisitions.is_empty());
        assert!(app.scan_results.is_empty());
        assert!(app.selected_device.is_none());
    }

    #[test]
    fn scan_populates_results() {
        let mut app = RustyBenchApp::default();
        for r in block_on(app.registry.scan_all()).unwrap() {
            app.scan_results.push(r);
        }
        assert!(!app.scan_results.is_empty());
        assert!(app.scan_results.iter().any(|r| r.driver == "demo"));
    }

    #[test]
    fn connect_adds_device_to_session() {
        let mut app = RustyBenchApp::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        assert_eq!(app.session.len(), 1);
        assert!(app.selected_device.is_some());
    }

    #[test]
    fn disconnect_removes_device() {
        let mut app = RustyBenchApp::default();
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
        let mut app = RustyBenchApp::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        let id = app.selected_device.clone().unwrap();

        // Start acquisition.
        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(app.acquisitions.contains_key(&id));

        // Drive the pool repeatedly, waiting between calls for
        // futures_timer delays to fire.  The first iteration processes
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

    fn scan_for_demo(app: &RustyBenchApp) -> ScanResult {
        block_on(app.registry.scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }

    // ── fx2lafw integration tests (MockTransport, no USB hardware) ──────────

    use rb_device::{DeviceId, DeviceInfo};
    use rb_drivers::fx2lafw::{Fx2lafwConfig, Fx2lafwDevice};
    use rb_transport::MockTransport;

    // ── SteppedTransport: simulates async USB reads ────────────────────────
    //
    // Real USB (nusb) returns `Pending` on the first poll of each read and
    // `Ready` only after the waker fires (on a later poll cycle).  MockTransport
    // always returns `Ready` immediately, so `run_until_stalled()` processes
    // everything in one call — hiding the frame-boundary issue that the GUI
    // hits with real hardware.
    //
    // `SteppedTransport` gates each `read()` call behind a barrier: it returns
    // `Pending` until the test calls `step()`.  Each `step()` unblocks exactly
    // one pending read.  This forces the acquisition future to yield at every
    // read, so the test must call `run_until_stalled()` once per chunk — exactly
    // like the GUI frame loop.

    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use async_trait::async_trait;
    use rb_transport::{
        Transport, TransportCapabilities, TransportError, TransportKind, TransportResult,
    };

    /// An async transport that unblocks one read per `step()` call.
    struct SteppedTransport {
        caps: TransportCapabilities,
        /// Queued byte chunks — each chunk is one read's worth of data.
        chunks: RefCell<VecDeque<Vec<u8>>>,
        /// Oneshot senders for blocked reads — the transport awaits
        /// these, and `step()` resolves the oldest one.
        steps: RefCell<VecDeque<futures::channel::oneshot::Sender<Vec<u8>>>>,
        control_responses: RefCell<VecDeque<Vec<u8>>>,
        control_transfers: RefCell<Vec<ControlTransferRecord>>,
        read_errors: RefCell<VecDeque<String>>,
    }

    /// Newtype wrapper so we can implement [`Transport`] (orphan rule).
    struct StepTransport(Rc<SteppedTransport>);

    impl StepTransport {
        fn queue_chunk(&self, data: impl AsRef<[u8]>) {
            self.0.chunks.borrow_mut().push_back(data.as_ref().to_vec());
        }

        fn queue_control_response(&self, data: impl AsRef<[u8]>) {
            self.0
                .control_responses
                .borrow_mut()
                .push_back(data.as_ref().to_vec());
        }

        /// Unblocks exactly one pending `read()` call with the next chunk
        /// (or an empty vec for EOF).
        fn step(&self) {
            let data = self.0.chunks.borrow_mut().pop_front().unwrap_or_default();
            if let Some(tx) = self.0.steps.borrow_mut().pop_front() {
                let _ = tx.send(data);
            }
        }
    }

    #[async_trait(?Send)]
    impl Transport for StepTransport {
        fn capabilities(&self) -> TransportCapabilities {
            self.0.caps
        }

        async fn write(&mut self, _data: &[u8]) -> TransportResult<usize> {
            Ok(0) // not used by fx2lafw read loop
        }

        async fn read(&mut self, buf: &mut [u8]) -> TransportResult<usize> {
            // Check for queued errors first.
            if let Some(msg) = self.0.read_errors.borrow_mut().pop_front() {
                return Err(TransportError::Io(msg));
            }

            let (tx, rx) = futures::channel::oneshot::channel();
            self.0.steps.borrow_mut().push_back(tx);
            // release borrows before awaiting
            let data = rx.await.map_err(|_| TransportError::Io("step channel closed".into()))?;

            if data.is_empty() {
                return Ok(0); // EOF
            }
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
            self.0.control_transfers.borrow_mut().push(ControlTransferRecord {
                request_type,
                request,
                value,
                index,
                data: data.to_vec(),
            });
            Ok(self
                .0
                .control_responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_default())
        }
    }

    // Re-import ControlTransferRecord (needed for SteppedTransport).
    use rb_transport::ControlTransferRecord;

    /// Helper: build an `Fx2lafwDevice` with a `MockTransport` whose control-
    /// transfer slots and read-data queue are already populated.
    ///
    /// Returns the device and the transport (so the caller can inspect
    /// recorded control transfers if needed).
    fn fx2lafw_mock_device(
        channels: u8,
        read_data: &[u8],
    ) -> (Fx2lafwDevice, MockTransport) {
        let mut transport = MockTransport::new();
        // Control responses consumed by open() and arm().
        transport.queue_control_response([1, 4]); // fw version 1.4
        transport.queue_control_response([1]); // revid=1 → FX2LP
        transport.queue_control_response([]); // start_acquisition response
        // Queue the sample data, then an empty read for clean EOF.
        transport.queue_read(read_data);
        transport.queue_read(&[]);

        let id = DeviceId::new("fx2lafw-test");
        let info = DeviceInfo::new("Test", "FX2LP");
        let config = Fx2lafwConfig {
            channels,
            sample_rate_hz: 1_000_000.0,
        };
        let dev = Fx2lafwDevice::new(id, info, Box::new(transport.clone()), config);
        (dev, transport)
    }

    /// Pump the app's cooperative executor repeatedly so the acquisition
    /// future makes progress and delivers chunks.  Returns the total number of
    /// samples drained.
    fn pump_and_drain(app: &mut RustyBenchApp, id: &DeviceId, iterations: usize) -> usize {
        for _ in 0..iterations {
            app.pump_once();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Final stall to let the read-loop exit on EOF.
        app.pump_once();
        if let Some(acq) = app.acquisitions.get_mut(id) {
            acq.drain();
        }
        app.device_sample_count(id)
    }

    #[test]
    fn fx2lafw_8bit_streaming_via_gui_exceeds_4096_samples() {
        // 8192 bytes = 8192 samples in 8-bit mode — well beyond one 4096-byte
        // read buffer, so the read loop must iterate at least twice.
        let read_data: Vec<u8> = (0u8..=255).cycle().take(8192).collect();
        let (dev, _transport) = fx2lafw_mock_device(8, &read_data);

        let mut session = Session::new();
        let id = session.add_device(Box::new(dev));
        let mut app = RustyBenchApp::from_session(session);

        // Simulate pressing ▶ in the sidebar.
        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(
            app.acquisitions.contains_key(&id),
            "acquisition should be spawned"
        );

        let count = pump_and_drain(&mut app, &id, 30);
        assert!(
            count > 4096,
            "expected > 4096 samples with 8192 bytes queued, got {count}"
        );
    }

    #[test]
    fn fx2lafw_packet_oriented_streaming_via_gui_exceeds_4096_samples() {
        // Packet-oriented MockTransport caps each read at 512 bytes (like USB
        // bulk).  8192 bytes require ~16 reads — if the loop exits after the
        // first read (4096-byte buffer), we get ≤ 4096 samples.
        let read_data: Vec<u8> = (0u8..=255).cycle().take(8192).collect();

        // Build a packet-oriented transport manually (not via the helper).
        let mut transport = MockTransport::packet(512);
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        transport.queue_read(&read_data);
        transport.queue_read(&[]);

        let id = DeviceId::new("fx2lafw-pkt");
        let info = DeviceInfo::new("Test", "FX2LP");
        let dev = Fx2lafwDevice::new(
            id.clone(),
            info,
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        let mut session = Session::new();
        session.add_device(Box::new(dev));
        let mut app = RustyBenchApp::from_session(session);

        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(app.acquisitions.contains_key(&id));

        let count = pump_and_drain(&mut app, &id, 50);
        assert!(
            count > 4096,
            "packet-oriented (512-byte reads): expected > 4096 samples, got {count}"
        );
    }

    #[test]
    fn fx2lafw_16bit_streaming_via_gui_decodes_all_samples() {
        // 16-bit mode: 8192 bytes = 4096 samples.  This is exactly 4096
        // samples, which would be the bug threshold if the loop misbehaves.
        // We verify ALL samples arrive.
        let read_data: Vec<u8> = (0..4096u16)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(read_data.len(), 8192);

        let mut transport = MockTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        transport.queue_read(&read_data);
        transport.queue_read(&[]);

        let id = DeviceId::new("fx2lafw-16bit");
        let info = DeviceInfo::new("Test", "FX2LP-16");
        let dev = Fx2lafwDevice::new(
            id.clone(),
            info,
            Box::new(transport),
            Fx2lafwConfig {
                channels: 16,
                sample_rate_hz: 1_000_000.0,
            },
        );

        let mut session = Session::new();
        session.add_device(Box::new(dev));
        let mut app = RustyBenchApp::from_session(session);

        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(app.acquisitions.contains_key(&id));

        let count = pump_and_drain(&mut app, &id, 30);
        assert_eq!(
            count, 4096,
            "16-bit mode with 8192 bytes should yield exactly 4096 samples, got {count}"
        );
    }

    // ── SteppedTransport: frame-boundary simulation ───────────────────────

    /// Simulates the GUI's frame-by-frame polling where each `read()` yields
    /// once (like real USB) and the test must `step()` between
    /// `run_until_stalled()` calls to unblock each read.
    ///
    /// This test **reproduces the 4096-sample bug**: with a single
    /// `run_until_stalled()` per frame, no data arrives.  Each `step()`
    /// simulates USB data arriving between frames, and each subsequent
    /// `run_until_stalled()` + `drain()` simulates the GUI's pump loop.
    /// The test verifies that the tight pump loop (matching the CLI's
    /// `run_until()` pattern) processes all chunks.
    #[test]
    fn stepped_transport_multi_chunk_via_tight_pump() {
        // Clone-friendly setup: keep a StepTransport handle for stepping.
        let inner = Rc::new(SteppedTransport {
            caps: TransportCapabilities {
                kind: TransportKind::Mock,
                packet_oriented: false,
                max_transfer: None,
            },
            chunks: RefCell::new(VecDeque::new()),
            steps: RefCell::new(VecDeque::new()),
            control_responses: RefCell::new(VecDeque::new()),
            control_transfers: RefCell::new(Vec::new()),
            read_errors: RefCell::new(VecDeque::new()),
        });
        let stepper = StepTransport(inner.clone());

        // Queue control responses + data chunks.
        stepper.queue_control_response([1, 4]);
        stepper.queue_control_response([1]);
        stepper.queue_control_response([]);
        for i in 0u8..4 {
            stepper.queue_chunk(vec![i; 4096]);
        }
        stepper.queue_chunk(&[]); // EOF

        let id = DeviceId::new("fx2lafw-stepped");
        let info = DeviceInfo::new("Test", "FX2LP");
        // The device takes Box<dyn Transport>, moving our StepTransport.
        let dev = Fx2lafwDevice::new(
            id.clone(),
            info,
            Box::new(StepTransport(inner)),
            Fx2lafwConfig::default(),
        );

        let mut session = Session::new();
        session.add_device(Box::new(dev));
        let mut app = RustyBenchApp::from_session(session);

        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        assert!(app.acquisitions.contains_key(&id));

        // Frame 1: pump_once — the acquisition starts, read_loop
        // calls transport.read(), gets blocked on the step barrier.
        app.pump_once();
        if let Some(acq) = app.acquisitions.get_mut(&id) {
            acq.drain();
        }
        assert_eq!(
            app.device_sample_count(&id),
            0,
            "no data before first step"
        );

        // Simulate 4 frames: step → run_until_stalled → drain.
        for expected_chunks in 1..=4 {
            stepper.step(); // USB data "arrives"
            app.pump_once(); // frame pump
            if let Some(acq) = app.acquisitions.get_mut(&id) {
                acq.drain();
            }
            let count = app.device_sample_count(&id);
            assert!(
                count >= expected_chunks * 4096,
                "after {expected_chunks} step(s), expected >= {} samples, got {count}",
                expected_chunks * 4096
            );
        }

        // Final step for EOF → read loop exits.
        stepper.step();
        app.pump_once();
        if let Some(acq) = app.acquisitions.get_mut(&id) {
            acq.drain();
        }
        assert_eq!(
            app.device_sample_count(&id),
            4 * 4096,
            "all 4 chunks should arrive"
        );
    }
}
