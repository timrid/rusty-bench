//! Tests for the `info` subcommand.

#[test]
fn info_prints_capabilities() {
    let mut out = Vec::new();
    rb_cli::run_info("demo:0", &mut out, false).unwrap();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("Demo Device"), "missing model");
    assert!(s.contains("Oscilloscope"), "missing Oscilloscope class");
    assert!(s.contains("LogicAnalyzer"), "missing LogicAnalyzer class");
    assert!(s.contains("A0"), "missing analog channel A0");
    assert!(s.contains("D0"), "missing digital channel D0");
    assert!(s.contains("1000000"), "missing sample rate");
}

#[test]
fn info_json_includes_all_capabilities() {
    let mut out = Vec::new();
    rb_cli::run_info("demo:0", &mut out, true).unwrap();
    let s = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&s).expect("info --json should be valid JSON");

    assert_eq!(parsed["vendor"], "RustyBench");
    assert_eq!(parsed["address"], "demo:0");
    let classes = parsed["classes"]
        .as_array()
        .expect("classes should be array");
    let names: Vec<&str> = classes.iter().map(|c| c.as_str().unwrap()).collect();
    assert!(
        names.contains(&"Oscilloscope"),
        "should include Oscilloscope class"
    );
    assert!(
        names.contains(&"LogicAnalyzer"),
        "should include LogicAnalyzer class"
    );

    let caps = &parsed["capabilities"];
    assert!(
        caps["oscilloscope"].is_object(),
        "oscilloscope capabilities present"
    );
    assert!(
        caps["logic_analyzer"].is_object(),
        "logic_analyzer capabilities present"
    );
    assert!(
        caps["multimeter"].is_null(),
        "multimeter should be null for demo device"
    );
    assert!(
        caps["power_supply"].is_null(),
        "power_supply should be null"
    );
    assert!(
        caps["waveform_generator"].is_null(),
        "waveform_generator should be null"
    );
    assert!(
        caps["electronic_load"].is_null(),
        "electronic_load should be null"
    );
}
