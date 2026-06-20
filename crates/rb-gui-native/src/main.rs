//! Native entrypoint for the RustyBench GUI.

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "RustyBench",
        native_options,
        Box::new(|cc| Ok(Box::new(rb_gui::RustyBenchApp::new(cc)))),
    )
}
