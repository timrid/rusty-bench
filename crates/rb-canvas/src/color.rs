//! RGBA color type with hex parsing, formatting, and alpha compositing.

/// 8-bit-per-channel RGBA color for the virtual canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    /// Fully opaque black.
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };
    /// Fully opaque white.
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };

    /// Create from 6-digit hex string like `"#facc15"`. Alpha is always 255.
    /// Panics on invalid input (test-only, so panics are fine).
    pub fn from_hex(s: &str) -> Self {
        assert_eq!(s.len(), 7, "hex color must be 7 chars: #RRGGBB, got {s:?}");
        assert_eq!(&s[0..1], "#", "hex color must start with #");
        let r = u8::from_str_radix(&s[1..3], 16).expect("invalid hex red");
        let g = u8::from_str_radix(&s[3..5], 16).expect("invalid hex green");
        let b = u8::from_str_radix(&s[5..7], 16).expect("invalid hex blue");
        Self { r, g, b, a: 255 }
    }

    /// Format as `"#RRGGBB"` hex string (drops alpha).
    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Return a new color with alpha scaled by `factor` (0.0–1.0).
    pub fn with_alpha(&self, factor: f64) -> Self {
        let a = (self.a as f64 * factor.clamp(0.0, 1.0)).round() as u8;
        Self { a, ..*self }
    }

    /// Alpha-composite `src` over `dst` (source-over, straight alpha).
    /// Returns the blended color (RGB premultiplied, alpha = resulting opacity).
    pub fn blend(src: Self, dst: Self) -> Self {
        if src.a == 0 {
            return dst;
        }
        if src.a == 255 {
            return src;
        }
        let src_a = src.a as f64 / 255.0;
        let dst_a = dst.a as f64 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a < 1e-6 {
            return Self::TRANSPARENT;
        }
        let inv_out = 1.0 / out_a;
        let r = ((src.r as f64 * src_a + dst.r as f64 * dst_a * (1.0 - src_a)) * inv_out).round() as u8;
        let g = ((src.g as f64 * src_a + dst.g as f64 * dst_a * (1.0 - src_a)) * inv_out).round() as u8;
        let b = ((src.b as f64 * src_a + dst.b as f64 * dst_a * (1.0 - src_a)) * inv_out).round() as u8;
        let a = (out_a * 255.0).round() as u8;
        Self { r, g, b, a }
    }

    /// Pack into a `u32` as `0xRRGGBBAA`.
    pub fn pack(&self) -> u32 {
        u32::from_be_bytes([self.r, self.g, self.b, self.a])
    }

    /// Unpack from a `u32` as `0xRRGGBBAA`.
    pub fn unpack(v: u32) -> Self {
        let [r, g, b, a] = v.to_be_bytes();
        Self { r, g, b, a }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_hex_parses_correctly() {
        let c = RgbaColor::from_hex("#facc15");
        assert_eq!(c.r, 0xfa);
        assert_eq!(c.g, 0xcc);
        assert_eq!(c.b, 0x15);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn to_hex_round_trips() {
        let c = RgbaColor::from_hex("#0d1117");
        assert_eq!(c.to_hex(), "#0d1117");
    }

    #[test]
    fn with_alpha_scales_opacity() {
        let c = RgbaColor::from_hex("#ffffff");
        let half = c.with_alpha(0.5);
        assert_eq!(half.a, 128);
        let zero = c.with_alpha(0.0);
        assert_eq!(zero.a, 0);
    }

    #[test]
    fn blend_opaque_over_transparent() {
        let bg = RgbaColor::TRANSPARENT;
        let fg = RgbaColor::from_hex("#ff0000");
        let result = RgbaColor::blend(fg, bg);
        assert_eq!(result, RgbaColor { r: 255, g: 0, b: 0, a: 255 });
    }

    #[test]
    fn blend_transparent_over_opaque() {
        let bg = RgbaColor::from_hex("#ffffff");
        let fg = RgbaColor::TRANSPARENT;
        let result = RgbaColor::blend(fg, bg);
        assert_eq!(result, bg);
    }

    #[test]
    fn blend_half_alpha_white_over_black_is_gray() {
        let bg = RgbaColor::BLACK;
        let fg = RgbaColor { r: 255, g: 255, b: 255, a: 128 };
        let result = RgbaColor::blend(fg, bg);
        // 50% white over black = 50% gray (opaque)
        assert_eq!(result.r, 128);
        assert_eq!(result.g, 128);
        assert_eq!(result.b, 128);
        assert_eq!(result.a, 255);
    }

    #[test]
    fn pack_unpack_round_trip() {
        let c = RgbaColor { r: 0x12, g: 0x34, b: 0x56, a: 0x78 };
        assert_eq!(RgbaColor::unpack(c.pack()), c);
    }
}
