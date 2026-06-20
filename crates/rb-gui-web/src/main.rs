//! Web (WASM) entrypoint for the RustyBench GUI, built with `trunk`.

// Native builds of this crate produce an empty binary; the real entrypoint is
// compiled only for wasm32 and started by trunk.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no global window")
            .document()
            .expect("no document on window");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing element with id 'the_canvas_id'")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("'the_canvas_id' is not a canvas element");

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(rb_gui::RustyBenchApp::new(cc)))),
            )
            .await
            .expect("failed to start eframe web runner");
    });
}
