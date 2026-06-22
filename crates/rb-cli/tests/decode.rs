//! Tests for the `decode` subcommand.

#[test]
fn decode_stub_returns_info_message() {
    let mut out = Vec::new();
    rb_cli::run_decode(
        std::path::Path::new("test.rbc"),
        None,
        None,
        "text",
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(
        s.contains("not yet implemented"),
        "decode stub should say 'not yet implemented': {s}"
    );
}
