//! Annotation types produced by protocol decoders.

use core::ops::Range;

use serde::{Deserialize, Serialize};

/// Classification of a decoded event, used for rendering and stacking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationKind {
    /// A data byte: displayed as `0xNN`.
    Data,
    /// An address byte (I²C 7-bit address + R/W bit).
    Address,
    /// Protocol frame marker: START, STOP, chip-select assert/deassert.
    Frame,
    /// Error event: framing error, NACK, parity error.
    Error,
}

/// A decoded event covering a contiguous sample range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Half-open sample-index range `[start, end)` this annotation covers.
    pub range: Range<usize>,
    /// Human-readable label, e.g. `"0x41"`, `"ACK"`, `"START"`.
    pub label: String,
    /// Classification for rendering and stacking.
    pub kind: AnnotationKind,
    /// Raw byte value when `kind == Data`; `None` otherwise.
    ///
    /// Used by [`crate::StackedDecoder`] to pass decoded bytes to higher-level decoders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_byte: Option<u8>,
}

impl Annotation {
    /// Constructs a data-byte annotation with label `"0xNN"`.
    #[must_use]
    pub fn data(byte: u8, range: Range<usize>) -> Self {
        Self {
            range,
            label: format!("0x{byte:02X}"),
            kind: AnnotationKind::Data,
            data_byte: Some(byte),
        }
    }

    /// Constructs a protocol-frame annotation (START, STOP, etc.).
    #[must_use]
    pub fn frame(label: impl Into<String>, range: Range<usize>) -> Self {
        Self {
            range,
            label: label.into(),
            kind: AnnotationKind::Frame,
            data_byte: None,
        }
    }

    /// Constructs an error annotation.
    #[must_use]
    pub fn error(label: impl Into<String>, range: Range<usize>) -> Self {
        Self {
            range,
            label: label.into(),
            kind: AnnotationKind::Error,
            data_byte: None,
        }
    }

    /// Constructs an address annotation (e.g. I²C 7-bit address + R/W).
    #[must_use]
    pub fn address(byte: u8, label: impl Into<String>, range: Range<usize>) -> Self {
        Self {
            range,
            label: label.into(),
            kind: AnnotationKind::Address,
            data_byte: Some(byte),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_annotation_formats_hex() {
        let a = Annotation::data(0x41, 0..100);
        assert_eq!(a.label, "0x41");
        assert_eq!(a.kind, AnnotationKind::Data);
        assert_eq!(a.data_byte, Some(0x41));
    }

    #[test]
    fn frame_annotation_has_no_data_byte() {
        let a = Annotation::frame("START", 0..2);
        assert_eq!(a.kind, AnnotationKind::Frame);
        assert!(a.data_byte.is_none());
    }

    #[test]
    fn error_annotation_has_no_data_byte() {
        let a = Annotation::error("FRAME ERR 0x00", 0..10);
        assert_eq!(a.kind, AnnotationKind::Error);
        assert!(a.data_byte.is_none());
    }

    #[test]
    fn address_annotation_stores_byte() {
        let a = Annotation::address(0x42, "0x42 W", 5..50);
        assert_eq!(a.kind, AnnotationKind::Address);
        assert_eq!(a.data_byte, Some(0x42));
        assert_eq!(a.label, "0x42 W");
    }
}
