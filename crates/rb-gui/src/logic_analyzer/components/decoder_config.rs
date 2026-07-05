//! Decoder configuration widget: dropdown + per-kind parameter inputs.
//!
//! [`DecoderState`] owns all decoder-related state (kind, per-protocol
//! parameters, cached decoder instance, and annotations).  It is stored as a
//! field on [`WaveformView`].

use dioxus::prelude::*;

use rb_core::DeviceHandle;
use rb_decode::{Annotation, Decoder, I2cConfig, I2cDecoder, SpiConfig, SpiDecoder, UartConfig, UartDecoder};
use rb_model::DigitalTrace;

use crate::logic_analyzer::view::{DecoderKind, WaveformView};

// ── Decoder state ────────────────────────────────────────────────────────────

/// All mutable decoder state: protocol selection, per-protocol parameters,
/// the (non-Clone) decoder instance, and its output annotations.
///
/// Lives as `WaveformView::decoder`.
///
/// Manual Clone impl because `Box<dyn Decoder>` isn't Clone — the decoder
/// is rebuilt on demand after cloning.
pub struct DecoderState {
    /// Which protocol decoder is selected.
    pub kind: DecoderKind,
    /// Rebuilt on demand; skipped by Clone (reconstructed from config).
    #[allow(clippy::type_complexity)]
    decoder: Option<Box<dyn Decoder>>,
    /// Annotations produced by the decoder.
    pub annotations: Vec<Annotation>,
    /// How many digital-store words have been fed to the decoder so far.
    decoded_up_to: usize,
    /// When `true`, the decoder is rebuilt and all annotations are cleared on
    /// the next `feed()` call.
    pub dirty: bool,

    // ── Per-protocol parameters ──────────────────────────────────────────
    pub uart_baud: u32,
    pub uart_rx_bit: u8,
    pub i2c_scl_bit: u8,
    pub i2c_sda_bit: u8,
    pub spi_mode: u8,
    pub spi_clk_bit: u8,
    pub spi_mosi_bit: u8,
    pub spi_miso_bit: u8,
    pub spi_cs_bit: u8,
}

impl Default for DecoderState {
    fn default() -> Self {
        Self {
            kind: DecoderKind::None,
            decoder: None,
            annotations: Vec::new(),
            decoded_up_to: 0,
            dirty: false,
            uart_baud: 115_200,
            uart_rx_bit: 0,
            i2c_scl_bit: 0,
            i2c_sda_bit: 1,
            spi_mode: 0,
            spi_clk_bit: 0,
            spi_mosi_bit: 1,
            spi_miso_bit: 2,
            spi_cs_bit: 3,
        }
    }
}

// Manual Clone because `Box<dyn Decoder>` isn't Clone.
impl Clone for DecoderState {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            decoder: None, // rebuilt on demand
            annotations: self.annotations.clone(),
            decoded_up_to: 0,
            dirty: true, // force rebuild
            uart_baud: self.uart_baud,
            uart_rx_bit: self.uart_rx_bit,
            i2c_scl_bit: self.i2c_scl_bit,
            i2c_sda_bit: self.i2c_sda_bit,
            spi_mode: self.spi_mode,
            spi_clk_bit: self.spi_clk_bit,
            spi_mosi_bit: self.spi_mosi_bit,
            spi_miso_bit: self.spi_miso_bit,
            spi_cs_bit: self.spi_cs_bit,
        }
    }
}

impl DecoderState {
    /// Rebuild the decoder from the current `kind` + parameters, clearing
    /// cached annotations.
    pub fn rebuild(&mut self) {
        self.decoder = match self.kind {
            DecoderKind::None => None,
            DecoderKind::Uart => Some(Box::new(UartDecoder::new(UartConfig {
                rx_bit: self.uart_rx_bit,
                baud_rate: self.uart_baud,
                ..Default::default()
            }))),
            DecoderKind::I2c => Some(Box::new(I2cDecoder::new(I2cConfig {
                scl_bit: self.i2c_scl_bit,
                sda_bit: self.i2c_sda_bit,
            }))),
            DecoderKind::Spi => Some(Box::new(SpiDecoder::new(SpiConfig {
                clk_bit: self.spi_clk_bit,
                mosi_bit: self.spi_mosi_bit,
                miso_bit: self.spi_miso_bit,
                cs_bit: self.spi_cs_bit,
                mode: self.spi_mode,
                ..Default::default()
            }))),
        };
        self.annotations.clear();
        self.decoded_up_to = 0;
        self.dirty = false;
    }

