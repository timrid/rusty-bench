//! Thin SCPI helper layer over the `Transport` trait: query/write, timeouts and
//! `*IDN?` parsing reused by SCPI-speaking drivers.

pub const CRATE: &str = "rb-scpi";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE, "rb-scpi");
    }
}
