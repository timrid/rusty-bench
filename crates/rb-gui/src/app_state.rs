//! Application state: the top-level orchestrator.
//!
//! [`AppState`] owns the [`DeviceManager`], all [`SessionState`]s, and the
//! executor for background acquisition futures. It is framework-agnostic and
//! testable without a display.

use std::collections::HashMap;

use futures::channel::mpsc;
use rb_core::{
    run_acquisition, AcquisitionCommand, AcquisitionState, DeviceHandle,
};
use rb_device::DeviceId;

use crate::device_acquisition::{AcquisitionConfig, DeviceAcquisition};
use crate::device_manager::DeviceManager;
use crate::session_state::{SessionId, SessionState};

// Executors: tokio LocalSet when native feature is enabled (non-wasm),
// LocalPool otherwise (non-wasm tests only; WASM uses wasm-bindgen-futures).
#[cfg(not(any(feature = "native", target_arch = "wasm32")))]
use {futures::executor::LocalPool, futures::executor::LocalSpawner, futures::task::LocalSpawnExt};
#[cfg(feature = "native")]
use tokio::task::LocalSet;

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    /// Program-level device manager (scan, connect, handle pool).
    pub device_manager: DeviceManager,

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
    pub pending_disconnect: Option<SessionId>,
    pub pending_start: Option<SessionId>,
    pub pending_stop: Option<SessionId>,
}

impl AppState {
    /// Creates an app state with a device manager that already has a connected
    /// device (for integration tests that inject a mock device).
    #[must_use]
    pub fn from_device_manager(device_manager: DeviceManager) -> Self {
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

        // If a device is already connected, assign it to the first session.
        let device_ids = device_manager.connected_device_ids();
        if let Some(did) = device_ids.into_iter().next() {
            state.assigned_device_id = Some(did.clone());
            if let Some(label) = device_manager.device_label(&did) {
                state.label = label.to_string();
            }
        }

        let mut sessions = HashMap::new();
        sessions.insert(first_id, state);

        Self {
            device_manager,
            sessions,
            active_session: first_id,
            next_session_id: 2,
            #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
            pool,
            #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
            spawner,
            #[cfg(feature = "native")]
            local_set,
            pending_disconnect: None,
            pending_start: None,
            pending_stop: None,
        }
    }

