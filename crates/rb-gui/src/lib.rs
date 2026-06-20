//! Platform-neutral RustyBench GUI built on [`eframe`]/egui.
//!
//! The same [`RustyBenchApp`] runs natively (via `rb-gui-native`) and in the
//! browser (via `rb-gui-web`). It holds presentation state only; the source of
//! truth lives in `rb-core` (`Session`, `DeviceHandle`).
//!
//! # Frame loop
//! 1. Deferred actions from the previous frame (connect/disconnect/start/stop)
//!    are applied via [`RustyBenchApp::apply_pending_actions`].
//! 2. All running devices are pumped (`Session::pump_all`) — synchronous, no
//!    blocking I/O for the demo device.
//! 3. The left sidebar lists available and connected devices.
//! 4. The central panel shows per-device waveform tabs.

#![forbid(unsafe_code)]

mod waveform_view;

use std::collections::{HashMap, HashSet};

use eframe::egui;
use futures::executor::block_on;
use rb_core::{AcquisitionCommand, AcquisitionState, DriverRegistry, ScanResult, Session};
use rb_device::DeviceId;
use waveform_view::WaveformView;

// ── App state ─────────────────────────────────────────────────────────────────

/// The root RustyBench application.
///
/// All fields are presentation state; device data lives in `session`.
pub struct RustyBenchApp {
    session: Session,
    registry: DriverRegistry,
    /// Results of the most recent scan, cleared on error.
    scan_results: Vec<ScanResult>,
    scan_error: Option<String>,
    connect_error: Option<String>,
    /// The device whose waveform tab is currently shown.
    selected_device: Option<DeviceId>,
    /// Per-device pan/zoom state, keyed by device id.
    views: HashMap<DeviceId, WaveformView>,
    // Deferred actions: set during draw, applied at the start of the next frame.
    pending_connect: Option<ScanResult>,
    pending_disconnect: Option<DeviceId>,
    pending_start: Option<DeviceId>,
    pending_stop: Option<DeviceId>,
}

impl Default for RustyBenchApp {
    fn default() -> Self {
        Self {
            session: Session::new(),
            registry: DriverRegistry::with_default_factories(),
            scan_results: Vec::new(),
            scan_error: None,
            connect_error: None,
            selected_device: None,
            views: HashMap::new(),
            pending_connect: None,
            pending_disconnect: None,
            pending_start: None,
            pending_stop: None,
        }
    }
}

impl RustyBenchApp {
    /// Construct the app from an eframe creation context.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    /// Execute any connect / disconnect / start / stop actions that were
    /// deferred during the previous frame's draw pass.
    ///
    /// Uses `block_on` for the async device calls; this is acceptable because
    /// the demo driver's control operations are instant (no real I/O).
    fn apply_pending_actions(&mut self) {
        if let Some(result) = self.pending_connect.take() {
            match block_on(self.registry.connect(&result.driver, &result.candidate)) {
                Ok(device) => {
                    let id = self.session.add_device(device);
                    self.selected_device = Some(id);
                    self.connect_error = None;
                }
                Err(e) => {
                    self.connect_error = Some(e.to_string());
                }
            }
        }
        if let Some(id) = self.pending_disconnect.take() {
            if let Some(mut handle) = self.session.remove(&id) {
                if handle.state() == &AcquisitionState::Running {
                    let _ = block_on(handle.stop());
                }
            }
            self.views.remove(&id);
            if self.selected_device.as_ref() == Some(&id) {
                self.selected_device = self.session.device_ids().into_iter().next();
            }
        }
        if let Some(id) = self.pending_start.take() {
            if let Some(handle) = self.session.device_mut(&id) {
                let _ = block_on(handle.apply(AcquisitionCommand::Start));
            }
        }
        if let Some(id) = self.pending_stop.take() {
            if let Some(handle) = self.session.device_mut(&id) {
                let _ = block_on(handle.apply(AcquisitionCommand::Stop));
            }
        }
    }

    // ── Sidebar ───────────────────────────────────────────────────────────────

    fn draw_device_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Devices");
        ui.separator();

