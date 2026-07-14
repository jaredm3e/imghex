//! Integration tests for the JPEG (JFIF) format. JPEG stores DCT-compressed,
//! entropy-coded pixels, so the parser exposes marker *structure* rather than
//! per-byte pixel colors — these tests pin that structure down.

use imghex_core::region::RegionKind;
use imghex_core::{fixtures, parse_auto};

#[test]
fn detects_and_names_jpeg() {
    let jpg = fixtures::jpeg_baseline(16, 8);
    let img = parse_auto(&jpg).unwrap();
    assert_eq!(img.format_name, "JPEG");

    // Not confused with BMP/Netpbm magic.
    assert!(parse_auto(b"BM....").is_err() || parse_auto(b"BM....").unwrap().format_name != "JPEG");
}

#[test]
fn soi_and_eoi_are_marked() {
    let jpg = fixtures::jpeg_baseline(16, 8);
    let img = parse_auto(&jpg).unwrap();

    // The file opens with SOI (FFD8) as a file-header region at offset 0.
    let soi = img.region_at(0).unwrap();
    assert_eq!(soi.kind, RegionKind::FileHeader);
    assert_eq!(soi.name, "SOI");

    // The final two bytes are EOI (FFD9).
    let eoi = img.region_at(jpg.len() - 1).unwrap();
    assert_eq!(eoi.name, "EOI");
    assert_eq!(&jpg[jpg.len() - 2..], &[0xFF, 0xD9]);
}

#[test]
fn sof_dimensions_are_decoded() {
    let jpg = fixtures::jpeg_baseline(320, 240);
    let img = parse_auto(&jpg).unwrap();

    let width = img.fields.iter().find(|f| f.name == "width").unwrap();
    assert_eq!(width.value, "320");
    let height = img.fields.iter().find(|f| f.name == "height").unwrap();
    assert_eq!(height.value, "240");

    // And the summary reports them.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Dimensions" && v == "320 × 240 px"));
    // Baseline DCT is reflected in the format line.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Format" && v.contains("baseline")));
}

#[test]
fn jfif_and_comment_are_read() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "JFIF version" && v == "1.01"));
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Comment" && v == "imghex demo"));

    // The APP0 segment is categorized as metadata.
    let app0 = img.regions.iter().find(|r| r.name == "APP0").unwrap();
    assert_eq!(app0.kind, RegionKind::Metadata);
}

#[test]
fn scan_data_becomes_a_pixel_region() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    // The entropy-coded scan is the one PixelData region, and it must not have
    // stopped early on the stuffed FF00 or the FFD0 restart marker.
    let scan = img
        .regions
        .iter()
        .find(|r| r.kind == RegionKind::PixelData)
        .expect("scan data region");
    assert_eq!(scan.name, "Entropy-coded scan data");
    let scan_bytes = &jpg[scan.start..scan.end];
    assert!(scan_bytes.windows(2).any(|w| w == [0xFF, 0x00]));
    assert!(scan_bytes.windows(2).any(|w| w == [0xFF, 0xD0]));
}

#[test]
fn compressed_pixels_are_not_byte_addressable() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    // No pixel_info, no palette: the preview / bit-plane tools stay dark.
    assert!(img.pixel_info.is_none());
    assert!(img.palette.is_empty());
    assert!(img.render(&jpg).is_none());
}

#[test]
fn regions_are_sorted_and_non_overlapping() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();
    for pair in img.regions.windows(2) {
        assert!(pair[0].start <= pair[1].start, "regions must be sorted");
        assert!(pair[0].end <= pair[1].start, "regions must not overlap");
    }
}

#[test]
fn dqt_header_and_coefficients_are_decoded() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    // The baseline fixture's DQT holds one 8-bit table (id 0) of 64 values, all
    // 0x10 = 16. The precision/id byte becomes its own field...
    let header = img
        .fields
        .iter()
        .find(|f| f.name == "quant_table")
        .expect("quant table header field");
    assert_eq!(header.value, "table 0, 8-bit");
    assert_eq!(header.len(), 1);

    // ...and each of the 64 coefficients is a separate field.
    let coeffs: Vec<_> = img
        .fields
        .iter()
        .filter(|f| f.name.starts_with("q["))
        .collect();
    assert_eq!(coeffs.len(), 64);

    // The first coefficient (zig-zag position 0) maps to natural index (0,0) and
    // sits immediately after the header byte, with the fixture's value of 16.
    let dc = img.fields.iter().find(|f| f.name == "q[0][0]").unwrap();
    assert_eq!(dc.value, "16");
    assert_eq!(dc.start, header.end);
    assert_eq!(dc.len(), 1);

    // Zig-zag position 2 maps to natural index 8, i.e. row 1, col 0.
    assert!(img.fields.iter().any(|f| f.name == "q[1][0]"));

    // Summary reports the table count.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Quantization tables" && v == "1"));
}

#[test]
fn dqt_decodes_multiple_tables_and_16bit_precision() {
    let jpg = fixtures::jpeg_dual_dqt();
    let img = parse_auto(&jpg).unwrap();

    // Two table headers, 128 coefficient fields (64 per table).
    let headers: Vec<_> = img
        .fields
        .iter()
        .filter(|f| f.name == "quant_table")
        .collect();
    assert_eq!(headers.len(), 2);
    let coeffs: Vec<_> = img
        .fields
        .iter()
        .filter(|f| f.name.starts_with("q["))
        .collect();
    assert_eq!(coeffs.len(), 128);

    // Payload begins at offset 6 (SOI=2, FF DB + 2-byte length = 4). Table 0's
    // 8-bit coefficients occupy offsets 7..71, so table 1's header is at 71.
    let t0 = img
        .fields
        .iter()
        .find(|f| f.start == 6 && f.name == "quant_table")
        .unwrap();
    assert_eq!(t0.value, "table 0, 8-bit");
    let t1 = img
        .fields
        .iter()
        .find(|f| f.start == 71 && f.name == "quant_table")
        .unwrap();
    assert_eq!(t1.value, "table 1, 16-bit");

    // Table 1's first (16-bit) coefficient sits at offset 72, spans two bytes,
    // and decodes the big-endian value 1000.
    let t1_dc = img.fields.iter().find(|f| f.start == 72).unwrap();
    assert_eq!(t1_dc.name, "q[0][0]");
    assert_eq!(t1_dc.value, "1000");
    assert_eq!(t1_dc.len(), 2);

    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Quantization tables" && v == "2"));
}

#[test]
fn truncated_after_soi_does_not_panic() {
    // Just the SOI marker, nothing else.
    let img = parse_auto(&[0xFF, 0xD8]).unwrap();
    assert_eq!(img.format_name, "JPEG");
    assert!(img.pixel_info.is_none());
}
