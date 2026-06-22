//! Thin SCPI helper layer: query/write, timeouts and `*IDN?` parsing reused
//! by SCPI-speaking drivers.  Will be reconnected to the transport layer when
//! SCPI support is implemented.

pub const CRATE: &str = "rb-scpi";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-scpi");
    }
}
