//! BMP (Windows Bitmap) parser.
//!
//! Supports `BITMAPCOREHEADER` (12-byte) and `BITMAPINFOHEADER`-family headers
//! (40 bytes and the V4/V5 extensions). Indexed (1/4/8-bpp) and direct-color
//! (24/32-bpp) uncompressed images get full pixel decoding; other variants
//! (RLE, JPEG/PNG embedded, 16-bpp, bit-fields) are still mapped into regions
//! and headers but skip per-pixel color resolution.
//!
//! References: the on-disk layout is
//! `BITMAPFILEHEADER` (14 bytes) → DIB header → optional bit-field masks →
//! optional color table → pixel array (at `bfOffBits`).

use crate::color::Rgba;
use crate::field::Field;
use crate::format::{ImageFormat, ParseError};
use crate::model::{PaletteInfo, ParsedImage, PixelEncoding, PixelInfo};
use crate::region::{Region, RegionKind};

pub const FILE_HEADER_LEN: usize = 14;

/// The BMP format parser. Zero-sized; state lives in [`ParsedImage`].
pub struct BmpFormat;

// --- Little-endian readers (callers guarantee bounds). -----------------------

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn i32le(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn compression_name(c: u32) -> &'static str {
    match c {
        0 => "BI_RGB (uncompressed)",
        1 => "BI_RLE8",
        2 => "BI_RLE4",
        3 => "BI_BITFIELDS",
        4 => "BI_JPEG",
        5 => "BI_PNG",
        6 => "BI_ALPHABITFIELDS",
        11 => "BI_CMYK",
        12 => "BI_CMYKRLE8",
        13 => "BI_CMYKRLE4",
        _ => "unknown",
    }
}

