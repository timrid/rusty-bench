//! Application state: the top-level orchestrator.
//!
//! [`AppState`] owns the [`DeviceManager`], all [`TabState`]s, and the
//! executor for background acquisition futures. It is framework-agnostic and
//! testable without a display.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::Signal;
use rb_core::{DeviceHandle, DeviceOrigin};
use rb_device::DeviceId;

use crate::device_manager::DeviceManager;
use crate::tab_state::{TabId, TabSource, TabState};

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    /// Program-level device manager (scan, connect, handle pool).
    pub device_manager: DeviceManager,

    /// All open tabs, keyed by id.
    pub tabs: HashMap<TabId, TabState>,
    /// The currently active (visible) tab.
    pub active_tab: TabId,
    next_tab_id: u64,
    next_session_num: u64,
}

impl AppState {
    /// Creates an app state with a device manager that already has a connected
    /// device (for integration tests that inject a mock device).
    #[must_use]
    pub fn from_device_manager(device_manager: DeviceManager) -> Self {
        let first_id = TabId(1);
        let mut tab = TabState::new(first_id, "Session 1");

        // If a device is already connected, assign it to the first tab.
        let device_ids = device_manager.connected_device_ids();
        if let Some(did) = device_ids.into_iter().next() {
            tab.source = TabSource::Device(did.clone());
            tab.content = Some(crate::logic_analyzer::default_content());
        }

        let mut tabs = HashMap::new();
        tabs.insert(first_id, tab);

        Self {
            device_manager,
            tabs,
            active_tab: first_id,
            next_tab_id: 2,
            next_session_num: 2,
        }
    }

    pub fn new() -> Self {
        Self::from_device_manager(DeviceManager::new())
    }

    // ── Tab management ────────────────────────────────────────────────────────