    /// Feed new digital samples to the decoder, appending any new annotations
    /// to `self.annotations`.
    pub fn feed(&mut self, dt: &DigitalTrace) {
        if self.dirty {
            self.rebuild();
        }
        if let Some(dec) = &mut self.decoder {
            let words = dt.store().words();
            let rate = dt.timebase().sample_rate_hz();
            if self.decoded_up_to < words.len() {
                let new_anns = dec.feed(&words[self.decoded_up_to..], self.decoded_up_to, rate);
                self.annotations.extend(new_anns);
                self.decoded_up_to = words.len();
            }
        }
    }

    /// Convenience: feed the decoder from a `DeviceHandle`'s digital trace.
    pub fn update_from_handle(&mut self, handle: &DeviceHandle) {
        if self.dirty {
            self.rebuild();
        }
        if let Some(dt) = handle.digital_trace() {
            self.feed(dt);
        }
    }
}

/// Decoder kind selector and per-decoder configuration controls.
#[component]
pub fn DecoderConfig(view: Signal<WaveformView>) -> Element {
    let v = view.read();
    let kind_label = v.decoder.kind.label().to_string();
    let uart_baud = v.decoder.uart_baud;
    let uart_rx_bit = v.decoder.uart_rx_bit;
    let i2c_scl_bit = v.decoder.i2c_scl_bit;
    let i2c_sda_bit = v.decoder.i2c_sda_bit;
    let spi_mode = v.decoder.spi_mode;
    drop(v);

    rsx! {
        div { class: "flex flex-wrap items-center gap-2 text-xs",

            label { class: "text-zinc-500", "Decoder:" }
            select {
                class: "bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                onchange: {
                    let mut view = view;
                    move |evt| {
                        let kind = match evt.value().as_str() {
                            "UART" => DecoderKind::Uart,
                            "I2C" => DecoderKind::I2c,
                            "SPI" => DecoderKind::Spi,
                            _ => DecoderKind::None,
                        };
                        let mut v = view.write();
                        if v.decoder.kind != kind {
                            v.decoder.kind = kind;
                            v.decoder.dirty = true;
                        }
                    }
                },
                option { value: "None", selected: kind_label == "None", "None" }
                option { value: "UART", selected: kind_label == "UART", "UART" }
                option { value: "I2C", selected: kind_label == "I\u{00B2}C", "I\u{00B2}C" }
                option { value: "SPI", selected: kind_label == "SPI", "SPI" }
            }

            match kind_label.as_str() {
                "UART" => rsx! {
                    label { class: "text-zinc-500 ml-1", "Baud:" }
                    input {
                        class: "w-20 bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                        r#type: "number",
                        value: "{uart_baud}",
                        min: "300",
                        max: "4000000",
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<u32>() {
                                let mut v = view.write();
                                if v.decoder.uart_baud != val {
                                    v.decoder.uart_baud = val;
                                    v.decoder.dirty = true;
                                }
                            }
                        },
                    }
                    label { class: "text-zinc-500 ml-1", "RX bit:" }
                    input {
                        class: "w-14 bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                        r#type: "number",
                        value: "{uart_rx_bit}",
                        min: "0",
                        max: "63",
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<u8>() {
                                let mut v = view.write();
                                if v.decoder.uart_rx_bit != val {
                                    v.decoder.uart_rx_bit = val;
                                    v.decoder.dirty = true;
                                }
                            }
                        },
                    }
                },
                "I2C" | "I\u{00B2}C" => rsx! {
                    label { class: "text-zinc-500 ml-1", "SCL:" }
                    input {
                        class: "w-14 bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                        r#type: "number",
                        value: "{i2c_scl_bit}",
                        min: "0",
                        max: "63",
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<u8>() {
                                let mut v = view.write();
                                if v.decoder.i2c_scl_bit != val {
                                    v.decoder.i2c_scl_bit = val;
                                    v.decoder.dirty = true;
                                }
                            }
                        },
                    }
                    label { class: "text-zinc-500 ml-1", "SDA:" }
                    input {
                        class: "w-14 bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                        r#type: "number",
                        value: "{i2c_sda_bit}",
                        min: "0",
                        max: "63",
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<u8>() {
                                let mut v = view.write();
                                if v.decoder.i2c_sda_bit != val {
                                    v.decoder.i2c_sda_bit = val;
                                    v.decoder.dirty = true;
                                }
                            }
                        },
                    }
                },
                "SPI" => rsx! {
                    label { class: "text-zinc-500 ml-1", "Mode:" }
                    input {
                        class: "w-12 bg-zinc-800 border border-zinc-700 text-zinc-200 rounded px-1 py-0.5 text-xs",
                        r#type: "number",
                        value: "{spi_mode}",
                        min: "0",
                        max: "3",
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<u8>() {
                                let mut v = view.write();
                                if v.decoder.spi_mode != val {
                                    v.decoder.spi_mode = val;
                                    v.decoder.dirty = true;
                                }
                            }
                        },
                    }
                },
                _ => rsx! {},
            }
        }
    }
}
