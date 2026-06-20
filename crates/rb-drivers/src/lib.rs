//! Device drivers for RustyBench.
//!
//! One module per driver, each gated behind a Cargo feature. An explicit registry
//! function collects the active `DriverFactory` implementations via `#[cfg(feature)]`.

pub const CRATE: &str = "rb-drivers";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-drivers");
    }
}
