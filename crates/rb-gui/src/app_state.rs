//! Application state: the top-level orchestrator.
//!
//! [`AppState`] owns the [`DeviceManager`], all [`TabState`]s, and the
//! executor for background acquisition futures. It is framework-agnostic and
//! testable without a display.

use std::collections::HashMap;

use rb_core::{
    AcquisitionCommand, AcquisitionState, DeviceHandle,
};
use rb_device::DeviceId;

use crate::logic_analyzer::acquisition::{AcquisitionConfig, DeviceAcquisition};
use crate::logic_analyzer::control;
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
    /// Tab that initiated a WASM async connect (assign device on completion).
    #[cfg(target_arch = "wasm32")]
    pub pending_wasm_assign: Option<TabId>,
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
            #[cfg(target_arch = "wasm32")]
            pending_wasm_assign: None,
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
        // Stop acquisition if running (delegates to LA control).
        control::clear_acquisition(self, id);
        if let Some(tab) = self.tabs.get_mut(&id) {
            tab.set_assigned_device_id(None);
        }
        self.tabs.remove(&id);
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
        match self.device_manager.connect_blocking(scan_result) {
            Ok(device_id) => {
                self.assign_device_to_tab(tab_id, device_id.clone());
                Ok(device_id)
            }
            Err(e) => {
                // On WASM, connect is async — remember the tab for later assignment.
                #[cfg(target_arch = "wasm32")]
                {
                    self.pending_wasm_assign = Some(tab_id);
                }
                Err(e)
            }
        }
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
        control::clear_acquisition(self, tab_id);
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if let Some(did) = tab.assigned_device_id().cloned() {
                self.device_manager.disconnect(&did);
            }
            tab.set_assigned_device_id(None);
        }
    }

    // ── Pending actions ───────────────────────────────────────────────────────

    pub fn apply_pending_actions(&mut self) {
        self.device_manager.apply_pending_actions();
        self.device_manager.collect_returns();

        // On WASM: if an async connect just completed, assign the device to the tab.
        #[cfg(target_arch = "wasm32")]
        if let Some(tab_id) = self.pending_wasm_assign {
            for did in self.device_manager.connected_device_ids() {
                let already_assigned = self.tabs.values().any(|t| t.assigned_device_id() == Some(&did));
                if !already_assigned {
                    self.assign_device_to_tab(tab_id, did);
                    self.pending_wasm_assign = None;
                    break;
                }
            }
        }

        if let Some(tab_id) = self.pending_disconnect.take() {
            self.close_tab(tab_id);
        }
        if let Some(tab_id) = self.pending_start.take() {
            control::apply_start(self, tab_id);
        }
        if let Some(tab_id) = self.pending_stop.take() {
            control::apply_stop(self, tab_id);
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    pub fn device_label(&self, id: &DeviceId) -> String {
        self.device_manager
            .device_label(id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.to_string())
    }

    pub fn device_handle(&self, id: &DeviceId) -> Option<&DeviceHandle> {
        self.device_manager.device_handle(id)
    }

    pub fn handle_for_tab(&self, tab_id: TabId) -> Option<&DeviceHandle> {
        let tab = self.tabs.get(&tab_id)?;
        tab.assigned_device_id()
            .and_then(|did| self.device_manager.device_handle(did))
    }

    pub fn device_id_for_tab(&self, tab_id: TabId) -> Option<DeviceId> {
        self.tabs
            .get(&tab_id)
            .and_then(|t| t.assigned_device_id().cloned())
            .filter(|did| self.device_manager.is_connected(did))
    }

    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        self.device_manager.connected_device_ids()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    // ── Executor ──────────────────────────────────────────────────────────────

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
        assert!(active.logic_analyzer().acquisition.as_ref().is_none());
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
        assert!(active.logic_analyzer().acquisition.as_ref().is_none());
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
        control::start(&mut app, tab_id);

        let active = app.active_tab_state().unwrap();
        assert!(active.logic_analyzer().acquisition.as_ref().is_some());

        // Drive the pool repeatedly.
        for _ in 0..10 {
            app.pump_once();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Drain data.
        if let Some(active) = app.tabs.get_mut(&tab_id) {
            if let Some(acq) = active.logic_analyzer_mut().acquisition.as_mut() {
                acq.drain();
            }
        }

        let active = app.active_tab_state().unwrap();
        if let Some(acq) = active.logic_analyzer().acquisition.as_ref() {
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
