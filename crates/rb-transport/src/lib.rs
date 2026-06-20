//! Transport abstraction for RustyBench.
//!
//! Defines the byte/packet-oriented `Transport` trait that drivers speak against.
//! Concrete platform implementations (USB/Serial/Bluetooth/Ethernet on native,
//! WebUSB/WebSerial/WebBluetooth on web) are added behind Cargo features.

pub const CRATE: &str = "rb-transport";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-transport");
    }
}
