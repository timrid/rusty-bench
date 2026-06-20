//! Core data model for RustyBench: sample types, channels, timebase, the sample
//! store and its multi-resolution mip-map.
//!
//! This crate is intentionally free of I/O and async-runtime dependencies so it
//! compiles unchanged to native and `wasm32-unknown-unknown`.

/// Crate name, used as a placeholder until real types land in M1.
pub const CRATE: &str = "rb-model";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-model");
    }
}
