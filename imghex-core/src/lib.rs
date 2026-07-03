//! `imghex-core` — a format-aware model powering a hex editor for image files.
//!
//! The crate is intentionally free of any GUI dependency. It turns a byte
//! stream into:
//!
//! * [`region::Region`]s — coarse, colored sections (header, palette, pixels…),
//! * [`field::Field`]s — fine-grained, decoded named values,
//! * a [`model::ParsedImage`] that, given any byte offset, produces a
//!   [`model::SelectionInfo`] describing exactly what that byte means —
//!   including resolving a pixel byte in an indexed image to its palette color.
//!
//! New formats implement [`format::ImageFormat`] and register themselves in
//! [`format::registry`]; nothing else in the stack needs to change.

pub mod color;
pub mod field;
pub mod fixtures;
pub mod format;
pub mod formats;
pub mod model;
pub mod region;
pub mod search;
pub mod stats;

pub use color::Rgba;
pub use field::Field;
pub use format::{parse_auto, ImageFormat, ParseError};
pub use model::{
    Channel, ParsedImage, PixelInfo, PixelSample, RenderedImage, SelectionInfo, Swatch,
};
pub use region::{Region, RegionKind};
pub use stats::ByteStats;

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn detects_bmp_magic() {
        let bmp = fixtures::demo_indexed();
        assert!(format::registry()[0].detect(&bmp));
        assert!(!format::registry()[0].detect(b"\x89PNG"));
    }

    #[test]
    fn rejects_non_bmp() {
        assert_eq!(
            parse_auto(b"not an image").unwrap_err(),
            ParseError::NotRecognized
        );
    }

    #[test]
    fn rgba_hex_formatting() {
        assert_eq!(Rgba::rgb(0xFF, 0x00, 0x80).to_hex(), "#FF0080");
        assert_eq!(Rgba::rgba(0x10, 0x20, 0x30, 0x40).to_hex(), "#10203040");
    }

    #[test]
    fn region_contains_is_half_open() {
        let r = Region::new(4, 8, RegionKind::Palette, "p");
        assert!(!r.contains(3));
        assert!(r.contains(4));
        assert!(r.contains(7));
        assert!(!r.contains(8));
        assert_eq!(r.len(), 4);
    }
}
