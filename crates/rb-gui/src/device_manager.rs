//! Device management at the program level.
//!
//! [`DeviceManager`] owns all device connections, shared across sessions.
//! A device is connected once and can be used by multiple sessions
//! (though only one session can acquire data at a time).

use futures::channel::oneshot;
use rb_core::{DeviceHandle, DeviceOrigin, DriverRegistry, KnownDevice};
use rb_device::DeviceId;
use rb_transport::DeviceCandidate;

/// Managed state for a connected device.
struct ConnectedState {
    /// The device's stable identifier (kept even when handle is borrowed).
    device_id: DeviceId,
    /// The device handle; None while borrowed by an acquisition.
    handle: Option<DeviceHandle>,
    /// Human-readable label (vendor/model).
    label: String,
}

/// A single device slot: always has a [`KnownDevice`], optionally connected.
struct DeviceSlot {
    known: KnownDevice,
    /// `None` = not connected, `Some` = connected.
    state: Option<ConnectedState>,
}

/// Manages device discovery, connection, and handle lifecycle.
///
/// Devices are connected at the program level — not per session.
/// Multiple sessions can reference the same connected device;
/// the handle is lent out during acquisition and returned afterwards.
pub struct DeviceManager {
    registry: DriverRegistry,
    /// All known devices (scan results + manually added), some connected.
    devices: Vec<DeviceSlot>,
    pub scan_error: Option<String>,
    pub connect_error: Option<String>,
    /// Receivers for handles returning from completed acquisition futures.
    pending_returns: Vec<oneshot::Receiver<(DeviceId, DeviceHandle)>>,
}

impl DeviceManager {
    /// Creates a new manager with the default driver registry and firmware support.
    #[must_use]
    pub fn new() -> Self {
        let firmware_loader = Box::new(crate::firmware::Fx2lafwAssetLoader::new());
        let factories = rb_drivers::factories_with_firmware(Some(firmware_loader));
        Self {
            registry: DriverRegistry::new(factories),
            devices: Vec::new(),
            scan_error: None,
            connect_error: None,
            pending_returns: Vec::new(),
        }
    }

    /// Clones the driver registry for async use (WASM scan/connect).
    pub fn registry_clone(&self) -> DriverRegistry {
        self.registry.clone()
    }

    // ── Scan results ──────────────────────────────────────────────────────

    /// Replaces scan-discovered (not connected, not manual) entries with new
    /// scan results. Connected and manual devices are left untouched.
    pub fn apply_scan_results(&mut self, results: Vec<KnownDevice>) {
        // Retain: connected devices AND manual devices.
        self.devices.retain(|slot| {
            slot.state.is_some() || slot.known.origin == DeviceOrigin::Manual
        });

        // Dedup: skip results that overlap with existing entries.
        for result in results {
            let already_known = self.devices.iter().any(|slot| {
                slot.known.candidate == result.candidate
                    && slot.known.driver == result.driver
            });
            if !already_known {
                self.devices.push(DeviceSlot {
                    known: result,
                    state: None,
                });
            }
        }
    }

    /// Returns a snapshot of all known devices (connected + not connected).
    pub fn known_devices(&self) -> Vec<&KnownDevice> {
        self.devices.iter().map(|s| &s.known).collect()
    }

    /// All known devices as owned `Vec<KnownDevice>` for UI consumption.
    pub fn known_devices_owned(&self) -> Vec<KnownDevice> {
        self.devices.iter().map(|s| s.known.clone()).collect()
    }

    // ── Connect / Disconnect ──────────────────────────────────────────────

    /// Stores a connected device (used after an async connect completes).
    /// The device's known entry is updated (or created) with `state = Some`.
    /// `driver` and `origin` are used when creating a new slot for a
    /// directly-connected device that has no pre-existing known entry.
    pub fn store_connected(
        &mut self,
        id: DeviceId,
        handle: DeviceHandle,
        label: String,
        driver: &str,
        origin: DeviceOrigin,
    ) {
        let device_id = handle.device().id().clone();

        // Try to find an existing slot for this device.
        let existing = self.devices.iter_mut().find(|s| {
            // Match by stored device_id.
            s.state.as_ref().is_some_and(|cs| cs.device_id == device_id)
                || {
                    // Also match by candidate address for scan-discovered slots.
                    let did = DeviceId::new(&s.known.candidate.address);
                    did == device_id
                }
        });

        if let Some(slot) = existing {
            slot.state = Some(ConnectedState {
                device_id: device_id.clone(),
                handle: Some(handle),
                label,
            });
        } else {
            // Create a new slot for a directly-connected device (e.g., from tests).
            let info = handle.device().info().clone();
            let candidate = DeviceCandidate::new(info, id.to_string());
            self.devices.push(DeviceSlot {
                known: KnownDevice {
                    driver: driver.to_string(),
                    candidate,
                    origin,
                },
                state: Some(ConnectedState {
                    device_id: device_id.clone(),
                    handle: Some(handle),
                    label,
                }),
            });
        }
        self.connect_error = None;
    }

