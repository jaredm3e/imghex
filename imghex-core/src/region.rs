//! Coarse-grained, non-overlapping byte regions used to color the hex view.
//!
//! A [`Region`] spans a contiguous, half-open byte range `[start, end)` and is
//! tagged with a [`RegionKind`] that drives its highlight color and legend
//! entry. Regions are format-independent: every format maps its structure onto
//! this shared vocabulary so the GUI never needs format-specific knowledge to
//! render highlights.

use crate::color::Rgba;

/// The semantic category of a region. Each kind has a stable color and label so
/// the legend and highlighting are consistent across formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum RegionKind {
    /// A fixed file header / magic-number block (e.g. `BITMAPFILEHEADER`).
    FileHeader,
    /// A format/info header describing dimensions, depth, compression, etc.
    InfoHeader,
    /// Bit-field masks or other auxiliary header extensions.
    ColorMasks,
    /// A color palette / color table (indexed color).
    Palette,
    /// Application/metadata segments carrying no pixel data (e.g. JPEG APPn,
    /// EXIF, comments).
    Metadata,
    /// Coding tables that parameterize compression (e.g. JPEG quantization and
    /// Huffman tables).
    Table,
    /// The raw pixel / sample data. For compressed formats this is the
    /// entropy-coded stream rather than directly addressable samples.
    PixelData,
    /// Bytes that are part of the file but not attributed to any structure
    /// (padding between sections, trailing data).
    Gap,
    /// Bytes past the end of what the parser could account for, or otherwise
    /// unrecognized.
    Unknown,
}

impl RegionKind {
    /// A short, human-readable label for legends and the sidebar.
    pub fn label(&self) -> &'static str {
        match self {
            RegionKind::FileHeader => "File header",
            RegionKind::InfoHeader => "Info header",
            RegionKind::ColorMasks => "Color masks",
            RegionKind::Palette => "Palette",
            RegionKind::Metadata => "Metadata",
            RegionKind::Table => "Coding tables",
            RegionKind::PixelData => "Pixel data",
            RegionKind::Gap => "Gap / padding",
            RegionKind::Unknown => "Unknown",
        }
    }

    /// The highlight color used to render bytes belonging to this kind.
    ///
    /// Colors are chosen to be distinct and to read well behind dark text.
    pub fn color(&self) -> Rgba {
        match self {
            RegionKind::FileHeader => Rgba::rgb(0x8E, 0xC0, 0x7C), // green
            RegionKind::InfoHeader => Rgba::rgb(0x7C, 0xA7, 0xC0), // blue
            RegionKind::ColorMasks => Rgba::rgb(0xC0, 0xA7, 0x7C), // tan
            RegionKind::Palette => Rgba::rgb(0xC0, 0x7C, 0xB8),    // magenta
            RegionKind::Metadata => Rgba::rgb(0xB0, 0x8C, 0xC8),   // violet
            RegionKind::Table => Rgba::rgb(0x6C, 0xC0, 0xB4),      // teal
            RegionKind::PixelData => Rgba::rgb(0xC0, 0xB0, 0x60),  // gold
            RegionKind::Gap => Rgba::rgb(0x88, 0x88, 0x88),        // gray
            RegionKind::Unknown => Rgba::rgb(0x66, 0x66, 0x66),    // dark gray
        }
    }
}

/// A contiguous, half-open `[start, end)` byte range of one [`RegionKind`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub kind: RegionKind,
    /// A specific name for this instance (e.g. "BITMAPINFOHEADER").
    pub name: String,
}

impl Region {
    pub fn new(start: usize, end: usize, kind: RegionKind, name: impl Into<String>) -> Self {
        debug_assert!(end >= start, "region end must be >= start");
        Self {
            start,
            end,
            kind,
            name: name.into(),
        }
    }

    /// Number of bytes covered by this region.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `offset` falls within `[start, end)`.
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }
}
