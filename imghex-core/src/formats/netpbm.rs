//! Netpbm binary parser: P5 (grayscale, PGM) and P6 (RGB, PPM).
//!
//! This is the crate's second format, present mainly to demonstrate that the
//! `ImageFormat` abstraction is genuinely format-agnostic — the GUI renders it
//! with no changes. Binary Netpbm has an ASCII header
//! (`P6\n<width> <height>\n<maxval>\n`, comments allowed) followed by raw
//! samples. Only 8-bit samples (`maxval <= 255`) get pixel decoding.

use crate::field::Field;
use crate::format::{ImageFormat, ParseError};
use crate::model::{ParsedImage, PixelEncoding, PixelInfo};
use crate::region::{Region, RegionKind};

pub struct NetpbmFormat;

/// A token scanned from the ASCII header, with its byte span.
struct Token {
    text: String,
    start: usize,
    end: usize,
}

/// Scan whitespace-separated tokens (skipping `#` comments) starting at `pos`.
/// Returns the token and the offset just past it.
fn next_token(bytes: &[u8], mut pos: usize) -> Option<(Token, usize)> {
    loop {
        // Skip whitespace.
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip a comment to end of line.
        if pos < bytes.len() && bytes[pos] == b'#' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        break;
    }
    if pos >= bytes.len() {
        return None;
    }
    let start = pos;
    while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let text = String::from_utf8_lossy(&bytes[start..pos]).into_owned();
    Some((
        Token {
            text,
            start,
            end: pos,
        },
        pos,
    ))
}

impl ImageFormat for NetpbmFormat {
    fn name(&self) -> &'static str {
        "Netpbm"
    }

    fn detect(&self, bytes: &[u8]) -> bool {
        bytes.len() >= 2 && bytes[0] == b'P' && matches!(bytes[1], b'5' | b'6')
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedImage, ParseError> {
        if !self.detect(bytes) {
            return Err(ParseError::NotRecognized);
        }
        let is_rgb = bytes[1] == b'6';

        let mut regions = Vec::new();
        let mut fields = Vec::new();
        let mut summary = Vec::new();

        // Magic number occupies the first two bytes.
        fields.push(Field::new(
            0,
            2,
            "magic",
            format!("P{}", bytes[1] as char),
            if is_rgb {
                "P6 — binary RGB (PPM)"
            } else {
                "P5 — binary grayscale (PGM)"
            },
        ));

        // Header tokens after the magic: width, height, maxval.
        let (w_tok, pos) =
            next_token(bytes, 2).ok_or_else(|| ParseError::Malformed("missing width".into()))?;
        let (h_tok, pos) =
            next_token(bytes, pos).ok_or_else(|| ParseError::Malformed("missing height".into()))?;
        let (m_tok, pos) =
            next_token(bytes, pos).ok_or_else(|| ParseError::Malformed("missing maxval".into()))?;

        let width: u32 = w_tok
            .text
            .parse()
            .map_err(|_| ParseError::Malformed("bad width".into()))?;
        let height: u32 = h_tok
            .text
            .parse()
            .map_err(|_| ParseError::Malformed("bad height".into()))?;
        let maxval: u32 = m_tok
            .text
            .parse()
            .map_err(|_| ParseError::Malformed("bad maxval".into()))?;

        fields.push(Field::new(
            w_tok.start,
            w_tok.end,
            "width",
            format!("{width}"),
            "Image width in pixels.",
        ));
        fields.push(Field::new(
            h_tok.start,
            h_tok.end,
            "height",
            format!("{height}"),
            "Image height in pixels.",
        ));
        fields.push(Field::new(
            m_tok.start,
            m_tok.end,
            "maxval",
            format!("{maxval}"),
            "Maximum sample value (255 for 8-bit samples).",
        ));

        // Exactly one whitespace byte separates the header from the raster.
        let data_start = (pos + 1).min(bytes.len());
        regions.push(Region::new(
            0,
            data_start,
            RegionKind::InfoHeader,
            "Netpbm header",
        ));

        // Pixel data.
        let bytes_per_pixel = if is_rgb { 3 } else { 1 };
        let row_stride = width as usize * bytes_per_pixel;
        let computed = row_stride * height as usize;
        let data_end = (data_start + computed).min(bytes.len());
        if data_end > data_start {
            regions.push(Region::new(
                data_start,
                data_end,
                RegionKind::PixelData,
                "Pixel data",
            ));
        }
        if data_end < bytes.len() {
            regions.push(Region::new(
                data_end,
                bytes.len(),
                RegionKind::Unknown,
                "Trailing data",
            ));
        }

        // Only 8-bit samples map cleanly to one byte per channel.
        let pixel_info = if maxval > 0 && maxval <= 255 && width > 0 && height > 0 {
            Some(PixelInfo {
                data_start,
                width,
                height,
                top_down: true, // Netpbm rasters are top-to-bottom.
                row_stride,
                encoding: if is_rgb {
                    PixelEncoding::RgbDirect { bytes: 3 }
                } else {
                    PixelEncoding::Grayscale
                },
            })
        } else {
            None
        };

        summary.push((
            "Format".into(),
            if is_rgb {
                "Netpbm P6 (PPM, RGB)".into()
            } else {
                "Netpbm P5 (PGM, grayscale)".into()
            },
        ));
        summary.push(("Dimensions".into(), format!("{width} × {height} px")));
        summary.push(("Max sample value".into(), format!("{maxval}")));
        summary.push(("Row order".into(), "top-down".into()));
        summary.push(("Pixel data offset".into(), format!("{data_start}")));
        summary.push(("Actual file size".into(), format!("{} bytes", bytes.len())));

        regions.sort_by_key(|r| r.start);
        fields.sort_by_key(|f| f.start);

        Ok(ParsedImage {
            format_name: "Netpbm".into(),
            regions,
            fields,
            summary,
            palette: Vec::new(),
            palette_info: None,
            pixel_info,
        })
    }
}
