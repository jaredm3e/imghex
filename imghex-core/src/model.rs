//! The parsed representation of an image file and per-offset decoding.
//!
//! [`ParsedImage`] is what a format parser produces: the coarse [`Region`]s for
//! coloring, the fine-grained [`Field`]s, a human-readable summary, and (when
//! applicable) a palette and [`PixelInfo`] describing how to interpret the
//! pixel-data region. Given any byte offset, [`ParsedImage::describe`] assembles
//! a [`SelectionInfo`] for the sidebar — this is where a byte in the pixel data
//! of an indexed image is resolved to its palette color, exactly as requested.

use crate::color::Rgba;
use crate::field::Field;
use crate::region::{Region, RegionKind};

/// How the bytes in the pixel-data region encode samples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PixelEncoding {
    /// Each pixel is an index into the palette. `bits` is 1, 4, or 8.
    Indexed { bits: u8 },
    /// Direct color stored as B, G, R (`bytes == 3`) or B, G, R, A
    /// (`bytes == 4`) — the ordering used by BMP.
    BgrDirect { bytes: u8 },
    /// Direct color stored as R, G, B (`bytes == 3`) or R, G, B, A
    /// (`bytes == 4`) — the ordering used by Netpbm/PNG.
    RgbDirect { bytes: u8 },
    /// One byte per pixel; the value is the gray level (8-bit samples).
    Grayscale,
}

impl PixelEncoding {
    /// Bytes consumed per pixel for direct/gray encodings (`None` for indexed,
    /// where a byte may hold several pixels).
    fn bytes_per_pixel(&self) -> Option<usize> {
        match self {
            PixelEncoding::Indexed { .. } => None,
            PixelEncoding::BgrDirect { bytes } | PixelEncoding::RgbDirect { bytes } => {
                Some(*bytes as usize)
            }
            PixelEncoding::Grayscale => Some(1),
        }
    }

    /// The channel name of byte `idx` within a pixel (direct/gray encodings).
    fn channel_name(&self, idx: usize) -> &'static str {
        match self {
            PixelEncoding::Grayscale => "Gray",
            PixelEncoding::BgrDirect { .. } => ["Blue", "Green", "Red", "Alpha"]
                .get(idx)
                .copied()
                .unwrap_or("?"),
            PixelEncoding::RgbDirect { .. } => ["Red", "Green", "Blue", "Alpha"]
                .get(idx)
                .copied()
                .unwrap_or("?"),
            PixelEncoding::Indexed { .. } => "?",
        }
    }

    /// Resolve the color of a direct/gray pixel that begins at `pixel_start`.
    fn read_direct(&self, pixel_start: usize, raw: &[u8]) -> Option<Rgba> {
        match self {
            PixelEncoding::Grayscale => {
                let v = *raw.get(pixel_start)?;
                Some(Rgba::rgb(v, v, v))
            }
            PixelEncoding::BgrDirect { bytes } => {
                let b = *raw.get(pixel_start)?;
                let g = *raw.get(pixel_start + 1)?;
                let r = *raw.get(pixel_start + 2)?;
                let a = if *bytes == 4 {
                    raw.get(pixel_start + 3).copied().unwrap_or(255)
                } else {
                    255
                };
                Some(Rgba::rgba(r, g, b, a))
            }
            PixelEncoding::RgbDirect { bytes } => {
                let r = *raw.get(pixel_start)?;
                let g = *raw.get(pixel_start + 1)?;
                let b = *raw.get(pixel_start + 2)?;
                let a = if *bytes == 4 {
                    raw.get(pixel_start + 3).copied().unwrap_or(255)
                } else {
                    255
                };
                Some(Rgba::rgba(r, g, b, a))
            }
            PixelEncoding::Indexed { .. } => None,
        }
    }
}

