//! JPEG/JFIF parser: marker-segment structure, not pixel decoding.
//!
//! Unlike BMP or Netpbm, JPEG stores pixels as DCT coefficients that are then
//! Huffman-entropy-coded, so there is no byte→pixel mapping to expose: this
//! parser leaves `pixel_info`/`palette` empty (the preview and bit-plane tools
//! stay dark, exactly as for a BMP compression the core cannot decode) and
//! instead decodes the *marker structure*. Every `0xFF <marker>` segment
//! becomes a colored region with decoded header fields — the useful view for a
//! hex editor: where the JFIF/EXIF metadata, the quantization and Huffman
//! tables, the frame header (dimensions), and the entropy-coded scan data live.
//!
//! Reference: a JPEG file is a sequence of marker segments framed by
//! SOI (`FFD8`) … EOI (`FFD9`). Standalone markers (SOI, EOI, RSTn, TEM) carry
//! no payload; every other marker is followed by a big-endian 2-byte length
//! (which counts itself but not the marker) and that many bytes of payload.
//! The entropy-coded scan data after an SOS segment is not length-prefixed: it
//! runs until the next marker that is neither a stuffed `FF 00` byte nor a
//! restart marker (`FFD0`–`FFD7`).

use crate::field::Field;
use crate::format::{ImageFormat, ParseError};
use crate::model::ParsedImage;
use crate::region::{Region, RegionKind};

/// The JPEG format parser. Zero-sized; state lives in [`ParsedImage`].
pub struct JpegFormat;