    /// Disconnects a device, dropping its handle and clearing state.
    pub fn disconnect(&mut self, device_id: &DeviceId) {
        if let Some(slot) = self.devices.iter_mut().find(|s| {
            s.state.as_ref().is_some_and(|cs| &cs.device_id == device_id)
        }) {
            slot.state = None;
        }
    }

    /// Returns true if the device is connected (handle may be borrowed).
    pub fn is_connected(&self, device_id: &DeviceId) -> bool {
        self.devices.iter().any(|s| {
            s.state.as_ref().is_some_and(|cs| &cs.device_id == device_id)
        })
    }

    /// Returns true if the device handle is available (not borrowed by an acquisition).
    pub fn is_available(&self, device_id: &DeviceId) -> bool {
        self.devices.iter().any(|s| {
            s.state.as_ref().is_some_and(|cs| {
                cs.handle.is_some() && &cs.device_id == device_id
            })
        })
    }

    // Compatibility: check by KnownDevice match.
    pub fn is_connected_result(&self, result: &KnownDevice) -> bool {
        self.devices.iter().any(|s| {
            s.known.candidate == result.candidate
                && s.known.driver == result.driver
                && s.state.is_some()
        })
    }

    /// Returns the DeviceId for a connected known device, if any.
    pub fn device_id_for_result(&self, result: &KnownDevice) -> Option<DeviceId> {
        self.devices.iter().find_map(|s| {
            if s.known.candidate == result.candidate
                && s.known.driver == result.driver
                && s.state.is_some()
            {
                s.state.as_ref().map(|cs| cs.device_id.clone())
            } else {
                None
            }
        })
    }

    // ── Handle borrowing ──────────────────────────────────────────────────

    /// Takes the device handle for use in an acquisition.
    ///
    /// Returns `None` if the device is not connected or the handle is already borrowed.
    pub fn take_handle(&mut self, device_id: &DeviceId) -> Option<DeviceHandle> {
        self.devices
            .iter_mut()
            .find(|s| {
                s.state.as_ref().is_some_and(|cs| &cs.device_id == device_id)
            })
            .and_then(|s| s.state.as_mut().and_then(|cs| cs.handle.take()))
    }

    /// Returns a device handle after acquisition completes.
    pub fn return_handle(&mut self, _device_id: DeviceId, handle: DeviceHandle) {
        if let Some(slot) = self.devices.iter_mut().find(|s| {
            s.state.as_ref().is_some_and(|cs| {
                cs.handle.is_none() // handle was taken
            })
        }) {
            if let Some(ref mut cs) = slot.state {
                cs.handle = Some(handle);
            }
        }
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
        self.devices.iter().find_map(|s| {
            s.state.as_ref()
                .filter(|cs| &cs.device_id == device_id)
                .and_then(|cs| cs.handle.as_ref())
        })
    }

    /// Returns a mutable reference to the device handle, if available.
    pub fn device_handle_mut(&mut self, device_id: &DeviceId) -> Option<&mut DeviceHandle> {
        self.devices.iter_mut().find_map(|s| {
            s.state.as_mut()
                .filter(|cs| &cs.device_id == device_id)
                .and_then(|cs| cs.handle.as_mut())
        })
    }

    /// Returns the device label (vendor/model).
    pub fn device_label(&self, device_id: &DeviceId) -> Option<&str> {
        self.devices.iter().find_map(|s| {
            s.state.as_ref()
                .filter(|cs| &cs.device_id == device_id)
                .map(|cs| cs.label.as_str())
        })
    }

    /// Returns all connected device IDs.
    pub fn connected_device_ids(&self) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter_map(|s| s.state.as_ref().map(|cs| cs.device_id.clone()))
            .collect()
    }

    /// Returns the driver registry (for direct access).
    pub fn registry(&self) -> &DriverRegistry {
        &self.registry
    }

    /// Returns the Device's `additional_info()` for a connected device, if available.
    pub fn additional_info(&self, device_id: &DeviceId) -> Vec<(&str, &str)> {
        self.device_handle(device_id)
            .map(|h| h.device().additional_info())
            .unwrap_or_default()
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}
