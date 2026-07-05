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

## End-to-End Tests

The GUI has a [Playwright](https://playwright.dev/)-based E2E test suite that runs
against the web target (`dx serve --platform web`). Tests are written in Python with
[pytest-playwright](https://playwright.dev/python/) and managed by [uv](https://docs.astral.sh/uv/).

### Prerequisites

- **uv** ([install](https://docs.astral.sh/uv/getting-started/installation/))
- Install Playwright browsers (one-time):
  ```sh
  uv run playwright install chromium
  ```

### Running the tests

```sh
# Run all E2E tests (starts dx serve automatically)
uv run pytest

# Run with visible browser
uv run pytest --headed

# Run a specific test
uv run pytest -k "test_theme_toggle"

# Show the HTML report after a run
uv run playwright show-report
```

The first run builds the WASM artifact, which takes several minutes. Subsequent
runs reuse an already-running dev server.

### CI mode

In CI, `dx serve` is replaced with a one-shot `dx run` to avoid keeping a
long-lived server:

```sh
# Start the server in one-shot mode (builds + serves + exits when done)
dx run --force-sequential --web --addr 127.0.0.1 --port 9990 --release

# Run tests against it (in another terminal)
uv run pytest
```

Set `CI=1` to force the test suite to start its own `dx serve` instance instead
of reusing an existing one.



## License

MIT — see [`LICENSE`](LICENSE). Drivers and decoders are written clean-room from open
protocol specifications; no code is derived from GPLv3 sigrok sources.
