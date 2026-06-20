//! RustyBench core: the `Session` that owns connected devices, the driver registry
//! and the runtime glue that ties the model, devices, transports and decoders
//! together for the CLI and GUI front-ends.

pub const CRATE: &str = "rb-core";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-core");
    }
}
