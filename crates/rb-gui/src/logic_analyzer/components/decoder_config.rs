//! Decoder configuration widget: dropdown + per-kind parameter inputs.

use dioxus::prelude::*;

use crate::logic_analyzer::view::{DecoderKind, WaveformView};

/// Decoder kind selector and per-decoder configuration controls.
#[component]
pub fn DecoderConfig(view: Signal<WaveformView>) -> Element {
    let v = view.read();

    let kind_label = v.decoder_kind.label().to_string();
    let uart_baud = v.uart_baud;
    let uart_rx_bit = v.uart_rx_bit;
    let i2c_scl_bit = v.i2c_scl_bit;
    let i2c_sda_bit = v.i2c_sda_bit;
    let spi_mode = v.spi_mode;
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
                        if v.decoder_kind != kind {
                            v.decoder_kind = kind;
                            v.decoder_dirty = true;
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
                                if v.uart_baud != val {
                                    v.uart_baud = val;
                                    v.decoder_dirty = true;
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
                                if v.uart_rx_bit != val {
                                    v.uart_rx_bit = val;
                                    v.decoder_dirty = true;
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
                                if v.i2c_scl_bit != val {
                                    v.i2c_scl_bit = val;
                                    v.decoder_dirty = true;
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
                                if v.i2c_sda_bit != val {
                                    v.i2c_sda_bit = val;
                                    v.decoder_dirty = true;
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
                                if v.spi_mode != val {
                                    v.spi_mode = val;
                                    v.decoder_dirty = true;
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
