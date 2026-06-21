# RustyBench

A vendor-independent, pure-Rust suite of electronics bench tools — logic analyzer,
multimeter, oscilloscope, power supply, waveform generator, SDR receiver, spectrum
analyzer and electronic load — with a scriptable CLI and an [egui](https://github.com/emilk/egui)
GUI that runs **natively and in the browser**.

> Status: fx2lafw device support (milestone **M8**). See [`BACKLOG.md`](BACKLOG.md) for the
> roadmap and [`UBIQUITOUS_LANGUAGE.md`](UBIQUITOUS_LANGUAGE.md) for the domain glossary.

## Workspace layout

| Crate           | Responsibility                                                        |
| --------------- | -------------------------------------------------------------------- |
| `rb-model`      | Core data types, sample store, mip-map. **No I/O, no runtime.**       |
| `rb-device`     | Device + capability traits. **No I/O, no runtime.**                   |
| `rb-transport`  | Transport trait + platform implementations (USB/Serial/… as features) |
| `rb-decode`     | Stacking protocol decoders (UART, I²C, SPI, …)                        |
| `rb-scpi`       | Thin SCPI helper layer over `Transport`                              |
| `rb-drivers`    | Device drivers, one module each, behind Cargo features               |
| `rb-core`       | Session, driver registry, runtime glue                              |
| `rb-cli`        | Headless, scriptable single-shot tool                               |
| `rb-gui`        | Platform-neutral eframe application                                 |
| `rb-gui-native` | Native GUI entrypoint                                                |
| `rb-gui-web`    | Web (WASM) GUI entrypoint                                            |

`rb-model` and `rb-device` must compile to `wasm32-unknown-unknown` with no feature flags.

## Build & run

```sh
# Native
cargo build --workspace
cargo test --workspace
cargo run -p rb-gui-native

# Web (requires trunk: cargo install trunk)
cd crates/rb-gui-web && trunk serve

# Verify wasm compatibility of the whole workspace
cargo build --workspace --target wasm32-unknown-unknown
```

## License

MIT — see [`LICENSE`](LICENSE). Drivers and decoders are written clean-room from open
protocol specifications; no code is derived from GPLv3 sigrok sources.