/// Everything needed to map a byte offset within the pixel-data region to a
/// pixel coordinate and (for indexed images) a palette index.
#[derive(Clone, Debug)]
pub struct PixelInfo {
    /// File offset where pixel data begins.
    pub data_start: usize,
    pub width: u32,
    /// Absolute image height (always positive; see `top_down`).
    pub height: u32,
    /// Whether row 0 is the top row. BMP is bottom-up unless height was
    /// stored negative.
    pub top_down: bool,
    /// Bytes per row, including padding to a 4-byte boundary.
    pub row_stride: usize,
    pub encoding: PixelEncoding,
}

/// A decoded pixel location and its resolved value.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelLocation {
    pub x: u32,
    pub y: u32,
    /// Palette index, for indexed encodings.
    pub palette_index: Option<u32>,
    /// The resolved color, when it can be determined.
    pub color: Option<Rgba>,
    /// Extra per-byte notes (e.g. "channel: Blue", "row padding").
    pub notes: Vec<(String, String)>,
}

/// One pixel encoded within a byte. A single byte holds several of these for
/// sub-byte indexed depths (8 for 1-bpp, 2 for 4-bpp).
#[derive(Clone, Debug, PartialEq)]
pub struct PixelSample {
    pub x: u32,
    pub y: u32,
    pub palette_index: Option<u32>,
    pub color: Option<Rgba>,
}

/// A named color chip for the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub label: String,
    pub color: Rgba,
}

/// A fully decoded image in top-down, row-major RGBA order.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height` pixels, row 0 at the top.
    pub pixels: Vec<Rgba>,
}

/// A color channel selectable for bit-plane analysis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Red,
    Green,
    Blue,
    /// Rec. 601 luminance.
    Luma,
}

impl RenderedImage {
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get((y * self.width + x) as usize).copied()
    }

    /// Extract one bit plane: `true` where bit `bit` (0 = LSB) of the chosen
    /// channel is set. Row-major, top-down, matching `pixels`. This is the
    /// core of LSB steganography visualization.
    pub fn bit_plane(&self, channel: Channel, bit: u8) -> Vec<bool> {
        self.pixels
            .iter()
            .map(|c| {
                let v = match channel {
                    Channel::Red => c.r,
                    Channel::Green => c.g,
                    Channel::Blue => c.b,
                    Channel::Luma => c.luminance(),
                };
                (v >> bit) & 1 == 1
            })
            .collect()
    }
}

impl PixelInfo {
    /// Resolve a pixel-data byte offset to a location. Returns `None` if the
    /// offset is not inside the pixel data.
    ///
    /// `palette` is consulted for indexed encodings; `raw` is the full file so
    /// direct-color pixels can read their neighboring channel bytes.
    pub fn locate(&self, offset: usize, raw: &[u8], palette: &[Rgba]) -> Option<PixelLocation> {
        if offset < self.data_start {
            return None;
        }
        let rel = offset - self.data_start;
        let row = rel / self.row_stride;
        if row >= self.height as usize {
            return None;
        }
        let col_byte = rel % self.row_stride;
        // The y coordinate depends on storage direction.
        let y = if self.top_down {
            row as u32
        } else {
            self.height - 1 - row as u32
        };

        let mut notes = Vec::new();

        match self.encoding {
            PixelEncoding::Indexed { bits } => {
                let byte = *raw.get(offset)?;
                // Pixels packed into this byte, left-to-right (MSB first).
                let pixels_per_byte = (8 / bits) as usize;
                let first_x = col_byte * pixels_per_byte;
                if first_x >= self.width as usize {
                    notes.push(("Region".into(), "row padding (beyond image width)".into()));
                    return Some(PixelLocation {
                        x: first_x as u32,
                        y,
                        palette_index: None,
                        color: None,
                        notes,
                    });
                }
                let index = match bits {
                    8 => byte as u32,
                    4 => {
                        notes.push((
                            "Packing".into(),
                            format!(
                                "2 px/byte — high nibble x={}, low nibble x={}",
                                first_x,
                                first_x + 1
                            ),
                        ));
                        (byte >> 4) as u32
                    }
                    1 => {
                        notes.push(("Packing".into(), "8 px/byte — MSB is leftmost pixel".into()));
                        (byte >> 7) as u32
                    }
                    _ => byte as u32,
                };
                let color = palette.get(index as usize).copied();
                Some(PixelLocation {
                    x: first_x as u32,
                    y,
                    palette_index: Some(index),
                    color,
                    notes,
                })
            }
            // Direct color / grayscale: one pixel spans `bpp` bytes.
            _ => {
                let bpp = self.encoding.bytes_per_pixel()?;
                let x = col_byte / bpp;
                if x >= self.width as usize {
                    notes.push(("Region".into(), "row padding (beyond image width)".into()));
                    return Some(PixelLocation {
                        x: x as u32,
                        y,
                        palette_index: None,
                        color: None,
                        notes,
                    });
                }
                let channel_idx = col_byte % bpp;
                notes.push((
                    "Channel".into(),
                    self.encoding.channel_name(channel_idx).into(),
                ));
                let pixel_start = self.data_start + row * self.row_stride + x * bpp;
                let color = self.encoding.read_direct(pixel_start, raw);
                Some(PixelLocation {
                    x: x as u32,
                    y,
                    palette_index: None,
                    color,
                    notes,
                })
            }
        }
    }

