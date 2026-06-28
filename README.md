# RustyBench

A vendor-independent, pure-Rust suite of electronics bench tools — logic analyzer,
multimeter, oscilloscope, power supply, waveform generator, SDR receiver, spectrum
analyzer and electronic load — with a scriptable CLI and a [Dioxus](https://dioxuslabs.com/)
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
| `rb-gui`        | Platform-neutral Dioxus application (native + web)               |

`rb-model` and `rb-device` must compile to `wasm32-unknown-unknown` with no feature flags.

## Build & run

```sh
# Desktop
dx serve --platform desktop

# Web
dx serve --platform web

# Verify wasm compatibility of the whole workspace
cargo build --workspace --exclude rb-cli --target wasm32-unknown-unknown
```

## Tests

```sh
# All tests (model + canvas + GUI + waveform rendering)
cargo test --workspace

# Only rb-canvas (virtual canvas primitives)
cargo test -p rb-canvas

# Only waveform rendering tests (generates PNG screenshots in target/test-screenshots/)
cargo test -p rb-gui -- waveform

# Run waveform tests without capturing output (see which screenshots are saved)
cargo test -p rb-gui -- waveform --nocapture
```

## License

MIT — see [`LICENSE`](LICENSE). Drivers and decoders are written clean-room from open
protocol specifications; no code is derived from GPLv3 sigrok sources.