fn u16be(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

/// The JPEG zig-zag scan order: for each of the 64 positions in the coefficient
/// stream, the natural (row-major) index it occupies in the 8×8 block. A DQT
/// stores its quantization values in this order, so element `k` of the stream
/// scales the DCT coefficient at natural index `ZIGZAG[k]` (row `n / 8`, col
/// `n % 8`).
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Decode a DQT (Define Quantization Table) payload into fine-grained fields.
///
/// The payload holds one or more tables back to back. Each is a `Pq<<4 | Tq`
/// header byte — high nibble `Pq` is the element precision (0 = 8-bit, 1 =
/// 16-bit values), low nibble `Tq` is the table id (0–3) — followed by 64
/// quantization values in zig-zag order (64 bytes for 8-bit, 128 for 16-bit).
/// `base` is the file offset of the first payload byte. Returns the number of
/// complete tables decoded (for the summary). A truncated trailing table is
/// decoded as far as the payload allows and not counted.
fn decode_dqt(payload: &[u8], base: usize, fields: &mut Vec<Field>) -> usize {
    let mut i = 0usize;
    let mut tables = 0usize;
    while i < payload.len() {
        let pq_tq = payload[i];
        let precision = pq_tq >> 4; // 0 = 8-bit values, 1 = 16-bit values
        let table_id = pq_tq & 0x0F;
        let bytes_per = if precision == 0 { 1 } else { 2 };
        let header_off = base + i;
        fields.push(Field::new(
            header_off,
            header_off + 1,
            "quant_table",
            format!(
                "table {table_id}, {}-bit",
                if precision == 0 { 8 } else { 16 }
            ),
            "Quantization table header: high nibble is element precision (0 = 8-bit, 1 = 16-bit), low nibble is the table id (0–3).",
        ));
        i += 1;
        // 64 quantization steps follow, stored in zig-zag scan order.
        for (k, &n) in ZIGZAG.iter().enumerate() {
            if i + bytes_per > payload.len() {
                // Truncated table: stop without counting it.
                return tables;
            }
            let start = base + i;
            let value = if bytes_per == 1 {
                payload[i] as u16
            } else {
                u16be(payload, i)
            };
            let (row, col) = (n / 8, n % 8);
            fields.push(Field::new(
                start,
                start + bytes_per,
                format!("q[{row}][{col}]"),
                format!("{value}"),
                format!(
                    "Quantization step for the DCT coefficient at row {row}, col {col} (zig-zag position {k})."
                ),
            ));
            i += bytes_per;
        }
        tables += 1;
    }
    tables
}

/// Decode a DHT (Define Huffman Table) payload into fine-grained fields.
///
/// The payload holds one or more tables back to back. Each is a `Tc<<4 | Th`
/// header byte — high nibble `Tc` is the table class (0 = DC / lossless, 1 =
/// AC), low nibble `Th` is the table id (0–3) — followed by 16 bytes giving the
/// number of Huffman codes of each length 1..16, then `sum(counts)` symbol
/// bytes (the values those codes map to, in order of increasing code length).
/// `base` is the file offset of the first payload byte. Returns the `(class,
/// id)` of every complete table decoded (for the summary). A truncated trailing
/// table is decoded as far as the payload allows and not returned.
fn decode_dht(payload: &[u8], base: usize, fields: &mut Vec<Field>) -> Vec<(u8, u8)> {
    let mut i = 0usize;
    let mut tables = Vec::new();
    while i < payload.len() {
        let tc_th = payload[i];
        let class = tc_th >> 4; // 0 = DC (or lossless), 1 = AC
        let table_id = tc_th & 0x0F;
        let class_name = match class {
            0 => "DC",
            1 => "AC",
            _ => "?",
        };
        let header_off = base + i;
        fields.push(Field::new(
            header_off,
            header_off + 1,
            "huff_table",
            format!("{class_name} table {table_id}"),
            "Huffman table header: high nibble is the table class (0 = DC, 1 = AC), low nibble is the table id (0–3).",
        ));
        i += 1;
        // 16 code-length counts: counts[n] = number of codes of length n+1.
        if i + 16 > payload.len() {
            // Truncated before the full count list: stop without counting it.
            return tables;
        }
        let counts = &payload[i..i + 16];
        let total: usize = counts.iter().map(|&c| c as usize).sum();
        fields.push(Field::new(
            base + i,
            base + i + 16,
            "code_counts",
            format!("{total} symbols"),
            "Sixteen bytes giving the number of Huffman codes of each length 1..16; they sum to the number of symbols that follow.",
        ));
        i += 16;
        // `total` symbol bytes, one field each, in order of increasing code
        // length. The value each Huffman code decodes to.
        for k in 0..total {
            if i >= payload.len() {
                // Truncated symbol list: stop without counting this table.
                return tables;
            }
            let start = base + i;
            let symbol = payload[i];
            fields.push(Field::new(
                start,
                start + 1,
                format!("symbol[{k}]"),
                format!("0x{symbol:02X}"),
                format!("Symbol #{k} in the Huffman table (the value this code maps to)."),
            ));
            i += 1;
        }
        tables.push((class, table_id));
    }
    tables
}

/// Does this marker stand alone (no length + payload)? SOI/EOI/TEM/RSTn do.
fn is_standalone(marker: u8) -> bool {
    matches!(marker, 0xD8 | 0xD9 | 0x01) || (0xD0..=0xD7).contains(&marker)
}

/// SOF (Start Of Frame) markers carry the image dimensions. All of
/// `0xC0..=0xCF` are SOF markers except DHT (0xC4), JPG (0xC8) and DAC (0xCC).
fn is_sof(marker: u8) -> bool {
    (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// A short name and human-readable description for a marker byte.
fn marker_info(marker: u8) -> (&'static str, &'static str) {
    match marker {
        0xD8 => ("SOI", "Start of Image"),
        0xD9 => ("EOI", "End of Image"),
        0xC0 => ("SOF0", "Start of Frame — baseline DCT"),
        0xC1 => ("SOF1", "Start of Frame — extended sequential DCT"),
        0xC2 => ("SOF2", "Start of Frame — progressive DCT"),
        0xC3 => ("SOF3", "Start of Frame — lossless (sequential)"),
        0xC5 => ("SOF5", "Start of Frame — differential sequential DCT"),
        0xC6 => ("SOF6", "Start of Frame — differential progressive DCT"),
        0xC7 => ("SOF7", "Start of Frame — differential lossless"),
        0xC9 => (
            "SOF9",
            "Start of Frame — extended sequential DCT, arithmetic",
        ),
        0xCA => ("SOF10", "Start of Frame — progressive DCT, arithmetic"),
        0xCB => ("SOF11", "Start of Frame — lossless, arithmetic"),
        0xCD => (
            "SOF13",
            "Start of Frame — differential sequential DCT, arithmetic",
        ),
        0xCE => (
            "SOF14",
            "Start of Frame — differential progressive DCT, arithmetic",
        ),
        0xCF => (
            "SOF15",
            "Start of Frame — differential lossless, arithmetic",
        ),
        0xC4 => ("DHT", "Define Huffman Table(s)"),
        0xC8 => ("JPG", "JPEG extensions"),
        0xCC => ("DAC", "Define Arithmetic Coding conditioning"),
        0xDB => ("DQT", "Define Quantization Table(s)"),
        0xDA => ("SOS", "Start of Scan"),
        0xDD => ("DRI", "Define Restart Interval"),
        0xDC => ("DNL", "Define Number of Lines"),
        0xDE => ("DHP", "Define Hierarchical Progression"),
        0xDF => ("EXP", "Expand Reference Component(s)"),
        0xFE => ("COM", "Comment"),
        0x01 => ("TEM", "Temporary (used only in arithmetic coding)"),
        0xE0..=0xEF => ("APPn", "Application-specific segment"),
        0xD0..=0xD7 => ("RSTn", "Restart marker"),
        _ => ("?", "Unknown marker"),
    }
}

/// Map a marker to the coarse region color/legend category.
fn region_kind(marker: u8) -> RegionKind {
    match marker {
        0xD8 | 0xD9 => RegionKind::FileHeader,
        0xE0..=0xEF | 0xFE => RegionKind::Metadata,
        0xDB | 0xC4 | 0xDD | 0xCC | 0xDC => RegionKind::Table,
        0xDA => RegionKind::InfoHeader,
        m if is_sof(m) => RegionKind::InfoHeader,
        _ => RegionKind::Unknown,
    }
}

/// Scan for the byte offset of the next real marker at or after `start`, i.e. a
/// `0xFF` that is not fill (`FF FF`), not a stuffed literal (`FF 00`), and not a
/// restart marker (`FFD0`–`FFD7`). Returns `bytes.len()` if none is found.
fn find_next_marker(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == 0xFF {
            let n = bytes[i + 1];
            if n != 0x00 && n != 0xFF && !(0xD0..=0xD7).contains(&n) {
                return i;
            }
        }
        i += 1;
    }
    bytes.len()
}

impl ImageFormat for JpegFormat {
    fn name(&self) -> &'static str {
        "JPEG"
    }

    fn detect(&self, bytes: &[u8]) -> bool {
        bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8
    }

    fn parse(&self, bytes: &[u8]) -> Result<ParsedImage, ParseError> {
        if !self.detect(bytes) {
            return Err(ParseError::NotRecognized);
        }
        let len = bytes.len();
        let mut regions = Vec::new();
        let mut fields = Vec::new();

        // Facts gathered from headers, assembled into the summary at the end.
        let mut width = 0u32;
        let mut height = 0u32;
        let mut precision = 0u8;
        let mut components = 0u8;
        let mut sof_desc: Option<&'static str> = None;
        let mut jfif_version: Option<String> = None;
        let mut has_exif = false;
        let mut comment: Option<String> = None;
        let mut quant_tables = 0usize;
        let mut huff_tables: Vec<(u8, u8)> = Vec::new();
        let mut segment_count = 0usize;

        let mut pos = 0usize;
        while pos < len {
            if bytes[pos] != 0xFF {
                // A marker boundary should begin with 0xFF; anything else is
                // unaccounted-for data. Record it and stop walking.
                regions.push(Region::new(
                    pos,
                    len,
                    RegionKind::Unknown,
                    "Unrecognized data",
                ));
                break;
            }
            // Tolerate fill bytes: any run of 0xFF may precede a marker.
            let mut m = pos + 1;
            while m < len && bytes[m] == 0xFF {
                m += 1;
            }
            if m >= len {
                regions.push(Region::new(
                    pos,
                    len,
                    RegionKind::Gap,
                    "Trailing fill bytes",
                ));
                break;
            }
            let marker = bytes[m];
            let marker_start = m - 1; // the 0xFF immediately preceding the marker
            if marker_start > pos {
                // The extra 0xFF fill bytes before the marker.
                regions.push(Region::new(
                    pos,
                    marker_start,
                    RegionKind::Gap,
                    "Fill bytes",
                ));
            }

            let (short, desc) = marker_info(marker);
            let display_name = if (0xE0..=0xEF).contains(&marker) {
                format!("APP{}", marker - 0xE0)
            } else {
                short.to_string()
            };
            segment_count += 1;

            if is_standalone(marker) {
                let end = (m + 1).min(len);
                regions.push(Region::new(
                    marker_start,
                    end,
                    region_kind(marker),
                    display_name.clone(),
                ));
                fields.push(Field::new(
                    marker_start,
                    end,
                    "marker",
                    format!("FF{marker:02X}"),
                    format!("{display_name} — {desc}"),
                ));
                if marker == 0xD9 {
                    // End of image. Anything after it is trailing data.
                    if end < len {
                        regions.push(Region::new(
                            end,
                            len,
                            RegionKind::Unknown,
                            "Trailing data (after EOI)",
                        ));
                    }
                    break;
                }
                pos = end;
                continue;
            }

            // Length-prefixed segment. The 2-byte big-endian length counts
            // itself but not the marker.
            if m + 2 >= len {
                regions.push(Region::new(
                    marker_start,
                    len,
                    RegionKind::Unknown,
                    format!("{display_name} (truncated)"),
                ));
                break;
            }
            let seg_len = u16be(bytes, m + 1) as usize;
            let payload_start = m + 3;
            let seg_end = (m + 1 + seg_len).min(len);

            regions.push(Region::new(
                marker_start,
                seg_end,
                region_kind(marker),
                display_name.clone(),
            ));
            fields.push(Field::new(
                marker_start,
                m + 1,
                "marker",
                format!("FF{marker:02X}"),
                format!("{display_name} — {desc}"),
            ));
            fields.push(Field::new(
                m + 1,
                (m + 3).min(len),
                "length",
                format!("{seg_len}"),
                "Segment length in bytes (includes these two length bytes, excludes the marker).",
            ));

            let payload: &[u8] = if payload_start <= seg_end {
                &bytes[payload_start..seg_end]
            } else {
                &[]
            };

            if is_sof(marker) && payload.len() >= 6 {
                precision = payload[0];
                height = u16be(payload, 1) as u32;
                width = u16be(payload, 3) as u32;
                components = payload[5];
                sof_desc = Some(desc);
                fields.push(Field::new(
                    payload_start,
                    payload_start + 1,
                    "precision",
                    format!("{precision}"),
                    "Sample precision in bits per component.",
                ));
                fields.push(Field::new(
                    payload_start + 1,
                    payload_start + 3,
                    "height",
                    format!("{height}"),
                    "Image height in pixels (number of lines).",
                ));
                fields.push(Field::new(
                    payload_start + 3,
                    payload_start + 5,
                    "width",
                    format!("{width}"),
                    "Image width in pixels (samples per line).",
                ));
                fields.push(Field::new(
                    payload_start + 5,
                    payload_start + 6,
                    "components",
                    format!("{components}"),
                    "Number of image components (1 = grayscale, 3 = YCbCr color, 4 = CMYK/YCCK).",
                ));
            } else if marker == 0xE0 && payload.len() >= 7 && &payload[0..5] == b"JFIF\0" {
                let ver = format!("{}.{:02}", payload[5], payload[6]);
                jfif_version = Some(ver.clone());
                fields.push(Field::new(
                    payload_start,
                    payload_start + 5,
                    "identifier",
                    "JFIF",
                    "JFIF APP0 identifier (the bytes \"JFIF\\0\").",
                ));
                fields.push(Field::new(
                    payload_start + 5,
                    payload_start + 7,
                    "version",
                    ver,
                    "JFIF version (major.minor).",
                ));
            } else if marker == 0xE1 && payload.len() >= 6 && &payload[0..6] == b"Exif\0\0" {
                has_exif = true;
                fields.push(Field::new(
                    payload_start,
                    payload_start + 6,
                    "identifier",
                    "Exif",
                    "EXIF APP1 identifier (the bytes \"Exif\\0\\0\").",
                ));
            } else if marker == 0xFE {
                let text = String::from_utf8_lossy(payload).into_owned();
                if payload_start < seg_end {
                    fields.push(Field::new(
                        payload_start,
                        seg_end,
                        "comment",
                        text.clone(),
                        "Free-form comment text.",
                    ));
                }
                comment = Some(text);
            } else if marker == 0xDD && payload.len() >= 2 {
                let ri = u16be(payload, 0);
                fields.push(Field::new(
                    payload_start,
                    payload_start + 2,
                    "restart_interval",
                    format!("{ri}"),
                    "Number of MCUs between restart markers.",
                ));
            } else if marker == 0xDB {
                quant_tables += decode_dqt(payload, payload_start, &mut fields);
            } else if marker == 0xC4 {
                huff_tables.extend(decode_dht(payload, payload_start, &mut fields));
            }

            if marker == 0xDA {
                // Start of scan: the entropy-coded data that follows is not
                // length-prefixed and runs to the next non-restart marker.
                let scan_start = seg_end;
                let scan_end = find_next_marker(bytes, scan_start);
                if scan_end > scan_start {
                    regions.push(Region::new(
                        scan_start,
                        scan_end,
                        RegionKind::PixelData,
                        "Entropy-coded scan data",
                    ));
                }
                pos = scan_end;
                continue;
            }

            pos = seg_end;
        }

        let mut summary = Vec::new();
        summary.push((
            "Format".into(),
            match sof_desc {
                Some(d) => format!("JPEG ({d})"),
                None => "JPEG".into(),
            },
        ));
        if width > 0 || height > 0 {
            summary.push(("Dimensions".into(), format!("{width} × {height} px")));
        }
        if precision > 0 {
            summary.push((
                "Sample precision".into(),
                format!("{precision} bits/component"),
            ));
        }
        if components > 0 {
            let kind = match components {
                1 => " (grayscale)",
                3 => " (YCbCr color)",
                4 => " (CMYK/YCCK)",
                _ => "",
            };
            summary.push(("Components".into(), format!("{components}{kind}")));
        }
        if quant_tables > 0 {
            summary.push(("Quantization tables".into(), format!("{quant_tables}")));
        }
        if !huff_tables.is_empty() {
            // List the defined tables by class, e.g. "DC: 0, 1 · AC: 0, 1".
            let ids = |class: u8| {
                huff_tables
                    .iter()
                    .filter(|&&(c, _)| c == class)
                    .map(|&(_, id)| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut parts = Vec::new();
            let dc = ids(0);
            if !dc.is_empty() {
                parts.push(format!("DC: {dc}"));
            }
            let ac = ids(1);
            if !ac.is_empty() {
                parts.push(format!("AC: {ac}"));
            }
            if !parts.is_empty() {
                summary.push(("Huffman tables".into(), parts.join(" · ")));
            }
        }
        if let Some(v) = jfif_version {
            summary.push(("JFIF version".into(), v));
        }
        summary.push((
            "EXIF metadata".into(),
            if has_exif { "present" } else { "absent" }.into(),
        ));
        if let Some(c) = comment {
            summary.push(("Comment".into(), c));
        }
        summary.push((
            "Pixel storage".into(),
            "DCT coefficients, Huffman-coded (no direct byte→pixel mapping)".into(),
        ));
        summary.push(("Segments".into(), format!("{segment_count}")));
        summary.push(("Actual file size".into(), format!("{len} bytes")));

        regions.sort_by_key(|r| r.start);
        fields.sort_by_key(|f| f.start);

        Ok(ParsedImage {
            format_name: "JPEG".into(),
            regions,
            fields,
            summary,
            palette: Vec::new(),
            palette_info: None,
            // JPEG pixels are DCT-compressed: no per-byte pixel resolution.
            pixel_info: None,
        })
    }
}