        // Scan
        if ui.button("⟳ Scan").clicked() {
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

        // Available candidates (not yet connected)
        let connected: HashSet<String> = self
            .session
            .device_ids()
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

        // Connected devices
        ui.separator();
        ui.label("Connected:");
        let device_ids = self.session.device_ids();
        if device_ids.is_empty() {
            ui.weak("(none)");
        }

        for id in &device_ids {
            let label = if let Some(handle) = self.session.device(id) {
                let info = handle.device().info();
                format!("{}/{}", info.vendor, info.model)
            } else {
                id.to_string()
            };

            let is_selected = self.selected_device.as_ref() == Some(id);
            if ui.selectable_label(is_selected, &label).clicked() {
                self.selected_device = Some(id.clone());
            }

            // Per-device controls row
            ui.horizontal(|ui| {
                if let Some(handle) = self.session.device(id) {
                    match handle.state() {
                        AcquisitionState::Running => {
                            ui.colored_label(egui::Color32::GREEN, "●");
                            if ui.small_button("⏹").on_hover_text("Stop").clicked() {
                                self.pending_stop = Some(id.clone());
                            }
                        }
                        AcquisitionState::Error(msg) => {
                            let msg = msg.clone();
                            ui.colored_label(egui::Color32::RED, "⚠")
                                .on_hover_text(&msg);
                        }
                        _ => {
                            if ui.small_button("▶").on_hover_text("Start").clicked() {
                                self.pending_start = Some(id.clone());
                            }
                        }
                    }
                }
                if ui.small_button("✖").on_hover_text("Disconnect").clicked() {
                    self.pending_disconnect = Some(id.clone());
                }
            });

            if let Some(handle) = self.session.device(id) {
                ui.weak(format!("{} samples", handle.sample_count()));
            }
            ui.add_space(4.0);
        }
    }

    // ── Central panel ─────────────────────────────────────────────────────────

    fn draw_main_panel(&mut self, ui: &mut egui::Ui) {
        let device_ids = self.session.device_ids();

        if device_ids.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No devices connected.\n\nUse the sidebar to scan and connect a device.");
            });
            return;
        }

        // Ensure the selected tab refers to a connected device.
        if self
            .selected_device
            .as_ref()
            .is_none_or(|id| !device_ids.contains(id))
        {
            self.selected_device = device_ids.first().cloned();
        }

        // Tab bar
        ui.horizontal(|ui| {
            for id in &device_ids {
                let label = if let Some(handle) = self.session.device(id) {
                    let info = handle.device().info();
                    format!("{}/{}", info.vendor, info.model)
                } else {
                    id.to_string()
                };
                let selected = self.selected_device.as_ref() == Some(id);
                if ui.selectable_label(selected, label).clicked() {
                    self.selected_device = Some(id.clone());
                }
            }
        });
        ui.separator();

        // Waveform area for the selected device.
        if let Some(id) = self.selected_device.clone() {
            // `self.views` (mut) and `self.session` (immut) are different fields:
            // the borrow checker splits them correctly.
            let view = self.views.entry(id.clone()).or_default();
            if let Some(handle) = self.session.device(&id) {
                view.draw(ui, handle);
            }
        }
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RustyBenchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 1. Apply deferred actions from the last frame.
        self.apply_pending_actions();

        // 2. Pump all running devices (sync, no blocking I/O for demo).
        self.session.pump_all(512);

        // 3. Layout: resizable sidebar on the left, waveform panel on the right.
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
        assert!(app.selected_device.is_none());
    }

    #[test]
    fn start_and_pump_acquires_samples() {
        let mut app = RustyBenchApp::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();

        let id = app.selected_device.clone().unwrap();
        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        app.session.pump_all(100);

        assert_eq!(app.session.device(&id).unwrap().sample_count(), 100);
    }

    #[test]
    fn stop_freezes_sample_count() {
        let mut app = RustyBenchApp::default();
        let demo = scan_for_demo(&app);
        app.pending_connect = Some(demo);
        app.apply_pending_actions();

        let id = app.selected_device.clone().unwrap();
        app.pending_start = Some(id.clone());
        app.apply_pending_actions();
        app.session.pump_all(50);

        app.pending_stop = Some(id.clone());
        app.apply_pending_actions();
        app.session.pump_all(50); // no-op after stop

        assert_eq!(app.session.device(&id).unwrap().sample_count(), 50);
    }

    fn scan_for_demo(app: &RustyBenchApp) -> ScanResult {
        block_on(app.registry.scan_all())
            .unwrap()
            .into_iter()
            .find(|r| r.driver == "demo")
            .expect("demo driver present")
    }
}
