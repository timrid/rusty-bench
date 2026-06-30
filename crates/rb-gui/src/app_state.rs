//! Application state: the top-level orchestrator.
//!
//! [`AppState`] owns the [`DeviceManager`], all [`TabState`]s, and the
//! executor for background acquisition futures. It is framework-agnostic and
//! testable without a display.

use std::collections::HashMap;

use futures::channel::mpsc;
use rb_core::{
    run_acquisition, AcquisitionCommand, AcquisitionState, DeviceHandle,
};
use rb_device::DeviceId;

use crate::logic_analyzer::acquisition::{AcquisitionConfig, DeviceAcquisition};
use crate::tab_content::{LogicAnalyzerContent, TabContent};
use crate::device_manager::DeviceManager;
use crate::tab_state::{TabId, TabSource, TabState};

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

    /// All open tabs, keyed by id.
    pub tabs: HashMap<TabId, TabState>,
    /// The currently active (visible) tab.
    pub active_tab: TabId,
    next_tab_id: u64,

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
    pub pending_disconnect: Option<TabId>,
    pub pending_start: Option<TabId>,
    pub pending_stop: Option<TabId>,
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

        let first_id = TabId(1);
        let mut tab = TabState::new(first_id, "Session 1");

        // If a device is already connected, assign it to the first tab.
        let device_ids = device_manager.connected_device_ids();
        if let Some(did) = device_ids.into_iter().next() {
            tab.source = TabSource::Device(did.clone());
            tab.content = Some(TabContent::LogicAnalyzer(LogicAnalyzerContent::default()));
            if let Some(label) = device_manager.device_label(&did) {
                tab.label = label.to_string();
            }
        }

        let mut tabs = HashMap::new();
        tabs.insert(first_id, tab);

        Self {
            device_manager,
            tabs,
            active_tab: first_id,
            next_tab_id: 2,
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

    // ── Tab management ────────────────────────────────────────────────────────

    /// Creates a new empty tab and makes it the active tab.
    pub fn create_tab(&mut self, label: impl Into<String>) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let tab = TabState::new(id, label);
        self.tabs.insert(id, tab);
        self.active_tab = id;
        id
    }

    /// Closes a tab, stopping any running acquisition.
    /// If closing the active tab, activates the nearest remaining tab
    /// or creates a fresh one. Does NOT disconnect the device — the device
    /// connection is program-level and may be used by other tabs.
    pub fn close_tab(&mut self, id: TabId) {
        // Stop acquisition if running.
        if let Some(tab) = self.tabs.get_mut(&id) {
            if let Some(acq) = tab.acquisition_mut() {
                acq.send_command(AcquisitionCommand::Stop);
                acq.state = AcquisitionState::Stopped;
            }
            tab.set_acquisition(None);
            tab.set_assigned_device_id(None);
        }

        self.tabs.remove(&id);

        // If we closed the active tab, pick a new one or create a fresh tab.
        if self.active_tab == id {
            if let Some(&next_id) = self.tabs.keys().next() {
                self.active_tab = next_id;
            } else {
                self.create_tab("Untitled");
            }
        }
    }

    /// Assigns a device (by [`DeviceId`]) to a tab and updates the tab label.
    /// Also builds the initial [`AcquisitionConfig`] from the device's capabilities
    /// and sets the [`TabContent`] to [`TabContent::LogicAnalyzer`].
    pub fn assign_device_to_tab(&mut self, tab_id: TabId, device_id: DeviceId) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.source = TabSource::Device(device_id.clone());
            // Build config from device capabilities.
            if let Some(handle) = self.device_manager.device_handle(&device_id) {
                let config = AcquisitionConfig::from_device(handle.device());
                tab.content = Some(TabContent::LogicAnalyzer(LogicAnalyzerContent {
                    acquisition_config: config,
                    ..LogicAnalyzerContent::default()
                }));
            } else {
                tab.content = Some(TabContent::default());
            }
            // Update label from device info.
            if let Some(label) = self.device_manager.device_label(&device_id) {
                tab.label = label.to_string();
            }
        }
    }

    /// Connects to a device candidate and assigns it to the given tab.
    /// Returns the [`DeviceId`] on success.
    pub fn connect_and_assign(
        &mut self,
        tab_id: TabId,
        scan_result: &rb_core::ScanResult,
    ) -> Result<DeviceId, rb_core::SessionError> {
        let device_id = self.device_manager.connect_blocking(scan_result)?;
        self.assign_device_to_tab(tab_id, device_id.clone());
        Ok(device_id)
    }

    /// Returns a reference to the active tab.
    pub fn active_tab_state(&self) -> Option<&TabState> {
        self.tabs.get(&self.active_tab)
    }

    /// Returns a mutable reference to the active tab.
    pub fn active_tab_state_mut(&mut self) -> Option<&mut TabState> {
        self.tabs.get_mut(&self.active_tab)
    }

    // ── Device connection (delegates to DeviceManager) ────────────────────────

    /// Triggers a device scan.
    pub fn trigger_scan(&mut self) {
        self.device_manager.trigger_scan();
    }

    /// Disconnect the device from the given tab and the program.
    pub fn disconnect_blocking(&mut self, tab_id: TabId) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.set_acquisition(None);
            if let Some(did) = tab.assigned_device_id().cloned() {
                self.device_manager.disconnect(&did);
            }
            tab.set_assigned_device_id(None);
        }
    }

    // ── Acquisition control ───────────────────────────────────────────────────

    /// Start acquisition for the given tab.
    /// Takes the device handle from [`DeviceManager`] and spawns the
    /// acquisition future. The handle is returned when acquisition stops.
    pub fn start_blocking(&mut self, tab_id: TabId) {
        // Check if we need to connect first (device assigned but not connected).
        let need_connect = {
            let tab = self.tabs.get(&tab_id);
            tab.is_some_and(|t| {
                t.assigned_device_id()
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
            .tabs
            .get(&tab_id)
            .is_some_and(|t| t.acquisition().is_some());

        if already_acquiring {
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                if let Some(acq) = tab.acquisition_mut() {
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
            .tabs
            .get(&tab_id)
            .and_then(|t| t.assigned_device_id().cloned());

        let handle = device_id
            .as_ref()
            .and_then(|did| self.device_manager.take_handle(did));

        if let Some(handle) = handle {
            let device_id = device_id.unwrap();
            let acq = self.spawn_acquisition(handle, device_id);
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                tab.set_acquisition(Some(acq));
            }
        }
    }

    /// Stop acquisition for the given tab.
    pub fn stop_blocking(&mut self, tab_id: TabId) {
        let tab = match self.tabs.get_mut(&tab_id) {
            Some(t) => t,
            None => return,
        };
        if let Some(acq) = tab.acquisition_mut() {
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

        // Handle pending disconnect (tab close).
        if let Some(tab_id) = self.pending_disconnect.take() {
            self.close_tab(tab_id);
        }

        // Handle pending start.
        if let Some(tab_id) = self.pending_start.take() {
            self.apply_start(tab_id);
        }

        // Handle pending stop.
        if let Some(tab_id) = self.pending_stop.take() {
            self.apply_stop(tab_id);
        }
    }

    /// Check if connect is needed, connect, then start acquisition.
    fn apply_start(&mut self, tab_id: TabId) {
        let need_connect = self
            .tabs
            .get(&tab_id)
            .is_some_and(|t| {
                t.assigned_device_id()
                    .is_some_and(|did| !self.device_manager.is_connected(did))
            });

        if need_connect {
            // Without a ScanResult we can't auto-connect.
            return;
        }

        let already_acquiring = self
            .tabs
            .get(&tab_id)
            .is_some_and(|t| t.acquisition().is_some());

        if already_acquiring {
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                if let Some(acq) = tab.acquisition_mut() {
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
            .tabs
            .get(&tab_id)
            .and_then(|t| t.assigned_device_id().cloned());

        let handle = device_id
            .as_ref()
            .and_then(|did| self.device_manager.take_handle(did));

        if let Some(handle) = handle {
            let device_id = device_id.unwrap();
            let acq = self.spawn_acquisition(handle, device_id);
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                tab.set_acquisition(Some(acq));
            }
        }
    }

    fn apply_stop(&mut self, tab_id: TabId) {
        let tab = match self.tabs.get_mut(&tab_id) {
            Some(t) => t,
            None => return,
        };
        if let Some(acq) = tab.acquisition_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        }
    }

    /// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
    /// The device handle is returned to [`DeviceManager`] when the future completes.
    /// Traces are built from the active tab's [`AcquisitionConfig`].
    #[allow(unused_mut)]
    pub fn spawn_acquisition(
        &mut self,
        mut handle: DeviceHandle,
        device_id: DeviceId,
    ) -> DeviceAcquisition {
        // Read config from the tab that owns this device.
        let config = self
            .tabs
            .values()
            .find(|t| t.assigned_device_id() == Some(&device_id))
            .map(|t| t.acquisition_config().clone())
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

    /// Returns the acquisition state for a device, searching across all tabs.
    pub fn device_state(&self, id: &DeviceId) -> Option<AcquisitionState> {
        for tab in self.tabs.values() {
            if tab.assigned_device_id() == Some(id) {
                if let Some(acq) = tab.acquisition() {
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

    /// Returns a reference to the acquisition for a tab.
    pub fn acq_for_tab(&self, tab_id: TabId) -> Option<&DeviceAcquisition> {
        self.tabs.get(&tab_id).and_then(|t| t.acquisition())
    }

    /// Returns a mutable reference to the acquisition for a tab.
    pub fn acq_for_tab_mut(&mut self, tab_id: TabId) -> Option<&mut DeviceAcquisition> {
        self.tabs.get_mut(&tab_id).and_then(|t| t.acquisition_mut())
    }

    /// Returns a reference to the device handle for a tab.
    pub fn handle_for_tab(&self, tab_id: TabId) -> Option<&DeviceHandle> {
        let tab = self.tabs.get(&tab_id)?;
        tab.assigned_device_id()
            .and_then(|did| self.device_manager.device_handle(did))
    }

    /// Returns the connected device id for a tab.
    pub fn device_id_for_tab(&self, tab_id: TabId) -> Option<DeviceId> {
        self.tabs
            .get(&tab_id)
            .and_then(|t| t.assigned_device_id().cloned())
            .filter(|did| self.device_manager.is_connected(did))
    }

    /// Returns the acquisition state of the active tab.
    pub fn active_tab_acquisition_state(&self) -> AcquisitionState {
        let tab = match self.active_tab_state() {
            Some(t) => t,
            None => return AcquisitionState::Idle,
        };
        if let Some(acq) = tab.acquisition() {
            acq.state().clone()
        } else {
            tab.assigned_device_id()
                .and_then(|did| self.device_manager.device_handle(did))
                .map(|h| h.state().clone())
                .unwrap_or(AcquisitionState::Idle)
        }
    }

    /// Returns all connected device ids from DeviceManager.
    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        self.device_manager.connected_device_ids()
    }

    /// Returns the number of open tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
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

    /// Drains acquisition data for the **active tab only** into local
    /// stores. Returns true if any new data arrived.
    pub fn drain_all(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(&self.active_tab) else {
            return false;
        };
        let Some(acq) = tab.acquisition_mut() else {
            return false;
        };
        let before = acq.sample_count();
        acq.drain();
        acq.sample_count() > before
    }

    /// Returns true if any acquisition is currently running.
    pub fn any_running(&self) -> bool {
        self.tabs.values().any(|t| t.is_running())
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
    fn app_initializes_with_one_empty_tab() {
        let app = AppState::default();
        assert_eq!(app.tabs.len(), 1);
        let active = app.active_tab_state().unwrap();
        assert!(active.assigned_device_id().is_none());
        assert!(active.acquisition().is_none());
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
    fn connect_adds_device_to_active_tab() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let tab_id = app.active_tab;
        let did = app.connect_and_assign(tab_id, &demo).unwrap();
        assert!(app.device_manager.is_connected(&did));
        let active = app.active_tab_state().unwrap();
        assert_eq!(active.assigned_device_id().cloned(), Some(did));
    }

    #[test]
    fn disconnect_removes_device_from_tab() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let tab_id = app.active_tab;
        let did = app.connect_and_assign(tab_id, &demo).unwrap();

        app.disconnect_blocking(tab_id);
        let active = app.active_tab_state().unwrap();
        assert!(active.assigned_device_id().is_none());
        assert!(active.acquisition().is_none());
        assert!(!app.device_manager.is_connected(&did));
    }

    #[test]
    fn start_spawns_and_pumps_samples() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let tab_id = app.active_tab;

        // Connect and assign the device.
        let _did = app.connect_and_assign(tab_id, &demo).unwrap();

        // Start acquisition.
        app.start_blocking(tab_id);

        let active = app.active_tab_state().unwrap();
        assert!(active.acquisition().is_some());

        // Drive the pool repeatedly.
        for _ in 0..10 {
            app.pump_once();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Drain data.
        if let Some(active) = app.tabs.get_mut(&tab_id) {
            if let Some(acq) = active.acquisition_mut() {
                acq.drain();
            }
        }

        let active = app.active_tab_state().unwrap();
        if let Some(acq) = active.acquisition() {
            assert!(acq.sample_count() > 0, "expected samples after pump, got {}", acq.sample_count());
        }
    }

    #[test]
    fn create_and_close_tabs() {
        let mut app = AppState::default();
        let initial = app.active_tab;
        assert_eq!(app.tabs.len(), 1);

        // Create a second tab.
        let id2 = app.create_tab("Test 2");
        assert_eq!(app.active_tab, id2);
        assert_eq!(app.tabs.len(), 2);

        // Close the active tab (Test 2), should fall back to initial.
        app.close_tab(id2);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_tab, initial);
    }

    fn scan_for_demo(app: &AppState) -> rb_core::ScanResult {
        block_on(app.device_manager.registry().scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
