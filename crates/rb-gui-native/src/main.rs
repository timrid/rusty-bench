//! Native entrypoint for the RustyBench GUI.

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "RustyBench",
        native_options,
        Box::new(|cc| Ok(Box::new(rb_gui::RustyBenchApp::new(cc)))),
    )
}
