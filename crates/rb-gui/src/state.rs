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

// ── Session identifier ────────────────────────────────────────────────────────

/// Opaque identifier for a GUI session (one tab = one session = one device).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub(crate) u64);

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

// ── Per-session state ─────────────────────────────────────────────────────────

/// All state owned by one GUI session (one tab).
///
/// Each session is tied to at most one device. The device is assigned via the
/// device dropdown but **not connected** until the user presses Play.
pub struct SessionState {
    pub id: SessionId,
    /// Display name shown in the tab (device label or "Untitled").
    pub label: String,
    /// The device candidate assigned to this session (via dropdown).
    /// `None` until the user picks a device. Connection happens on Play.
    pub assigned_device: Option<ScanResult>,
    /// Holds the connected device handle (if connected).
    pub session: Session,
    /// Active acquisition, if running or stopped.
    pub acquisition: Option<DeviceAcquisition>,
    /// Per-session waveform pan/zoom/marker state.
    pub view: WaveformView,
    /// The id of the connected device, persisted even when the handle is
    /// temporarily moved into the acquisition future.
    connected_id: Option<DeviceId>,
}

impl SessionState {
    pub fn new(id: SessionId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            assigned_device: None,
            session: Session::new(),
            acquisition: None,
            view: WaveformView::default(),
            connected_id: None,
        }
    }

    /// Returns the connected device id, if any.
    /// Checks `connected_id` first (persisted across handle moves),
    /// then falls back to the session's device list.
    pub fn connected_device_id(&self) -> Option<DeviceId> {
        if let Some(ref id) = self.connected_id {
            Some(id.clone())
        } else {
            self.session.device_ids().into_iter().next()
        }
    }

    /// Returns the device label (vendor/model) for display.
    pub fn device_label(&self) -> Option<String> {
        self.connected_device_id()
            .and_then(|id| self.session.device(&id))
            .map(|h| {
                let info = h.device().info();
                format!("{}/{}", info.vendor, info.model)
            })
    }

    /// Returns the current acquisition state.
    pub fn acquisition_state(&self) -> AcquisitionState {
        if let Some(acq) = &self.acquisition {
            acq.state().clone()
        } else {
            self.session
                .device_ids()
                .first()
                .and_then(|id| self.session.device(id))
                .map(|h| h.state().clone())
                .unwrap_or(AcquisitionState::Idle)
        }
    }

    /// Returns the sample count from the acquisition or device handle.
    pub fn sample_count(&self) -> usize {
        if let Some(acq) = &self.acquisition {
            acq.sample_count()
        } else {
            self.connected_device_id()
                .and_then(|id| self.session.device(&id))
                .map(|h| h.sample_count())
                .unwrap_or(0)
        }
    }

    /// Returns true if this session is currently acquiring.
    pub fn is_running(&self) -> bool {
        matches!(self.acquisition_state(), AcquisitionState::Running)
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    pub registry: DriverRegistry,
    pub scan_results: Vec<ScanResult>,
    pub scan_error: Option<String>,
    pub connect_error: Option<String>,

    /// All open sessions, keyed by id.
    pub sessions: HashMap<SessionId, SessionState>,
    /// The currently active (visible) session tab.
    pub active_session: SessionId,
    next_session_id: u64,

    /// Executor for background acquisition futures (non-WASM, non-native tests).
    #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
    #[allow(dead_code)]
    pub pool: LocalPool,
    #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
    #[allow(dead_code)]
    pub spawner: LocalSpawner,
    /// Tokio LocalSet for native builds (proper I/O integration).
    #[cfg(feature = "native")]
    pub local_set: LocalSet,

    // Deferred actions.
    pub pending_connect: Option<ScanResult>,
    pub pending_disconnect: Option<SessionId>,
    pub pending_start: Option<SessionId>,
    pub pending_stop: Option<SessionId>,
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

        let first_id = SessionId(1);
        let mut state = SessionState::new(first_id, "Session 1");
        state.session = session;
        state.connected_id = state.session.device_ids().into_iter().next();

        // Update label from device if one is present.
        if let Some(label) = state.device_label() {
            state.label = label;
        }

        let mut sessions = HashMap::new();
        sessions.insert(first_id, state);

        Self {
            registry: DriverRegistry::with_default_factories(),
            scan_results: Vec::new(),
            scan_error: None,
            connect_error: None,
            sessions,
            active_session: first_id,
            next_session_id: 2,
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

    // ── Session management ────────────────────────────────────────────────────

    /// Creates a new empty session and makes it the active tab.
    pub fn create_session(&mut self, label: impl Into<String>) -> SessionId {
        let id = SessionId(self.next_session_id);
        self.next_session_id += 1;
        let state = SessionState::new(id, label);
        self.sessions.insert(id, state);
        self.active_session = id;
        id
    }

    /// Closes a session, stopping any running acquisition and disconnecting
    /// the device. If closing the active session, activates the nearest
    /// remaining session or creates a fresh one.
    pub fn close_session(&mut self, id: SessionId) {
        // Stop and disconnect first.
        if let Some(session_state) = self.sessions.get_mut(&id) {
            // Stop acquisition if running.
            if let Some(acq) = session_state.acquisition.as_mut() {
                acq.send_command(AcquisitionCommand::Stop);
                acq.state = AcquisitionState::Stopped;
            }
            // Disconnect: replace session with empty one (drops all handles).
            session_state.session = Session::new();
            session_state.acquisition = None;
            session_state.assigned_device = None;
            session_state.connected_id = None;
        }

        self.sessions.remove(&id);

        // If we closed the active session, pick a new one or create a fresh session.
        if self.active_session == id {
            if let Some(&next_id) = self.sessions.keys().next() {
                self.active_session = next_id;
            } else {
                // No sessions left — create a fresh empty one.
                self.create_session("Untitled");
            }
        }
    }

    /// Assigns a device candidate to a session. Does NOT connect — connection
    /// happens on Play.
    pub fn assign_device_to_session(&mut self, session_id: SessionId, result: ScanResult) {
        if let Some(state) = self.sessions.get_mut(&session_id) {
            state.assigned_device = Some(result.clone());
            // Update the tab label to the driver name.
            state.label = result.driver.clone();
        }
    }

    /// Returns a reference to the active session.
    pub fn active_session_state(&self) -> Option<&SessionState> {
        self.sessions.get(&self.active_session)
    }

    /// Returns a mutable reference to the active session.
    pub fn active_session_state_mut(&mut self) -> Option<&mut SessionState> {
        self.sessions.get_mut(&self.active_session)
    }

    // ── Device connection (legacy, used during Play flow) ──────────────────────

    /// Connect to a device candidate for the active session (blocking).
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
                    if let Some(state) = self.active_session_state_mut() {
                        let id = state.session.add_device(device);
                        state.connected_id = Some(id);
                        // Update label from device info.
                        if let Some(label) = state.device_label() {
                            state.label = label;
                        }
                        self.connect_error = None;
                    }
                }
                Err(e) => self.connect_error = Some(e.to_string()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.pending_connect = Some(result.clone());
        }
    }

    /// Disconnect the device from the given session (synchronous).
    pub fn disconnect_blocking(&mut self, session_id: SessionId) {
        if let Some(state) = self.sessions.get_mut(&session_id) {
            state.acquisition = None;
            state.session = Session::new();
            state.connected_id = None;
        }
    }

    // ── Acquisition control ───────────────────────────────────────────────────

    // ── Acquisition control ───────────────────────────────────────────────────

    /// Start acquisition for the given session.
    /// If the device is not yet connected, connects first, then starts.
    pub fn start_blocking(&mut self, session_id: SessionId) {
        // Check if we need to connect first.
        let need_connect = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.connected_device_id().is_none() && s.assigned_device.is_some());

        if need_connect {
            let assigned = self.sessions.get(&session_id).and_then(|s| s.assigned_device.clone());
            if let Some(result) = assigned {
                self.connect_blocking_with_session(session_id, &result);
            }
        }

        // Now start acquisition. Extract the handle first to avoid double borrow.
        let handle = self.sessions.get_mut(&session_id).and_then(|state| {
            if state.acquisition.is_some() {
                // Re-run: apply config.
                let rate = state.acquisition.as_ref().unwrap().config.sample_rate_hz;
                state.acquisition.as_mut().unwrap().send_command(AcquisitionCommand::SetSampleRate(rate));
                state.acquisition.as_mut().unwrap().reset_traces();
                state.acquisition.as_mut().unwrap().send_command(AcquisitionCommand::Start);
                state.acquisition.as_mut().unwrap().state = AcquisitionState::Running;
                None
            } else {
                state.connected_device_id().and_then(|did| state.session.remove(&did))
            }
        });

        if let Some(handle) = handle {
            let acq = self.spawn_acquisition(handle);
            if let Some(state) = self.sessions.get_mut(&session_id) {
                state.acquisition = Some(acq);
            }
        }
    }

    /// Stop acquisition for the given session.
    pub fn stop_blocking(&mut self, session_id: SessionId) {
        let state = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => return,
        };
        if let Some(acq) = state.acquisition.as_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        } else if let Some(handle) = state.session.device_ids().first().and_then(|id| state.session.device_mut(id)) {
            #[cfg(not(target_arch = "wasm32"))]
            let _ = futures::executor::block_on(handle.apply(AcquisitionCommand::Stop));
        }
    }

    /// Connect a device to a specific session (internal helper for start flow).
    pub(crate) fn connect_blocking_with_session(&mut self, session_id: SessionId, result: &ScanResult) {
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
                    if let Some(state) = self.sessions.get_mut(&session_id) {
                        let id = state.session.add_device(device);
                        state.connected_id = Some(id);
                        if let Some(label) = state.device_label() {
                            state.label = label;
                        }
                        self.connect_error = None;
                    }
                }
                Err(e) => self.connect_error = Some(e.to_string()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // On WASM, queue the connect via pending_connect.
            // The session id won't be needed since WASM uses the active session.
            self.pending_connect = Some(result.clone());
        }
    }

    pub fn apply_pending_actions(&mut self) {
        // Handle pending connect (used by WASM path and legacy callers).
        if let Some(result) = self.pending_connect.take() {
            let session_id = self.active_session;
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.connect_blocking_with_session(session_id, &result);
            }
            #[cfg(target_arch = "wasm32")]
            {
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

        // Handle pending disconnect (session close).
        if let Some(session_id) = self.pending_disconnect.take() {
            self.close_session(session_id);
        }

        // Handle pending start.
        if let Some(session_id) = self.pending_start.take() {
            self.apply_start(session_id);
        }

        // Handle pending stop.
        if let Some(session_id) = self.pending_stop.take() {
            self.apply_stop(session_id);
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
                self.pending_wasm_scan = Some(rx);
            }
        }
        // Check for completed WASM connect.
        #[cfg(target_arch = "wasm32")]
        if let Some(mut rx) = self.pending_wasm_connect.take() {
            if let Ok(Some(result)) = rx.try_recv() {
                Self::apply_connect_result(
                    result,
                    &mut self.sessions,
                    self.active_session,
                    &mut self.connect_error,
                );
            } else {
                self.pending_wasm_connect = Some(rx);
            }
        }
    }

    /// Check if connect is needed (device assigned but not connected), connect,
    /// then start acquisition. Used by pending_start.
    fn apply_start(&mut self, session_id: SessionId) {
        let need_connect = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.connected_device_id().is_none() && s.assigned_device.is_some());

        if need_connect {
            let assigned = self.sessions.get(&session_id).and_then(|s| s.assigned_device.clone());
            if let Some(result) = assigned {
                #[cfg(not(target_arch = "wasm32"))]
                self.connect_blocking_with_session(session_id, &result);
                #[cfg(target_arch = "wasm32")]
                {
                    self.pending_connect = Some(result);
                    self.pending_start = Some(session_id);
                    return;
                }
            }
        }

        // Extract handle first to avoid double borrow.
        let handle = self.sessions.get_mut(&session_id).and_then(|state| {
            if state.acquisition.is_some() {
                let rate = state.acquisition.as_ref().unwrap().config.sample_rate_hz;
                state.acquisition.as_mut().unwrap().send_command(AcquisitionCommand::SetSampleRate(rate));
                state.acquisition.as_mut().unwrap().reset_traces();
                state.acquisition.as_mut().unwrap().send_command(AcquisitionCommand::Start);
                state.acquisition.as_mut().unwrap().state = AcquisitionState::Running;
                None
            } else {
                state.connected_device_id().and_then(|did| state.session.remove(&did))
            }
        });

        if let Some(handle) = handle {
            let acq = self.spawn_acquisition(handle);
            if let Some(state) = self.sessions.get_mut(&session_id) {
                state.acquisition = Some(acq);
            }
        }
    }

    fn apply_stop(&mut self, session_id: SessionId) {
        let state = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => return,
        };
        if let Some(acq) = state.acquisition.as_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        } else if let Some(handle) = state.session.device_ids().first().and_then(|id| state.session.device_mut(id)) {
            #[cfg(not(target_arch = "wasm32"))]
            let _ = futures::executor::block_on(handle.apply(AcquisitionCommand::Stop));
        }
    }

    /// Apply a connect result (shared by sync and async paths).
    fn apply_connect_result(
        result: Result<Box<dyn rb_device::Device>, impl ToString>,
        sessions: &mut HashMap<SessionId, SessionState>,
        active_session: SessionId,
        connect_error: &mut Option<String>,
    ) {
        match result {
            Ok(device) => {
                if let Some(state) = sessions.get_mut(&active_session) {
                    let id = state.session.add_device(device);
                    state.connected_id = Some(id);
                    if let Some(label) = state.device_label() {
                        state.label = label;
                    }
                    *connect_error = None;
                }
            }
            Err(e) => *connect_error = Some(e.to_string()),
        }
    }

    /// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
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

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Returns the device label (vendor/model) for a connected device.
    pub fn device_label(&self, id: &DeviceId) -> String {
        for state in self.sessions.values() {
            if let Some(handle) = state.session.device(id) {
                let info = handle.device().info();
                return format!("{}/{}", info.vendor, info.model);
            }
        }
        id.to_string()
    }

    /// Returns the acquisition state for a device, searching across all sessions.
    pub fn device_state(&self, id: &DeviceId) -> Option<AcquisitionState> {
        for state in self.sessions.values() {
            if state.connected_device_id().as_ref() == Some(id) {
                if let Some(acq) = &state.acquisition {
                    return Some(acq.state().clone());
                }
            }
            if let Some(handle) = state.session.device(id) {
                return Some(handle.state().clone());
            }
        }
        None
    }

    /// Returns a reference to the device handle for a device, searching across all sessions.
    pub fn device_handle(&self, id: &DeviceId) -> Option<&DeviceHandle> {
        for state in self.sessions.values() {
            if let Some(handle) = state.session.device(id) {
                return Some(handle);
            }
        }
        None
    }

    /// Returns a reference to the acquisition for a session.
    pub fn acq_for_session(&self, session_id: SessionId) -> Option<&DeviceAcquisition> {
        self.sessions.get(&session_id).and_then(|s| s.acquisition.as_ref())
    }

    /// Returns a mutable reference to the acquisition for a session.
    pub fn acq_for_session_mut(&mut self, session_id: SessionId) -> Option<&mut DeviceAcquisition> {
        self.sessions.get_mut(&session_id).and_then(|s| s.acquisition.as_mut())
    }

    /// Returns a reference to the device handle for a session.
    pub fn handle_for_session(&self, session_id: SessionId) -> Option<&DeviceHandle> {
        let state = self.sessions.get(&session_id)?;
        state.connected_device_id().and_then(|id| state.session.device(&id))
    }

    /// Returns the connected device id for a session.
    pub fn device_id_for_session(&self, session_id: SessionId) -> Option<DeviceId> {
        self.sessions.get(&session_id).and_then(|s| s.connected_device_id())
    }

    /// Returns a mutable reference to the acquisition for a device.
    pub fn acquisition_for_device_mut(&mut self, device_id: &DeviceId) -> Option<&mut DeviceAcquisition> {
        for state in self.sessions.values_mut() {
            if state.connected_device_id().as_ref() == Some(device_id) {
                return state.acquisition.as_mut();
            }
        }
        None
    }

    /// Returns a reference to the session state that owns the given device.
    pub fn session_for_device(&self, device_id: &DeviceId) -> Option<&SessionState> {
        self.sessions.values().find(|s| s.connected_device_id().as_ref() == Some(device_id))
    }

    /// Returns a mutable reference to the session state that owns the given device.
    pub fn session_for_device_mut(&mut self, device_id: &DeviceId) -> Option<&mut SessionState> {
        self.sessions.values_mut().find(|s| s.connected_device_id().as_ref() == Some(device_id))
    }

    /// Returns the sample count for a device, searching across all sessions.
    pub fn device_sample_count(&self, id: &DeviceId) -> usize {
        for state in self.sessions.values() {
            if let Some(acq) = &state.acquisition {
                if state.connected_device_id().as_ref() == Some(id) {
                    return acq.sample_count();
                }
            }
            if let Some(handle) = state.session.device(id) {
                return handle.sample_count();
            }
        }
        0
    }

    /// Returns the acquisition state of the active session.
    pub fn active_session_acquisition_state(&self) -> AcquisitionState {
        self.active_session_state()
            .map(|s| s.acquisition_state())
            .unwrap_or(AcquisitionState::Idle)
    }

    /// Returns all connected device ids from all sessions.
    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        let mut ids = Vec::new();
        for state in self.sessions.values() {
            ids.extend(state.session.device_ids());
            if state.acquisition.is_some() {
                // Include device ids from acquisitions.
                if let Some(id) = state.connected_device_id() {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
        }
        ids
    }

    /// Returns the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
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
                let _ = request_supported_usb_devices().await;
                let result = registry.scan_all().await.map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
            self.pending_wasm_scan = Some(rx);
        }
    }

    /// Run the executor for one tick.
    pub fn pump_once(&mut self) {
        #[cfg(target_arch = "wasm32")]
        return;

        #[cfg(feature = "native")]
        {
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

    /// Drains acquisition data for the **active session only** into local
    /// stores. Returns true if any new data arrived.
    /// Inactive sessions are left untouched — their data stays in the mpsc
    /// channel until the user switches to their tab.
    pub fn drain_all(&mut self) -> bool {
        let Some(state) = self.sessions.get_mut(&self.active_session) else {
            return false;
        };
        let Some(acq) = state.acquisition.as_mut() else {
            return false;
        };
        let before = acq.sample_count();
        acq.drain();
        acq.sample_count() > before
    }

    /// Returns true if any acquisition is currently running.
    pub fn any_running(&self) -> bool {
        self.sessions.values().any(|s| s.is_running())
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
    fn app_initializes_with_one_empty_session() {
        let app = AppState::default();
        assert_eq!(app.sessions.len(), 1);
        let active = app.active_session_state().unwrap();
        assert!(active.session.is_empty());
        assert!(active.acquisition.is_none());
        assert!(app.scan_results.is_empty());
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
    fn connect_adds_device_to_active_session() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();
        let active = app.active_session_state().unwrap();
        assert_eq!(active.session.len(), 1);
    }

    #[test]
    fn disconnect_removes_device_from_session() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();

        let session_id = app.active_session;
        app.pending_disconnect = Some(session_id);
        app.apply_pending_actions();

        // The session should still exist (close creates a fresh one if last)
        assert_eq!(app.sessions.len(), 1);
        let active = app.active_session_state().unwrap();
        assert!(active.session.is_empty());
        assert!(active.acquisition.is_none());
    }

    #[test]
    fn start_spawns_and_pumps_samples() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let session_id = app.active_session;

        // Assign and connect the device.
        app.assign_device_to_session(session_id, demo.clone());
        app.pending_connect = Some(demo);
        app.apply_pending_actions();

        // Start acquisition.
        app.pending_start = Some(session_id);
        app.apply_pending_actions();

        let active = app.active_session_state().unwrap();
        assert!(active.acquisition.is_some());

        // Drive the pool repeatedly.
        for _ in 0..10 {
            app.pump_once();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Drain data.
        if let Some(active) = app.sessions.get_mut(&session_id) {
            if let Some(acq) = active.acquisition.as_mut() {
                acq.drain();
            }
        }

        let active = app.active_session_state().unwrap();
        assert!(active.sample_count() > 0, "expected samples after pump, got {}", active.sample_count());
    }

    #[test]
    fn create_and_close_sessions() {
        let mut app = AppState::default();
        let initial = app.active_session;
        assert_eq!(app.sessions.len(), 1);

        // Create a second session.
        let id2 = app.create_session("Test 2");
        assert_eq!(app.active_session, id2);
        assert_eq!(app.sessions.len(), 2);

        // Close the active session (Test 2), should fall back to initial.
        app.close_session(id2);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.active_session, initial);
    }

    fn scan_for_demo(app: &AppState) -> ScanResult {
        block_on(app.registry.scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
