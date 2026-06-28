//! rb-canvas — General-purpose virtual canvas abstraction.
//!
//! Provides a [`Canvas`] trait modelled on the HTML Canvas 2D Context API
//! and two implementations:
//!
//! - [`PixelCanvas`] — renders to an in-memory RGBA pixel buffer (for tests).
//! - [`JsCanvasRenderer`] — generates JavaScript strings for browser/webview
//!   canvas elements (production).

pub mod color;
pub mod traits;
pub mod pixel;
pub mod js;

pub use color::RgbaColor;
pub use traits::Canvas;
pub use pixel::PixelCanvas;
pub use js::JsCanvasRenderer;