    /// The color of image pixel `(x, y)` (top-left origin), or `None` if out of
    /// bounds or undecodable. Used to render the whole image.
    pub fn color_at(&self, x: u32, y: u32, raw: &[u8], palette: &[Rgba]) -> Option<Rgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let file_row = if self.top_down {
            y as usize
        } else {
            (self.height - 1 - y) as usize
        };
        let row_start = self.data_start + file_row * self.row_stride;
        match self.encoding {
            PixelEncoding::Indexed { bits } => {
                let bits = bits as usize;
                let x = x as usize;
                let byte = *raw.get(row_start + x * bits / 8)?;
                let index = match bits {
                    8 => byte as usize,
                    4 => {
                        if x.is_multiple_of(2) {
                            (byte >> 4) as usize
                        } else {
                            (byte & 0x0F) as usize
                        }
                    }
                    1 => ((byte >> (7 - (x % 8))) & 0x1) as usize,
                    _ => byte as usize,
                };
                palette.get(index).copied()
            }
            _ => {
                let bpp = self.encoding.bytes_per_pixel()?;
                self.encoding.read_direct(row_start + x as usize * bpp, raw)
            }
        }
    }

    /// The file offset of the byte that encodes image pixel `(x, y)` (top-left
    /// origin). For sub-byte indexed depths this is the byte holding the pixel.
    pub fn byte_offset_of(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let file_row = if self.top_down {
            y as usize
        } else {
            (self.height - 1 - y) as usize
        };
        let row_start = self.data_start + file_row * self.row_stride;
        Some(match self.encoding {
            PixelEncoding::Indexed { bits } => row_start + (x as usize * bits as usize) / 8,
            _ => row_start + x as usize * self.encoding.bytes_per_pixel().unwrap_or(1),
        })
    }

    /// Every pixel encoded by the byte at `offset`. For sub-byte indexed depths
    /// this returns all packed pixels (e.g. 8 for 1-bpp); for 8-bpp and direct
    /// color it returns a single sample. Empty if `offset` is outside the data
    /// or falls in row padding.
    pub fn samples(&self, offset: usize, raw: &[u8], palette: &[Rgba]) -> Vec<PixelSample> {
        if offset < self.data_start {
            return Vec::new();
        }
        let rel = offset - self.data_start;
        let row = rel / self.row_stride;
        if row >= self.height as usize {
            return Vec::new();
        }
        let col_byte = rel % self.row_stride;
        let y = if self.top_down {
            row as u32
        } else {
            self.height - 1 - row as u32
        };
        let byte = match raw.get(offset) {
            Some(b) => *b,
            None => return Vec::new(),
        };

        let mut out = Vec::new();
        match self.encoding {
            PixelEncoding::Indexed { bits } => {
                let ppb = (8 / bits) as usize;
                for k in 0..ppb {
                    let x = col_byte * ppb + k;
                    if x >= self.width as usize {
                        break; // remaining bits are row padding
                    }
                    // Pixels are packed most-significant-bit first.
                    let index = match bits {
                        1 => ((byte >> (7 - k)) & 0x1) as u32,
                        4 => ((byte >> (4 * (1 - k))) & 0xF) as u32,
                        _ => byte as u32,
                    };
                    out.push(PixelSample {
                        x: x as u32,
                        y,
                        palette_index: Some(index),
                        color: palette.get(index as usize).copied(),
                    });
                }
            }
            // Direct color / grayscale: one pixel spans `bpp` bytes.
            _ => {
                if let Some(bpp) = self.encoding.bytes_per_pixel() {
                    let x = col_byte / bpp;
                    if x < self.width as usize {
                        let pixel_start = self.data_start + row * self.row_stride + x * bpp;
                        out.push(PixelSample {
                            x: x as u32,
                            y,
                            palette_index: None,
                            color: self.encoding.read_direct(pixel_start, raw),
                        });
                    }
                }
            }
        }
        out
    }
}

