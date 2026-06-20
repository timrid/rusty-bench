//! Protocol decoders for RustyBench logic-analyzer captures.
//!
//! Decoders are streaming, stacking state machines written clean-room from open
//! protocol specifications (no GPLv3 sigrok source). They compile to wasm.

pub const CRATE: &str = "rb-decode";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-decode");
    }
}
