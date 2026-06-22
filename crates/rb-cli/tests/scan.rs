//! Tests for the `scan` subcommand.

#[test]
fn scan_finds_demo_device() {
    let mut out = Vec::new();
    rb_cli::run_scan(&mut out, false).unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("[demo]"), "missing driver tag");
    assert!(s.contains("demo:0"), "missing demo address");
    assert!(s.contains("RustyBench"), "missing vendor");
    assert!(s.contains("Demo Device"), "missing model");
}

#[test]
fn scan_json_output_is_valid() {
    let mut out = Vec::new();
    rb_cli::run_scan(&mut out, true).unwrap();
    let s = String::from_utf8(out).unwrap();
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&s).expect("scan --json should be valid JSON array");

    assert!(!parsed.is_empty(), "should list at least the demo device");
    let entry = &parsed[0];
    assert_eq!(
        entry["driver"], "demo",
        "first device should have driver tag"
    );
    assert_eq!(entry["address"], "demo:0", "demo address present");
    assert!(
        entry["model"].as_str().unwrap().contains("Demo"),
        "model should mention Demo"
    );
}