impl ImageFormat for BmpFormat {
    fn name(&self) -> &'static str {
        "BMP"
    }

    fn detect(&self, bytes: &[u8]) -> bool {
        bytes.len() >= 2 && &bytes[0..2] == b"BM"
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedImage, ParseError> {
        if !self.detect(bytes) {
            return Err(ParseError::NotRecognized);
        }
        if bytes.len() < FILE_HEADER_LEN + 4 {
            return Err(ParseError::Truncated {
                needed: FILE_HEADER_LEN + 4,
                got: bytes.len(),
            });
        }

        let mut regions = Vec::new();
        let mut fields = Vec::new();
        let mut summary = Vec::new();

        // --- BITMAPFILEHEADER (14 bytes) -----------------------------------
        let bf_size = u32le(bytes, 2);
        let bf_off_bits = u32le(bytes, 10) as usize;

        regions.push(Region::new(
            0,
            FILE_HEADER_LEN,
            RegionKind::FileHeader,
            "BITMAPFILEHEADER",
        ));
        fields.push(Field::new(
            0,
            2,
            "bfType",
            "\"BM\"",
            "Magic number identifying a BMP file.",
        ));
        fields.push(Field::new(
            2,
            6,
            "bfSize",
            format!("{bf_size} bytes"),
            "Total size of the file in bytes.",
        ));
        fields.push(Field::new(
            6,
            8,
            "bfReserved1",
            format!("{}", u16le(bytes, 6)),
            "Reserved; typically 0.",
        ));
        fields.push(Field::new(
            8,
            10,
            "bfReserved2",
            format!("{}", u16le(bytes, 8)),
            "Reserved; typically 0.",
        ));
        fields.push(Field::new(
            10,
            14,
            "bfOffBits",
            format!("{bf_off_bits}"),
            "Byte offset from the start of the file to the pixel data.",
        ));

        // --- DIB header ----------------------------------------------------
        let dib_size = u32le(bytes, 14) as usize;
        let dib_end = FILE_HEADER_LEN + dib_size;
        if bytes.len() < FILE_HEADER_LEN + dib_size.min(4) {
            return Err(ParseError::Truncated {
                needed: dib_end,
                got: bytes.len(),
            });
        }

        let dib_name = match dib_size {
            12 => "BITMAPCOREHEADER",
            40 => "BITMAPINFOHEADER",
            52 => "BITMAPV2INFOHEADER",
            56 => "BITMAPV3INFOHEADER",
            64 => "OS22XBITMAPHEADER",
            108 => "BITMAPV4HEADER",
            124 => "BITMAPV5HEADER",
            _ => "DIB header",
        };
        regions.push(Region::new(
            FILE_HEADER_LEN,
            dib_end,
            RegionKind::InfoHeader,
            dib_name,
        ));

        // Fields common to all: parse width/height/bit depth per header kind.
        let width: i64;
        let height_raw: i64;
        let bit_count: u16;
        let mut compression: u32 = 0;
        let mut clr_used: u32 = 0;

        fields.push(Field::new(
            14,
            18,
            "biSize",
            format!("{dib_size} ({dib_name})"),
            "Size of this DIB header; identifies the header version.",
        ));

        if dib_size == 12 {
            // BITMAPCOREHEADER: 16-bit dimensions.
            if bytes.len() < 26 {
                return Err(ParseError::Truncated {
                    needed: 26,
                    got: bytes.len(),
                });
            }
            width = u16le(bytes, 18) as i64;
            height_raw = u16le(bytes, 20) as i64;
            bit_count = u16le(bytes, 24);
            fields.push(Field::new(
                18,
                20,
                "bcWidth",
                format!("{width}"),
                "Image width in pixels.",
            ));
            fields.push(Field::new(
                20,
                22,
                "bcHeight",
                format!("{height_raw}"),
                "Image height in pixels.",
            ));
            fields.push(Field::new(
                22,
                24,
                "bcPlanes",
                format!("{}", u16le(bytes, 22)),
                "Number of color planes; must be 1.",
            ));
            fields.push(Field::new(
                24,
                26,
                "bcBitCount",
                format!("{bit_count}"),
                "Bits per pixel.",
            ));
        } else {
            // BITMAPINFOHEADER and later share the first 40 bytes.
            if bytes.len() < 54 {
                return Err(ParseError::Truncated {
                    needed: 54,
                    got: bytes.len(),
                });
            }
            width = i32le(bytes, 18) as i64;
            height_raw = i32le(bytes, 22) as i64;
            bit_count = u16le(bytes, 28);
            compression = u32le(bytes, 30);
            clr_used = u32le(bytes, 46);

            fields.push(Field::new(
                18,
                22,
                "biWidth",
                format!("{width}"),
                "Image width in pixels.",
            ));
            fields.push(Field::new(
                22,
                26,
                "biHeight",
                format!("{height_raw}"),
                "Image height. Negative means the image is stored top-down.",
            ));
            fields.push(Field::new(
                26,
                28,
                "biPlanes",
                format!("{}", u16le(bytes, 26)),
                "Number of color planes; must be 1.",
            ));
            fields.push(Field::new(
                28,
                30,
                "biBitCount",
                format!("{bit_count}"),
                "Bits per pixel (color depth).",
            ));
            fields.push(Field::new(
                30,
                34,
                "biCompression",
                format!("{compression} — {}", compression_name(compression)),
                "Compression method used on the pixel data.",
            ));
            fields.push(Field::new(
                34,
                38,
                "biSizeImage",
                format!("{}", u32le(bytes, 34)),
                "Size of the raw pixel data; may be 0 for BI_RGB.",
            ));
            fields.push(Field::new(
                38,
                42,
                "biXPelsPerMeter",
                format!("{}", i32le(bytes, 38)),
                "Horizontal resolution in pixels per meter.",
            ));
            fields.push(Field::new(
                42,
                46,
                "biYPelsPerMeter",
                format!("{}", i32le(bytes, 42)),
                "Vertical resolution in pixels per meter.",
            ));
            fields.push(Field::new(
                46,
                50,
                "biClrUsed",
                format!("{clr_used}"),
                "Number of palette entries actually used (0 = maximum).",
            ));
            fields.push(Field::new(
                50,
                54,
                "biClrImportant",
                format!("{}", u32le(bytes, 50)),
                "Number of important colors (0 = all).",
            ));
        }

        let top_down = height_raw < 0;
        let height = height_raw.unsigned_abs() as u32;
        let width_u = width.max(0) as u32;

        // --- Optional bit-field masks (BI_BITFIELDS on a 40-byte header) ----
        let is_core = dib_size == 12;
        let mut cursor = dib_end;
        if !is_core && dib_size == 40 && (compression == 3 || compression == 6) {
            let mask_len = if compression == 6 { 16 } else { 12 };
            let mask_end = (cursor + mask_len).min(bytes.len());
            regions.push(Region::new(
                cursor,
                mask_end,
                RegionKind::ColorMasks,
                "Bit-field masks",
            ));
            if bytes.len() >= cursor + 12 {
                fields.push(Field::new(
                    cursor,
                    cursor + 4,
                    "RedMask",
                    format!("0x{:08X}", u32le(bytes, cursor)),
                    "Bit mask for the red channel.",
                ));
                fields.push(Field::new(
                    cursor + 4,
                    cursor + 8,
                    "GreenMask",
                    format!("0x{:08X}", u32le(bytes, cursor + 4)),
                    "Bit mask for the green channel.",
                ));
                fields.push(Field::new(
                    cursor + 8,
                    cursor + 12,
                    "BlueMask",
                    format!("0x{:08X}", u32le(bytes, cursor + 8)),
                    "Bit mask for the blue channel.",
                ));
            }
            cursor = mask_end;
        }

        // --- Color table / palette -----------------------------------------
        let entry_size = if is_core { 3 } else { 4 };
        let palette_start = cursor;
        // Number of entries: explicit biClrUsed, else the maximum for the depth.
        let max_entries = if bit_count <= 8 {
            1usize << bit_count
        } else {
            0
        };
        let entry_count = if clr_used > 0 {
            clr_used as usize
        } else {
            max_entries
        };

        let mut palette: Vec<Rgba> = Vec::new();
        let mut palette_info = None;
        if entry_count > 0 {
            // Clamp to what actually fits before the pixel data / end of file.
            let bound = bf_off_bits.max(palette_start).min(bytes.len());
            let available = bound.saturating_sub(palette_start);
            let fitting = (available / entry_size).min(entry_count);
            let palette_bytes = fitting * entry_size;
            let palette_end = palette_start + palette_bytes;

            if palette_bytes > 0 {
                regions.push(Region::new(
                    palette_start,
                    palette_end,
                    RegionKind::Palette,
                    "Color table",
                ));
                for i in 0..fitting {
                    let o = palette_start + i * entry_size;
                    let b = bytes[o];
                    let g = bytes[o + 1];
                    let r = bytes[o + 2];
                    palette.push(Rgba::rgb(r, g, b));
                }
                palette_info = Some(PaletteInfo {
                    start: palette_start,
                    entry_size,
                    count: fitting,
                });
            }
            cursor = palette_end;
        }

        // --- Gap between the color table and the pixel array ----------------
        if bf_off_bits > cursor && bf_off_bits <= bytes.len() {
            regions.push(Region::new(
                cursor,
                bf_off_bits,
                RegionKind::Gap,
                "Padding before pixel data",
            ));
        }

        // --- Pixel data -----------------------------------------------------
        let row_stride = if width_u > 0 {
            (bit_count as usize * width_u as usize).div_ceil(32) * 4
        } else {
            0
        };
        let data_start = bf_off_bits.min(bytes.len());
        // For uncompressed data we know the exact size; otherwise use the rest.
        let computed = row_stride * height as usize;
        let uncompressed = compression == 0 || compression == 3 || compression == 6;
        let data_len = if uncompressed && computed > 0 {
            computed.min(bytes.len().saturating_sub(data_start))
        } else {
            bytes.len().saturating_sub(data_start)
        };
        let data_end = data_start + data_len;
        if data_len > 0 {
            regions.push(Region::new(
                data_start,
                data_end,
                RegionKind::PixelData,
                "Pixel data",
            ));
        }

        // Pixel interpretation is only produced for uncompressed encodings we
        // can decode byte-for-byte.
        let encoding = match (bit_count, compression) {
            (1, 0) => Some(PixelEncoding::Indexed { bits: 1 }),
            (4, 0) => Some(PixelEncoding::Indexed { bits: 4 }),
            (8, 0) => Some(PixelEncoding::Indexed { bits: 8 }),
            (24, 0) => Some(PixelEncoding::BgrDirect { bytes: 3 }),
            (32, 0) | (32, 3) => Some(PixelEncoding::BgrDirect { bytes: 4 }),
            _ => None,
        };
        let pixel_info = encoding.and_then(|encoding| {
            if row_stride == 0 || height == 0 {
                return None;
            }
            Some(PixelInfo {
                data_start,
                width: width_u,
                height,
                top_down,
                row_stride,
                encoding,
            })
        });

        // --- Trailing bytes -------------------------------------------------
        if data_end < bytes.len() {
            regions.push(Region::new(
                data_end,
                bytes.len(),
                RegionKind::Unknown,
                "Trailing data",
            ));
        }

        // --- Summary --------------------------------------------------------
        summary.push(("Format".into(), format!("BMP ({dib_name})")));
        summary.push(("Dimensions".into(), format!("{width_u} × {height} px")));
        summary.push(("Bit depth".into(), format!("{bit_count} bpp")));
        if !is_core {
            summary.push(("Compression".into(), compression_name(compression).into()));
        }
        summary.push((
            "Row order".into(),
            if top_down {
                "top-down".into()
            } else {
                "bottom-up".into()
            },
        ));
        summary.push(("Row stride".into(), format!("{row_stride} bytes")));
        if !palette.is_empty() {
            summary.push(("Palette entries".into(), format!("{}", palette.len())));
        }
        summary.push(("Pixel data offset".into(), format!("{bf_off_bits}")));
        summary.push(("File size (bfSize)".into(), format!("{bf_size} bytes")));
        summary.push(("Actual file size".into(), format!("{} bytes", bytes.len())));

        // Keep regions sorted by start for predictable rendering/lookup.
        regions.sort_by_key(|r| r.start);
        fields.sort_by_key(|f| f.start);

        Ok(ParsedImage {
            format_name: "BMP".into(),
            regions,
            fields,
            summary,
            palette,
            palette_info,
            pixel_info,
        })
    }
}
