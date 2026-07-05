//! Tests for control subcommands — multimeter, power-supply, waveform-gen, electronic-load.

#[tokio::test]
async fn multimeter_fails_for_demo_device() {
    let result = rb_cli::run_multimeter("demo:0", &mut Vec::new(), false);
    assert!(result.is_err(), "demo device has no multimeter");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("does not support multimeter"),
        "error should mention missing multimeter: {msg}"
    );
}

#[tokio::test]
async fn multimeter_json_fails_for_demo_device() {
    let result = rb_cli::run_multimeter("demo:0", &mut Vec::new(), true);
    assert!(result.is_err(), "demo device has no multimeter (JSON)");
}

// ── power-supply ───────────────────────────────────────────────────────────

#[tokio::test]
async fn power_supply_fails_for_demo_device() {
    let result = rb_cli::run_power_supply("demo:0", None, None, None, &mut Vec::new(), false);
    assert!(result.is_err(), "demo has no power-supply capability");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("power-supply"),
        "error should mention power-supply: {msg}"
    );
}

#[tokio::test]
async fn power_supply_set_voltage_fails_for_demo() {
    let result = rb_cli::run_power_supply("demo:0", Some(5.0), None, None, &mut Vec::new(), false);
    assert!(result.is_err(), "demo has no power-supply (set voltage)");
}

// ── waveform-gen ───────────────────────────────────────────────────────────

#[tokio::test]
async fn waveform_gen_fails_for_demo_device() {
    let result = rb_cli::run_waveform_gen("demo:0", None, None, None, &mut Vec::new(), false);
    assert!(result.is_err(), "demo has no waveform-gen capability");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("waveform"),
        "error should mention waveform: {msg}"
    );
}

// ── electronic-load ─────────────────────────────────────────────────────────

#[tokio::test]
async fn electronic_load_fails_for_demo_device() {
    let result = rb_cli::run_electronic_load("demo:0", None, None, None, &mut Vec::new(), false);
    assert!(result.is_err(), "demo has no electronic-load capability");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("electronic-load"),
        "error should mention electronic-load: {msg}"
    );
}