/// How a palette is laid out on disk.
#[derive(Clone, Debug)]
pub struct PaletteInfo {
    pub start: usize,
    /// Bytes per palette entry (4 for `RGBQUAD`, 3 for `RGBTRIPLE`).
    pub entry_size: usize,
    pub count: usize,
}

/// The complete parsed model of an image file.
#[derive(Clone, Debug)]
pub struct ParsedImage {
    pub format_name: String,
    /// Non-overlapping coarse regions, sorted by `start`.
    pub regions: Vec<Region>,
    /// Fine-grained leaf fields (headers). Non-overlapping, sorted by `start`.
    pub fields: Vec<Field>,
    /// Key/value facts for the summary panel.
    pub summary: Vec<(String, String)>,
    /// Resolved palette colors (empty for non-indexed images).
    pub palette: Vec<Rgba>,
    /// On-disk palette layout, if present.
    pub palette_info: Option<PaletteInfo>,
    /// Pixel-data interpretation, if the parser could determine it.
    pub pixel_info: Option<PixelInfo>,
}

/// The decoded description of a single selected byte, assembled for the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionInfo {
    pub offset: usize,
    pub byte: u8,
    pub region_kind: Option<RegionKind>,
    pub region_name: Option<String>,
    pub field: Option<Field>,
    /// Additional decoded key/value lines specific to this byte.
    pub details: Vec<(String, String)>,
    /// Color chips for this byte: a palette entry, or every pixel packed into
    /// the byte (multiple for 1- and 4-bpp images).
    pub swatches: Vec<Swatch>,
}

impl ParsedImage {
    /// The region containing `offset`, if any.
    pub fn region_at(&self, offset: usize) -> Option<&Region> {
        self.regions.iter().find(|r| r.contains(offset))
    }

    /// The innermost field containing `offset`, if any.
    pub fn field_at(&self, offset: usize) -> Option<&Field> {
        // Fields are non-overlapping; the smallest containing field wins in
        // case of any nesting.
        self.fields
            .iter()
            .filter(|f| f.contains(offset))
            .min_by_key(|f| f.len())
    }