    pub fn new() -> Self {
        Self::from_device_manager(DeviceManager::new())
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

    /// Closes a session, stopping any running acquisition.
    /// If closing the active session, activates the nearest remaining session
    /// or creates a fresh one. Does NOT disconnect the device — the device
    /// connection is program-level and may be used by other sessions.
    pub fn close_session(&mut self, id: SessionId) {
        // Stop acquisition if running.
        if let Some(session_state) = self.sessions.get_mut(&id) {
            if let Some(acq) = session_state.acquisition.as_mut() {
                acq.send_command(AcquisitionCommand::Stop);
                acq.state = AcquisitionState::Stopped;
            }
            session_state.acquisition = None;
            session_state.assigned_device_id = None;
        }

        self.sessions.remove(&id);

        // If we closed the active session, pick a new one or create a fresh session.
        if self.active_session == id {
            if let Some(&next_id) = self.sessions.keys().next() {
                self.active_session = next_id;
            } else {
                self.create_session("Untitled");
            }
        }
    }

    /// Assigns a device (by [`DeviceId`]) to a session and updates the tab label.
    /// Also builds the initial [`AcquisitionConfig`] from the device's capabilities.
    pub fn assign_device_to_session(&mut self, session_id: SessionId, device_id: DeviceId) {
        if let Some(state) = self.sessions.get_mut(&session_id) {
            state.assigned_device_id = Some(device_id.clone());
            // Build config from device capabilities.
            if let Some(handle) = self.device_manager.device_handle(&device_id) {
                state.acquisition_config =
                    AcquisitionConfig::from_device(handle.device());
            }
            // Update label from device info.
            if let Some(label) = self.device_manager.device_label(&device_id) {
                state.label = label.to_string();
            }
        }
    }

    /// Connects to a device candidate and assigns it to the given session.
    /// Returns the [`DeviceId`] on success.
    pub fn connect_and_assign(
        &mut self,
        session_id: SessionId,
        scan_result: &rb_core::ScanResult,
    ) -> Result<DeviceId, rb_core::SessionError> {
        let device_id = self.device_manager.connect_blocking(scan_result)?;
        self.assign_device_to_session(session_id, device_id.clone());
        Ok(device_id)
    }

    /// Returns a reference to the active session.
    pub fn active_session_state(&self) -> Option<&SessionState> {
        self.sessions.get(&self.active_session)
    }

    /// Returns a mutable reference to the active session.
    pub fn active_session_state_mut(&mut self) -> Option<&mut SessionState> {
        self.sessions.get_mut(&self.active_session)
    }

    // ── Device connection (delegates to DeviceManager) ────────────────────────

    /// Triggers a device scan.
    pub fn trigger_scan(&mut self) {
        self.device_manager.trigger_scan();
    }

    /// Disconnect the device from the given session and the program.
    pub fn disconnect_blocking(&mut self, session_id: SessionId) {
        if let Some(state) = self.sessions.get_mut(&session_id) {
            state.acquisition = None;
            if let Some(ref did) = state.assigned_device_id.clone() {
                self.device_manager.disconnect(did);
            }
            state.assigned_device_id = None;
        }
    }

    // ── Acquisition control ───────────────────────────────────────────────────

    /// Start acquisition for the given session.
    /// Takes the device handle from [`DeviceManager`] and spawns the
    /// acquisition future. The handle is returned when acquisition stops.
    pub fn start_blocking(&mut self, session_id: SessionId) {
        // Check if we need to connect first (device assigned but not connected).
        let need_connect = {
            let session = self.sessions.get(&session_id);
            session.is_some_and(|s| {
                s.assigned_device_id
                    .as_ref()
                    .is_some_and(|did| !self.device_manager.is_connected(did))
            })
        };

        if need_connect {
            // Can't auto-connect here without a ScanResult.
            // The UI should have connected via the dropdown first.
            return;
        }

        // Check if already acquiring — re-run.
        let already_acquiring = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.acquisition.is_some());

        if already_acquiring {
            if let Some(state) = self.sessions.get_mut(&session_id) {
                if let Some(acq) = state.acquisition.as_mut() {
                    let rate = acq.config.sample_rate_hz;
                    acq.send_command(AcquisitionCommand::SetSampleRate(rate));
                    acq.reset_traces();
                    acq.send_command(AcquisitionCommand::Start);
                    acq.state = AcquisitionState::Running;
                }
            }
            return;
        }

        // Take the handle from DeviceManager.
        let device_id = self
            .sessions
            .get(&session_id)
            .and_then(|s| s.assigned_device_id.clone());

        let handle = device_id
            .as_ref()
            .and_then(|did| self.device_manager.take_handle(did));

        if let Some(handle) = handle {
            let device_id = device_id.unwrap();
            let acq = self.spawn_acquisition(handle, device_id);
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
        }
        // If the device handle is in DeviceManager (not borrowed), it's already idle.
    }

    // ── Pending actions ───────────────────────────────────────────────────────

    pub fn apply_pending_actions(&mut self) {
        // Let DeviceManager process its own pending actions (WASM scan/connect).
        self.device_manager.apply_pending_actions();
        // Collect handles returned from completed acquisitions.
        self.device_manager.collect_returns();

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
    }

    /// Check if connect is needed, connect, then start acquisition.
    fn apply_start(&mut self, session_id: SessionId) {
        let need_connect = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| {
                s.assigned_device_id
                    .as_ref()
                    .is_some_and(|did| !self.device_manager.is_connected(did))
            });

        if need_connect {
            // Without a ScanResult we can't auto-connect.
            return;
        }

