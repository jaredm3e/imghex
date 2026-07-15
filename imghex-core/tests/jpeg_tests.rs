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
fn dht_degenerate_table_is_decoded() {
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    // The baseline fixture's DHT is a single DC table (id 0) with all-zero
    // counts and therefore no symbols. The class/id byte is its own field...
    let header = img
        .fields
        .iter()
        .find(|f| f.name == "huff_table")
        .expect("huffman table header field");
    assert_eq!(header.value, "DC table 0");
    assert_eq!(header.len(), 1);

    // ...the 16 counts collapse to a single field reporting the symbol total...
    let counts = img
        .fields
        .iter()
        .find(|f| f.name == "code_counts")
        .expect("code counts field");
    assert_eq!(counts.value, "0 symbols");
    assert_eq!(counts.len(), 16);
    assert_eq!(counts.start, header.end);

    // ...and with no symbols there are no symbol fields.
    assert!(!img.fields.iter().any(|f| f.name.starts_with("symbol[")));

    // Summary lists the one DC table.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Huffman tables" && v == "DC: 0"));
}

#[test]
fn dht_decodes_multiple_tables_and_symbols() {
    let jpg = fixtures::jpeg_dual_dht();
    let img = parse_auto(&jpg).unwrap();

    // Two table headers (a DC and an AC), each with 3 symbols → 6 symbol fields.
    let headers: Vec<_> = img
        .fields
        .iter()
        .filter(|f| f.name == "huff_table")
        .collect();
    assert_eq!(headers.len(), 2);
    let symbols: Vec<_> = img
        .fields
        .iter()
        .filter(|f| f.name.starts_with("symbol["))
        .collect();
    assert_eq!(symbols.len(), 6);

    // Payload begins at offset 6 (SOI=2, FF C4 + 2-byte length = 4). Table 0's
    // header sits there; its 16 counts occupy 7..23; its 3 symbols 23..26; so
    // the AC table's header falls at offset 26.
    let dc = img
        .fields
        .iter()
        .find(|f| f.start == 6 && f.name == "huff_table")
        .unwrap();
    assert_eq!(dc.value, "DC table 0");
    let dc_counts = img.fields.iter().find(|f| f.start == 7).unwrap();
    assert_eq!(dc_counts.name, "code_counts");
    assert_eq!(dc_counts.value, "3 symbols");

    let ac = img
        .fields
        .iter()
        .find(|f| f.start == 26 && f.name == "huff_table")
        .unwrap();
    assert_eq!(ac.value, "AC table 0");
    let ac_counts = img.fields.iter().find(|f| f.start == 27).unwrap();
    assert_eq!(ac_counts.name, "code_counts");
    assert_eq!(ac_counts.value, "3 symbols");

    // The AC table's symbols start immediately after its counts, at offset 43.
    let ac_sym0 = img.fields.iter().find(|f| f.start == 43).unwrap();
    assert_eq!(ac_sym0.name, "symbol[0]");
    assert_eq!(ac_sym0.value, "0x11");
    assert_eq!(ac_sym0.len(), 1);

    // Summary lists both classes.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Huffman tables" && v == "DC: 0 · AC: 0"));
}

#[test]
fn exif_tiff_header_and_ifd0_tags_are_decoded() {
    let jpg = fixtures::jpeg_exif_le();
    let img = parse_auto(&jpg).unwrap();

    // The TIFF header begins at file offset 12 (see `jpeg_exif_le`).
    let base = 12;

    // Byte-order, magic, IFD0 offset each become fields.
    let bo = img
        .fields
        .iter()
        .find(|f| f.name == "tiff_byte_order")
        .expect("byte-order field");
    assert_eq!(bo.start, base);
    assert!(bo.value.contains("little-endian"));
    let magic = img.fields.iter().find(|f| f.name == "tiff_magic").unwrap();
    assert_eq!(magic.value, "42");
    let ifd0 = img.fields.iter().find(|f| f.name == "ifd0_offset").unwrap();
    assert_eq!(ifd0.value, "8");

    // Make: an ASCII tag whose data is out-of-line. The 12-byte entry sits at
    // TIFF offset 10 (base + 10), decoding to "imghex".
    let make = img
        .fields
        .iter()
        .find(|f| f.name == "Make" && f.len() == 12)
        .expect("Make entry field");
    assert_eq!(make.value, "imghex");
    assert_eq!(make.start, base + 10);

    // The out-of-line string bytes (TIFF offset 62) also decode when selected.
    let make_data = img
        .fields
        .iter()
        .find(|f| f.name == "Make" && f.start == base + 62)
        .expect("Make value-data field");
    assert_eq!(make_data.value, "imghex");
    assert_eq!(make_data.len(), 7);

    // Orientation: an inline SHORT.
    let orient = img.fields.iter().find(|f| f.name == "Orientation").unwrap();
    assert_eq!(orient.value, "1");

    // DateTime: out-of-line ASCII.
    let dt = img
        .fields
        .iter()
        .find(|f| f.name == "DateTime" && f.len() == 12)
        .unwrap();
    assert_eq!(dt.value, "2026:07:14 12:00:00");
}

