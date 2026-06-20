//! Device and capability abstractions for RustyBench.
//!
//! Defines the base `Device` trait and the per-class capability traits. Like
//! `rb-model`, this crate is free of I/O and runtime dependencies.

pub const CRATE: &str = "rb-device";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-device");
    }
}
