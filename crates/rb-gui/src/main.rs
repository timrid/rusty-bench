//! Unified entrypoint for the RustyBench GUI.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("debug,nusb=info"),
    )
    .init();
    dioxus::launch(rb_gui::components::app::App);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    console_log::init_with_level(log::Level::max()).expect("console_log setup");
    dioxus::launch(rb_gui::components::app::App);
}