#[test]
fn exif_ifd_pointer_is_followed() {
    let jpg = fixtures::jpeg_exif_le();
    let img = parse_auto(&jpg).unwrap();

    // The ExifIFD (tag 0x8769) pointer is followed; its tags appear as fields.
    let exposure = img
        .fields
        .iter()
        .find(|f| f.name == "ExposureTime")
        .expect("ExposureTime from the ExifIFD");
    assert_eq!(exposure.value, "1/100");

    let iso = img
        .fields
        .iter()
        .find(|f| f.name == "ISOSpeedRatings")
        .expect("ISO from the ExifIFD");
    assert_eq!(iso.value, "400");
}

#[test]
fn exif_headline_tags_appear_in_summary() {
    let jpg = fixtures::jpeg_exif_le();
    let img = parse_auto(&jpg).unwrap();

    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "EXIF metadata" && v == "present"));
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Camera make" && v == "imghex"));
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Orientation" && v == "1 (normal)"));
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "DateTime" && v == "2026:07:14 12:00:00"));
}

#[test]
fn exif_selecting_ifd_bytes_decodes_them() {
    // The display requirement: selecting an offset inside the Make entry returns
    // the decoded field via `field_at`.
    let jpg = fixtures::jpeg_exif_le();
    let img = parse_auto(&jpg).unwrap();
    let base = 12;
    let f = img.field_at(base + 10).expect("a field at the Make entry");
    assert_eq!(f.name, "Make");
    assert_eq!(f.value, "imghex");
}

#[test]
fn exif_big_endian_is_decoded() {
    let jpg = fixtures::jpeg_exif_be();
    let img = parse_auto(&jpg).unwrap();

    let bo = img
        .fields
        .iter()
        .find(|f| f.name == "tiff_byte_order")
        .unwrap();
    assert!(bo.value.contains("big-endian"));
    let make = img
        .fields
        .iter()
        .find(|f| f.name == "Make" && f.len() == 12)
        .unwrap();
    assert_eq!(make.value, "imghex");
    // Orientation 6 → rotated 90° CW in the summary.
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Orientation" && v == "6 (rotated 90° CW)"));
}

#[test]
fn exif_bogus_ifd_offset_does_not_panic() {
    // A TIFF header pointing IFD0 far past the payload must decode the header
    // only, emit no tag fields, and neither panic nor loop.
    let jpg = fixtures::jpeg_exif_bad_offset();
    let img = parse_auto(&jpg).unwrap();

    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "EXIF metadata" && v == "present"));
    // Header fields exist, but no Make/Model/Orientation were decodable.
    assert!(img.fields.iter().any(|f| f.name == "tiff_byte_order"));
    assert!(!img.fields.iter().any(|f| f.name == "Make"));
    assert!(!img
        .summary
        .iter()
        .any(|(k, _)| k == "Camera make" || k == "Orientation"));
}

#[test]
fn truncated_after_soi_does_not_panic() {
    // Just the SOI marker, nothing else.
    let img = parse_auto(&[0xFF, 0xD8]).unwrap();
    assert_eq!(img.format_name, "JPEG");
    assert!(img.pixel_info.is_none());
}

#[test]
fn sof_component_fields_baseline() {
    // The baseline fixture's SOF0 has one component: id 1, 1×1 sampling, quant
    // table 0. Each per-component parameter becomes its own field.
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    let id = img
        .fields
        .iter()
        .find(|f| f.name == "component[0].id")
        .expect("component id field");
    assert_eq!(id.value, "1");
    assert_eq!(id.len(), 1);

    // The specs sit immediately after the component-count byte.
    let count = img.fields.iter().find(|f| f.name == "components").unwrap();
    assert_eq!(id.start, count.end);

    let sampling = img
        .fields
        .iter()
        .find(|f| f.name == "component[0].sampling")
        .unwrap();
    assert_eq!(sampling.value, "1×1");
    let quant = img
        .fields
        .iter()
        .find(|f| f.name == "component[0].quant_table")
        .unwrap();
    assert_eq!(quant.value, "0");

    // Only the one declared component, so no second spec.
    assert!(!img.fields.iter().any(|f| f.name == "component[1].id"));
}

#[test]
fn sos_header_fields_baseline() {
    // The baseline fixture's SOS has one scan component (selector 1, DC/AC
    // table 0) and baseline spectral selection (Ss=0, Se=63, Ah=Al=0).
    let jpg = fixtures::jpeg_baseline(16, 16);
    let img = parse_auto(&jpg).unwrap();

    let count = img
        .fields
        .iter()
        .find(|f| f.name == "scan_components")
        .expect("scan component count field");
    assert_eq!(count.value, "1");

    let selector = img
        .fields
        .iter()
        .find(|f| f.name == "scan[0].selector")
        .unwrap();
    assert_eq!(selector.value, "1");
    // The scan specs sit immediately after the scan-component-count byte.
    assert_eq!(selector.start, count.end);

    let huff = img
        .fields
        .iter()
        .find(|f| f.name == "scan[0].huff_tables")
        .unwrap();
    assert_eq!(huff.value, "DC 0, AC 0");

    let ss = img
        .fields
        .iter()
        .find(|f| f.name == "spectral_start")
        .unwrap();
    assert_eq!(ss.value, "0");
    let se = img
        .fields
        .iter()
        .find(|f| f.name == "spectral_end")
        .unwrap();
    assert_eq!(se.value, "63");
    let approx = img
        .fields
        .iter()
        .find(|f| f.name == "successive_approx")
        .unwrap();
    assert_eq!(approx.value, "Ah 0, Al 0");
}