        let already_acquiring = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.acquisition.is_some());

        if already_acquiring {
            if let Some(state) = self.sessions.get_mut(&session_id) {
                if let Some(acq) = state.acquisition.as_mut() {
                    let rate = acq.config.sample_rate_hz;
                    acq.send_command(AcquisitionCommand::SetSampleRate(rate));
                    acq.reset_traces();
                    acq.send_command(AcquisitionCommand::Start);
                    acq.state = AcquisitionState::Running;
                }
            }
            return;
        }

        let device_id = self
            .sessions
            .get(&session_id)
            .and_then(|s| s.assigned_device_id.clone());

        let handle = device_id
            .as_ref()
            .and_then(|did| self.device_manager.take_handle(did));

        if let Some(handle) = handle {
            let device_id = device_id.unwrap();
            let acq = self.spawn_acquisition(handle, device_id);
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
        }
    }

    /// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
    /// The device handle is returned to [`DeviceManager`] when the future completes.
    /// Traces are built from the active session's [`AcquisitionConfig`].
    #[allow(unused_mut)]
    pub fn spawn_acquisition(
        &mut self,
        mut handle: DeviceHandle,
        device_id: DeviceId,
    ) -> DeviceAcquisition {
        // Read config from the session that owns this device.
        let config = self
            .sessions
            .values()
            .find(|s| s.assigned_device_id.as_ref() == Some(&device_id))
            .map(|s| s.acquisition_config.clone())
            .unwrap_or_default();

        // Rebuild handle traces to match config (sample rate, channel layout).
        config.apply_to_handle(&mut handle);

        // Build traces for the GUI display.
        let (analog, digital) = config.build_traces();

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

            // When the acquisition future completes, return the handle to DeviceManager.
            let (return_tx, return_rx) = futures::channel::oneshot::channel();
            self.device_manager.register_pending_return(return_rx);

            let fut = run_acquisition(handle, cmd_rx, Some(data_tx));
            #[cfg(feature = "native")]
            {
                self.local_set.spawn_local(async move {
                    let handle = fut.await;
                    let _ = return_tx.send((device_id, handle));
                });
            }
            #[cfg(not(feature = "native"))]
            {
                self.spawner
                    .spawn_local(async move {
                        let handle = fut.await;
                        let _ = return_tx.send((device_id, handle));
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
        self.device_manager
            .device_label(id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.to_string())
    }

    /// Returns the acquisition state for a device, searching across all sessions.
    pub fn device_state(&self, id: &DeviceId) -> Option<AcquisitionState> {
        for state in self.sessions.values() {
            if state.assigned_device_id.as_ref() == Some(id) {
                if let Some(acq) = &state.acquisition {
                    return Some(acq.state().clone());
                }
            }
        }
        // Fall back to DeviceManager's handle state.
        self.device_manager
            .device_handle(id)
            .map(|h| h.state().clone())
    }

    /// Returns a reference to the device handle for a device.
    pub fn device_handle(&self, id: &DeviceId) -> Option<&DeviceHandle> {
        self.device_manager.device_handle(id)
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
        state
            .assigned_device_id
            .as_ref()
            .and_then(|did| self.device_manager.device_handle(did))
    }

    /// Returns the connected device id for a session.
    pub fn device_id_for_session(&self, session_id: SessionId) -> Option<DeviceId> {
        self.sessions
            .get(&session_id)
            .and_then(|s| s.assigned_device_id.clone())
            .filter(|did| self.device_manager.is_connected(did))
    }

    /// Returns the acquisition state of the active session.
    pub fn active_session_acquisition_state(&self) -> AcquisitionState {
        let session = match self.active_session_state() {
            Some(s) => s,
            None => return AcquisitionState::Idle,
        };
        if let Some(acq) = &session.acquisition {
            acq.state().clone()
        } else {
            session
                .assigned_device_id
                .as_ref()
                .and_then(|did| self.device_manager.device_handle(did))
                .map(|h| h.state().clone())
                .unwrap_or(AcquisitionState::Idle)
        }
    }

    /// Returns all connected device ids from DeviceManager.
    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        self.device_manager.connected_device_ids()
    }

    /// Returns the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    // ── Executor ──────────────────────────────────────────────────────────────

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
        self.device_manager.pending_wasm_scan.is_some()
            || self.device_manager.pending_wasm_connect.is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn wasm_pending(&self) -> bool {
        false
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::from_device_manager(DeviceManager::new())
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
pub(crate) async fn request_supported_usb_devices() {
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
        assert!(active.assigned_device_id.is_none());
        assert!(active.acquisition.is_none());
        assert!(app.device_manager.scan_results.is_empty());
    }

    #[test]
    fn scan_populates_results() {
        let mut app = AppState::default();
        let results = block_on(app.device_manager.registry().scan_all()).unwrap();
        app.device_manager.scan_results = results;
        assert!(!app.device_manager.scan_results.is_empty());
        assert!(app.device_manager.scan_results.iter().any(|r| r.driver == "demo"));
    }

    #[test]
    fn connect_adds_device_to_active_session() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let session_id = app.active_session;
        let did = app.connect_and_assign(session_id, &demo).unwrap();
        assert!(app.device_manager.is_connected(&did));
        let active = app.active_session_state().unwrap();
        assert_eq!(active.assigned_device_id, Some(did));
    }

    #[test]
    fn disconnect_removes_device_from_session() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let session_id = app.active_session;
        let did = app.connect_and_assign(session_id, &demo).unwrap();

        app.disconnect_blocking(session_id);
        let active = app.active_session_state().unwrap();
        assert!(active.assigned_device_id.is_none());
        assert!(active.acquisition.is_none());
        assert!(!app.device_manager.is_connected(&did));
    }

    #[test]
    fn start_spawns_and_pumps_samples() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let session_id = app.active_session;

        // Connect and assign the device.
        let _did = app.connect_and_assign(session_id, &demo).unwrap();

        // Start acquisition.
        app.start_blocking(session_id);

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
        if let Some(acq) = active.acquisition.as_ref() {
            assert!(acq.sample_count() > 0, "expected samples after pump, got {}", acq.sample_count());
        }
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

    fn scan_for_demo(app: &AppState) -> rb_core::ScanResult {
        block_on(app.device_manager.registry().scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