    /// Creates a new empty tab and makes it the active tab.
    /// The `_label` parameter is ignored — tabs are named "Session N" automatically.
    pub fn create_tab(&mut self, _label: impl Into<String>) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        let label = format!("Session {}", self.next_session_num);
        self.next_session_num += 1;
        let tab = TabState::new(id, label);
        self.tabs.insert(id, tab);
        self.active_tab = id;
        id
    }

    /// Closes a tab and removes it. If closing the active tab, activates
    /// the nearest remaining tab or creates a fresh one.
    ///
    /// The caller must ensure the tab is not running — close buttons are
    /// hidden while a device is connected.
    pub fn close_tab(&mut self, id: TabId) {
        if let Some(tab) = self.tabs.get_mut(&id) {
            if let Some(content) = tab.content.as_mut() {
                content.stop();
            }
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
    /// The caller is responsible for setting the appropriate [`TabContent`].
    pub fn assign_device_to_tab(&mut self, tab_id: TabId, device_id: DeviceId) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.source = TabSource::Device(device_id.clone());
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

    // ── Device connection ─────────────────────────────────────────────────────

    /// Connects a device, first disconnecting any other connected device.
    /// Returns the new device's ID, or an error string.
    ///
    /// Takes `&AppStateRef` (not `&mut self`) so the `RefCell` borrow is
    /// released before the async `connect()` call — the UI can keep rendering.
    /// Uses `try_borrow_mut` with a brief yield to avoid colliding with the
    /// Dioxus render cycle.
    pub async fn connect_single(
        app_ref: &std::rc::Rc<std::cell::RefCell<AppState>>,
        driver: &str,
        candidate: &rb_transport::DeviceCandidate,
        origin: DeviceOrigin,
    ) -> Result<DeviceId, String> {
        // Clone registry and disconnect any existing device.
        let registry = {
            let mut s = app_ref.borrow_mut();
            let r = s.device_manager.registry_clone();
            let connected: Vec<DeviceId> = s.device_manager.connected_device_ids();
            for id in &connected {
                s.device_manager.disconnect(id);
            }
            r
        };

        // Async connect — no borrow on AppState.
        let device = registry
            .connect(driver, candidate)
            .await
            .map_err(|e| e.to_string())?;

        // Re-borrow for store + tab assignment.
        let label = format!("{}/{}", device.info().vendor, device.info().model);
        let id = device.id().clone();
        let content = crate::logic_analyzer::init_content(device.as_ref());
        let handle = DeviceHandle::new(device);
        let mut s = app_ref.borrow_mut();
        s.device_manager
            .store_connected(id.clone(), handle, label, driver, origin);
        let tab_id = s.active_tab;
        s.assign_device_to_tab(tab_id, id.clone());
        if let Some(tab) = s.tabs.get_mut(&tab_id) {
            tab.content = Some(crate::tab_content::TabContent::LogicAnalyzer(content));
        }
        Ok(id)
    }

    // ── Hotplug watch ────────────────────────────────────────────────────────

    /// Spawns an event-driven device-discovery loop via `nusb::watch_devices()`.
    /// On each hotplug event, a device scan runs and `data_version` is bumped.
    ///
    /// No-op on WASM (no hotplug support).
    pub fn spawn_usb_hotplug_watch(app_ref: &Rc<RefCell<AppState>>, mut data_version: Signal<u64>) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use futures::StreamExt;

            let state = app_ref.clone();
            dioxus::prelude::spawn(async move {
                let Ok(mut watch) = nusb::watch_devices() else {
                    log::warn!("nusb::watch_devices not available");
                    return;
                };
                loop {
                    if watch.next().await.is_some() {
                        state.borrow_mut().scan_devices().await;
                        data_version += 1;
                    }
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (app_ref, data_version);
    }

    // ── Device connection (delegates to DeviceManager) ────────────────────────

    /// Triggers a device scan (always async, no blocking).
    /// Spawns a platform-appropriate task that updates scan results and bumps `data_version`.
    ///
    /// If `request_usb` is true (WASM only), the browser's WebUSB permission
    /// dialog is shown before scanning — use only for explicit user actions.
    pub fn trigger_scan(
        app_ref: &std::rc::Rc<std::cell::RefCell<AppState>>,
        mut data_version: dioxus::prelude::Signal<u64>,
        request_usb: bool,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = request_usb; // unused on desktop
        let registry = app_ref.borrow().device_manager.registry_clone();
        let app = app_ref.clone();

        // Dioxus spawn handles both desktop (tokio) and WASM (wasm-bindgen).
        dioxus::prelude::spawn(async move {
            #[cfg(target_arch = "wasm32")]
            if request_usb {
                let _ = crate::app_state::request_supported_usb_devices().await;
            }
            let result = registry.scan_all().await.map_err(|e| e.to_string());
            let mut s = app.borrow_mut();
            match result {
                Ok(results) => {
                    s.device_manager.apply_scan_results(results);
                    s.device_manager.scan_error = None;
                }
                Err(e) => {
                    s.device_manager.scan_error = Some(e);
                }
            }
            data_version += 1;
        });
    }

    // ── Device polling ───────────────────────────────────────────────────────

    /// Runs one device scan cycle and applies the results.
    pub async fn scan_devices(&mut self) {
        let registry = self.device_manager.registry_clone();
        match registry.scan_all().await {
            Ok(results) => {
                self.device_manager.apply_scan_results(results);
                self.device_manager.scan_error = None;
            }
            Err(e) => {
                self.device_manager.scan_error = Some(e.to_string());
            }
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Whether the active tab is currently running an acquisition.
    /// UI elements like tab switching and device re-assignment should be
    /// disabled while this is true.
    pub fn is_device_locked(&self) -> bool {
        self.active_tab_state()
            .is_some_and(|t| t.is_running())
    }

    /// Whether the active tab currently has recorded sample data.
    /// Used to decide whether to show a confirmation dialog before switching
    /// devices.
    pub fn active_tab_has_samples(&self) -> bool {
        let Some(tab) = self.active_tab_state() else {
            return false;
        };
        let handle = tab
            .assigned_device_id()
            .and_then(|did| self.device_manager.device_handle(did));
        tab.content.as_ref().is_some_and(|c| c.has_data(handle))
    }

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

    pub fn any_running(&self) -> bool {
        self.tabs.values().any(|t| t.is_running())
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
    use crate::logic_analyzer::control;

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
        assert!(app.device_manager.known_devices().is_empty());
    }

    #[test]
    fn scan_populates_results() {
        let mut app = AppState::default();
        let results = block_on(app.device_manager.registry().scan_all()).unwrap();
        app.device_manager.apply_scan_results(results);
        assert!(!app.device_manager.known_devices().is_empty());
        assert!(app.device_manager.known_devices().iter().any(|r| r.driver == "demo"));
    }

    #[test]
    fn connect_adds_device_to_active_tab() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let tab_id = app.active_tab;
        let did = block_on(connect_and_assign(&mut app, tab_id, &demo)).unwrap();
        assert!(app.device_manager.is_connected(&did));
        let active = app.active_tab_state().unwrap();
        assert_eq!(active.assigned_device_id().cloned(), Some(did));
    }

    #[test]

    fn start_spawns_and_pumps_samples() {
        let mut app = AppState::default();
        let demo = scan_for_demo(&app);
        let tab_id = app.active_tab;

        // Connect and assign the device.
        let _did = block_on(connect_and_assign(&mut app, tab_id, &demo)).unwrap();

        // Start acquisition.
        control::start_sync(&mut app, tab_id);

        let active = app.active_tab_state().unwrap();
        assert!(active.logic_analyzer().acquisition.as_ref().is_some());

        // Drive the pool repeatedly.
        for _ in 0..10 {
            control::pump_executor();
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

    /// Test helper: async connect + assign.
    async fn connect_and_assign(
        app: &mut AppState,
        tab_id: TabId,
        scan_result: &rb_core::KnownDevice,
    ) -> Result<DeviceId, rb_core::SessionError> {
        let device = app.device_manager.registry()
            .connect(&scan_result.driver, &scan_result.candidate).await?;
        let label = format!("{}/{}", device.info().vendor, device.info().model);
        let id = device.id().clone();
        let content = crate::logic_analyzer::init_content(device.as_ref());
        let handle = DeviceHandle::new(device);
        app.device_manager.store_connected(id.clone(), handle, label, &scan_result.driver, scan_result.origin);
        app.assign_device_to_tab(tab_id, id.clone());
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            tab.content = Some(crate::tab_content::TabContent::LogicAnalyzer(content));
        }
        Ok(id)
    }

    fn scan_for_demo(app: &AppState) -> rb_core::KnownDevice {
        block_on(app.device_manager.registry().scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
