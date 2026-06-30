//! Device management at the program level.
//!
//! [`DeviceManager`] owns all device connections, shared across sessions.
//! A device is connected once and can be used by multiple sessions
//! (though only one session can acquire data at a time).

use std::collections::HashMap;

use futures::channel::oneshot;
use rb_core::{DeviceHandle, DriverRegistry, ScanResult};
use rb_device::DeviceId;

/// Manages device discovery, connection, and handle lifecycle.
///
/// Devices are connected at the program level — not per session.
/// Multiple sessions can reference the same connected device;
/// the handle is lent out during acquisition and returned afterwards.
pub struct DeviceManager {
    registry: DriverRegistry,
    /// Discovered but not yet connected devices.
    pub scan_results: Vec<ScanResult>,
    pub scan_error: Option<String>,
    /// Currently connected devices (handle is None while borrowed by a session).
    connected: HashMap<DeviceId, DeviceEntry>,
    pub connect_error: Option<String>,
    /// Receivers for handles returning from completed acquisition futures.
    pending_returns: Vec<oneshot::Receiver<(DeviceId, DeviceHandle)>>,
    /// WASM: pending scan result receiver.
    #[cfg(target_arch = "wasm32")]
    pub pending_wasm_scan: Option<oneshot::Receiver<Result<Vec<ScanResult>, String>>>,
}

struct DeviceEntry {
    /// The device handle; None while borrowed by an acquisition.
    handle: Option<DeviceHandle>,
    /// Human-readable label (vendor/model).
    label: String,
}

impl DeviceManager {
    /// Creates a new manager with the default driver registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: DriverRegistry::with_default_factories(),
            scan_results: Vec::new(),
            scan_error: None,
            connected: HashMap::new(),
            connect_error: None,
            pending_returns: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            pending_wasm_scan: None,
        }
    }

    // ── Scan ──────────────────────────────────────────────────────────────

    /// Triggers a synchronous device scan (native) or queues an async one (WASM).
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
            let (tx, rx) = oneshot::channel();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = crate::app_state::request_supported_usb_devices().await;
                let result = registry.scan_all().await.map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
            self.pending_wasm_scan = Some(rx);
        }
    }

    // ── Connect / Disconnect ──────────────────────────────────────────────

    /// Stores a connected device (used after an async connect completes).
    pub fn store_connected(&mut self, id: DeviceId, handle: DeviceHandle, label: String) {
        self.connected.insert(
            id,
            DeviceEntry {
                handle: Some(handle),
                label,
            },
        );
        self.connect_error = None;
    }

    /// Disconnects a device, dropping its handle.
    pub fn disconnect(&mut self, device_id: &DeviceId) {
        self.connected.remove(device_id);
    }

    /// Returns true if a device matching this scan result is connected.
    /// DeviceId is derived from the candidate address, so we can look it up directly.
    pub fn is_connected_result(&self, result: &ScanResult) -> bool {
        let did = DeviceId::new(&result.candidate.address);
        self.connected.contains_key(&did)
    }

    /// Returns the DeviceId for a connected scan result, if any.
    pub fn device_id_for_result(&self, result: &ScanResult) -> Option<DeviceId> {
        let did = DeviceId::new(&result.candidate.address);
        self.connected.get(&did).map(|_| did)
    }

    /// Returns true if the device is connected (handle may be borrowed).
    pub fn is_connected(&self, device_id: &DeviceId) -> bool {
        self.connected.contains_key(device_id)
    }

    /// Returns true if the device handle is available (not borrowed by an acquisition).
    pub fn is_available(&self, device_id: &DeviceId) -> bool {
        self.connected
            .get(device_id)
            .is_some_and(|e| e.handle.is_some())
    }

    // ── Handle borrowing ──────────────────────────────────────────────────

    /// Takes the device handle for use in an acquisition.
    ///
    /// Returns `None` if the device is not connected or the handle is already borrowed.
    pub fn take_handle(&mut self, device_id: &DeviceId) -> Option<DeviceHandle> {
        self.connected.get_mut(device_id)?.handle.take()
    }

    /// Returns a device handle after acquisition completes.
    pub fn return_handle(&mut self, device_id: DeviceId, handle: DeviceHandle) {
        if let Some(entry) = self.connected.get_mut(&device_id) {
            entry.handle = Some(handle);
        }
        // If the device was disconnected while borrowed, the handle is dropped.
    }

    /// Registers a pending handle return from an acquisition future.
    pub fn register_pending_return(&mut self, rx: oneshot::Receiver<(DeviceId, DeviceHandle)>) {
        self.pending_returns.push(rx);
    }

    /// Processes completed acquisitions, returning handles to the pool.
    pub fn collect_returns(&mut self) {
        let pending: Vec<_> = self.pending_returns.drain(..).collect();
        let mut remaining = Vec::new();
        for mut rx in pending {
            if let Ok(Some((device_id, handle))) = rx.try_recv() {
                self.return_handle(device_id, handle);
            } else {
                remaining.push(rx);
            }
        }
        self.pending_returns = remaining;
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Returns a reference to the device handle, if available.
    pub fn device_handle(&self, device_id: &DeviceId) -> Option<&DeviceHandle> {
        self.connected.get(device_id)?.handle.as_ref()
    }

    /// Returns the device label (vendor/model).
    pub fn device_label(&self, device_id: &DeviceId) -> Option<&str> {
        self.connected.get(device_id).map(|e| e.label.as_str())
    }

    /// Returns all connected device IDs.
    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        self.connected.keys().cloned().collect()
    }

    /// Returns the driver registry (for direct access in legacy code paths).
    pub fn registry(&self) -> &DriverRegistry {
        &self.registry
    }

    // ── Pending actions (WASM) ────────────────────────────────────────────

    /// Processes pending WASM scan results.
    pub fn apply_pending_actions(&mut self) {
        // WASM scan
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
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}