#[test]
fn sof_component_fields_three_components() {
    // The YCbCr fixture's SOF2 has three components with distinct sampling
    // factors and quant-table selectors. The payload begins at offset 6, so the
    // three-byte specs occupy 12..21.
    let jpg = fixtures::jpeg_ycbcr();
    let img = parse_auto(&jpg).unwrap();

    let by_offset = |off: usize| img.fields.iter().find(|f| f.start == off).unwrap();

    // Y component (id 1, 2×2, quant 0) at offsets 12..15.
    assert_eq!(by_offset(12).name, "component[0].id");
    assert_eq!(by_offset(12).value, "1");
    assert_eq!(by_offset(13).name, "component[0].sampling");
    assert_eq!(by_offset(13).value, "2×2");
    assert_eq!(by_offset(14).name, "component[0].quant_table");
    assert_eq!(by_offset(14).value, "0");

    // Cb component (id 2, 1×1, quant 1) at offsets 15..18.
    assert_eq!(by_offset(15).value, "2"); // component[1].id
    assert_eq!(by_offset(16).value, "1×1"); // component[1].sampling
    assert_eq!(by_offset(17).value, "1"); // component[1].quant_table

    // Cr component (id 3, 2×1, quant 1) at offsets 18..21.
    assert_eq!(by_offset(18).name, "component[2].id");
    assert_eq!(by_offset(18).value, "3");
    assert_eq!(by_offset(19).value, "2×1"); // component[2].sampling
    assert_eq!(by_offset(20).value, "1"); // component[2].quant_table

    // Three components decoded; the summary reflects the color count.
    assert_eq!(
        img.fields
            .iter()
            .filter(|f| f.name.ends_with(".id") && f.name.starts_with("component["))
            .count(),
        3
    );
    assert!(img
        .summary
        .iter()
        .any(|(k, v)| k == "Components" && v.contains("YCbCr")));
}

#[test]
fn sos_header_fields_three_components() {
    // The YCbCr fixture's SOS codes three components with distinct DC/AC table
    // selectors and a progressive spectral selection. The payload begins at
    // offset 25.
    let jpg = fixtures::jpeg_ycbcr();
    let img = parse_auto(&jpg).unwrap();

    let by_offset = |off: usize| img.fields.iter().find(|f| f.start == off).unwrap();

    assert_eq!(by_offset(25).name, "scan_components");
    assert_eq!(by_offset(25).value, "3");

    // Component 1: selector 1, DC 0 / AC 0.
    assert_eq!(by_offset(26).name, "scan[0].selector");
    assert_eq!(by_offset(26).value, "1");
    assert_eq!(by_offset(27).name, "scan[0].huff_tables");
    assert_eq!(by_offset(27).value, "DC 0, AC 0");

    // Component 2: selector 2, DC 1 / AC 1.
    assert_eq!(by_offset(28).value, "2");
    assert_eq!(by_offset(29).value, "DC 1, AC 1");

    // Component 3: selector 3, DC 1 / AC 2.
    assert_eq!(by_offset(30).value, "3");
    assert_eq!(by_offset(31).value, "DC 1, AC 2");

    // Progressive spectral selection: Ss=1, Se=63, Ah=1, Al=2.
    assert_eq!(by_offset(32).name, "spectral_start");
    assert_eq!(by_offset(32).value, "1");
    assert_eq!(by_offset(33).name, "spectral_end");
    assert_eq!(by_offset(33).value, "63");
    assert_eq!(by_offset(34).name, "successive_approx");
    assert_eq!(by_offset(34).value, "Ah 1, Al 2");
}

#[test]
fn overrunning_sof_component_count_does_not_panic() {
    // An SOF0 header that *claims* 255 components but carries only one 3-byte
    // spec. The parser must decode the one present spec and stop, never reading
    // past the payload.
    let mut jpg = vec![0xFF, 0xD8]; // SOI
                                    // FF C0, length 0x000B (11 = 2 length bytes + 9 payload bytes), then the
                                    // 6-byte frame header with a component count of 0xFF, then one 3-byte spec.
    jpg.extend_from_slice(&[
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0xFF, 0x01, 0x11, 0x00,
    ]);
    jpg.extend_from_slice(&[0xFF, 0xD9]); // EOI

    let img = parse_auto(&jpg).unwrap();
    assert_eq!(img.format_name, "JPEG");
    // Exactly one component spec was decoded despite the bogus count of 255.
    assert!(img.fields.iter().any(|f| f.name == "component[0].id"));
    assert!(!img.fields.iter().any(|f| f.name == "component[1].id"));
}
