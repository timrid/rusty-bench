//! Platform-neutral RustyBench GUI built on [`eframe`]/egui.
//!
//! The same [`RustyBenchApp`] runs natively (via `rb-gui-native`) and in the
//! browser (via `rb-gui-web`). It holds presentation state only; the source of
//! truth lives in `rb-core`.

use eframe::egui;

/// The root RustyBench application.
#[derive(Default)]
pub struct RustyBenchApp {}

impl RustyBenchApp {
    /// Construct the app from an eframe creation context.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {}
    }
}

impl eframe::App for RustyBenchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("RustyBench");
        ui.label("Workspace skeleton (M0).");
        ui.label(format!("core: {}", rb_core::CRATE));
    }
}