    /// Assemble a full description of the byte at `offset`.
    ///
    /// `raw` is the complete file. Returns `None` if `offset` is out of bounds.
    pub fn describe(&self, offset: usize, raw: &[u8]) -> Option<SelectionInfo> {
        let byte = *raw.get(offset)?;
        let region = self.region_at(offset);
        let field = self.field_at(offset).cloned();

        let mut details = Vec::new();
        let mut swatches = Vec::new();

        // Palette-region decoding: which entry and channel this byte belongs to.
        if let (Some(r), Some(pal)) = (region, &self.palette_info) {
            if r.kind == RegionKind::Palette && field.is_none() {
                let rel = offset - pal.start;
                let entry = rel / pal.entry_size;
                let within = rel % pal.entry_size;
                let channel = match (pal.entry_size, within) {
                    (_, 0) => "Blue",
                    (_, 1) => "Green",
                    (_, 2) => "Red",
                    (4, 3) => "Reserved (0)",
                    _ => "?",
                };
                details.push(("Palette index".into(), format!("{entry}")));
                details.push(("Channel".into(), channel.into()));
                if let Some(c) = self.palette.get(entry) {
                    details.push(("Entry color".into(), c.to_hex()));
                    swatches.push(Swatch {
                        label: format!("Entry {entry}"),
                        color: *c,
                    });
                }
            }
        }

        // Pixel-data decoding: coordinate, packed pixels, resolved colors.
        if let (Some(r), Some(pi)) = (region, &self.pixel_info) {
            if r.kind == RegionKind::PixelData {
                if let Some(loc) = pi.locate(offset, raw, &self.palette) {
                    details.push(("Pixel (x, y)".into(), format!("({}, {})", loc.x, loc.y)));
                    if let Some(idx) = loc.palette_index {
                        details.push(("Palette index".into(), format!("{idx}")));
                    }
                    for note in &loc.notes {
                        details.push(note.clone());
                    }
                }
                // Every pixel encoded by this byte becomes a labeled swatch, so
                // 1- and 4-bpp bytes show all of their packed colors at once.
                for s in pi.samples(offset, raw, &self.palette) {
                    if let Some(c) = s.color {
                        let label = match s.palette_index {
                            Some(idx) => format!("({}, {}) · idx {}", s.x, s.y, idx),
                            None => format!("({}, {})", s.x, s.y),
                        };
                        swatches.push(Swatch { label, color: c });
                    }
                }
            }
        }

        Some(SelectionInfo {
            offset,
            byte,
            region_kind: region.map(|r| r.kind),
            region_name: region.map(|r| r.name.clone()),
            field,
            details,
            swatches,
        })
    }

    /// A single representative color for the byte at `offset`, used to paint the
    /// hex view in "pixel color" mode. Returns the palette entry color for a
    /// palette byte, or the first encoded pixel's color for a pixel byte.
    pub fn byte_color(&self, offset: usize, raw: &[u8]) -> Option<Rgba> {
        let region = self.region_at(offset)?;
        match region.kind {
            RegionKind::Palette => {
                let pal = self.palette_info.as_ref()?;
                let entry = offset.checked_sub(pal.start)? / pal.entry_size;
                self.palette.get(entry).copied()
            }
            RegionKind::PixelData => {
                let pi = self.pixel_info.as_ref()?;
                pi.samples(offset, raw, &self.palette)
                    .first()
                    .and_then(|s| s.color)
            }
            _ => None,
        }
    }

    /// Decode the whole image into a top-down RGBA buffer for preview and
    /// bit-plane analysis. Returns `None` if the format could not be decoded
    /// (e.g. a compression the parser does not handle). Missing/unresolved
    /// pixels fall back to opaque black.
    pub fn render(&self, raw: &[u8]) -> Option<RenderedImage> {
        let pi = self.pixel_info.as_ref()?;
        if pi.width == 0 || pi.height == 0 {
            return None;
        }
        let mut pixels = Vec::with_capacity((pi.width * pi.height) as usize);
        for y in 0..pi.height {
            for x in 0..pi.width {
                pixels.push(
                    pi.color_at(x, y, raw, &self.palette)
                        .unwrap_or(Rgba::rgb(0, 0, 0)),
                );
            }
        }
        Some(RenderedImage {
            width: pi.width,
            height: pi.height,
            pixels,
        })
    }
}
