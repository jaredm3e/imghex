//! Integration tests for BMP parsing and per-offset decoding.
//!
//! These treat `imghex-core` as an external consumer would, using the public
//! `fixtures` builders to generate known-good files and asserting the parsed
//! model matches the on-disk layout byte-for-byte.

use imghex_core::model::{PixelEncoding, PixelInfo};
use imghex_core::region::RegionKind;
use imghex_core::{fixtures, parse_auto, Rgba};

// A 4×4, 4-color indexed image. Layout (all offsets in bytes):
//   0..14   BITMAPFILEHEADER
//   14..54  BITMAPINFOHEADER
//   54..70  color table (4 × RGBQUAD)
//   70..86  pixel data (stride 4 × 4 rows)
const DEMO_OFF_BITS: usize = 70;
const DEMO_FILE_SIZE: usize = 86;

fn demo_palette() -> [Rgba; 4] {
    [
        Rgba::rgb(0xFF, 0x00, 0x00),
        Rgba::rgb(0x00, 0xFF, 0x00),
        Rgba::rgb(0x00, 0x00, 0xFF),
        Rgba::rgb(0xFF, 0xFF, 0x00),
    ]
}

#[test]
fn parses_demo_and_reports_basics() {
    let bmp = fixtures::demo_indexed();
    assert_eq!(bmp.len(), DEMO_FILE_SIZE);

    let img = parse_auto(&bmp).expect("should parse");
    assert_eq!(img.format_name, "BMP");
    assert_eq!(img.palette.len(), 4);
    assert_eq!(img.palette[0], Rgba::rgb(0xFF, 0x00, 0x00));
}

#[test]
fn regions_cover_expected_sections() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();

    assert_eq!(img.region_at(0).unwrap().kind, RegionKind::FileHeader);
    assert_eq!(img.region_at(13).unwrap().kind, RegionKind::FileHeader);
    assert_eq!(img.region_at(14).unwrap().kind, RegionKind::InfoHeader);
    assert_eq!(img.region_at(53).unwrap().kind, RegionKind::InfoHeader);
    assert_eq!(img.region_at(54).unwrap().kind, RegionKind::Palette);
    assert_eq!(img.region_at(69).unwrap().kind, RegionKind::Palette);
    assert_eq!(img.region_at(70).unwrap().kind, RegionKind::PixelData);
    assert_eq!(img.region_at(85).unwrap().kind, RegionKind::PixelData);

    // Regions must be sorted and non-overlapping.
    for pair in img.regions.windows(2) {
        assert!(pair[0].end <= pair[1].start, "regions overlap: {:?}", pair);
    }
}

#[test]
fn file_header_fields_decode() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();

    let sig = img.describe(0, &bmp).unwrap();
    assert_eq!(sig.field.as_ref().unwrap().name, "bfType");
    assert_eq!(sig.field.as_ref().unwrap().value, "\"BM\"");

    let off = img.describe(10, &bmp).unwrap();
    assert_eq!(off.field.as_ref().unwrap().name, "bfOffBits");
    assert_eq!(off.field.as_ref().unwrap().value, DEMO_OFF_BITS.to_string());

    let size = img.describe(2, &bmp).unwrap();
    assert!(size
        .field
        .as_ref()
        .unwrap()
        .value
        .contains(&DEMO_FILE_SIZE.to_string()));
}

#[test]
fn info_header_reports_dimensions_and_depth() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();

    let width = img.describe(18, &bmp).unwrap();
    assert_eq!(width.field.as_ref().unwrap().name, "biWidth");
    assert_eq!(width.field.as_ref().unwrap().value, "4");

    let depth = img.describe(28, &bmp).unwrap();
    assert_eq!(depth.field.as_ref().unwrap().name, "biBitCount");
    assert_eq!(depth.field.as_ref().unwrap().value, "8");
}

#[test]
fn palette_byte_resolves_to_entry_and_color() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();
    let pal = demo_palette();

    // First palette byte (offset 54) is the Blue channel of entry 0 (red).
    let s = img.describe(54, &bmp).unwrap();
    assert_eq!(s.region_kind, Some(RegionKind::Palette));
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Palette index" && v == "0"));
    assert!(s.details.iter().any(|(k, v)| k == "Channel" && v == "Blue"));
    assert!(s.swatches.iter().any(|w| w.color == pal[0]));

    // Entry 2 (blue) begins at 54 + 2*4 = 62; its Red channel is at 62+2 = 64.
    let s = img.describe(64, &bmp).unwrap();
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Palette index" && v == "2"));
    assert!(s.details.iter().any(|(k, v)| k == "Channel" && v == "Red"));
    assert!(s.swatches.iter().any(|w| w.color == pal[2]));
}

#[test]
fn indexed_pixel_byte_resolves_to_coordinate_and_palette_color() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();
    let pal = demo_palette();

    // First pixel byte (offset 70). Bottom-up storage: file row 0 is image
    // row y = height-1 = 3. Column 0. Its index value drives the color.
    let s = img.describe(DEMO_OFF_BITS, &bmp).unwrap();
    assert_eq!(s.region_kind, Some(RegionKind::PixelData));
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Pixel (x, y)" && v == "(0, 3)"));

    let index = bmp[DEMO_OFF_BITS] as usize;
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Palette index" && *v == index.to_string()));
    assert!(s.swatches.iter().any(|w| w.color == pal[index]));
}

#[test]
fn direct_color_24bpp_decodes_channels() {
    // 2×2 image, top-to-bottom source order.
    let px = [
        Rgba::rgb(0xFF, 0x00, 0x00), // (0,0) red
        Rgba::rgb(0x00, 0xFF, 0x00), // (1,0) green
        Rgba::rgb(0x00, 0x00, 0xFF), // (0,1) blue
        Rgba::rgb(0xFF, 0xFF, 0xFF), // (1,1) white
    ];
    let bmp = fixtures::bgr_24bpp(2, 2, &px);
    let img = parse_auto(&bmp).unwrap();
    assert!(img.palette.is_empty());

    // Data starts at 54 (no palette). File row 0 is image row y=1 (bottom-up),
    // so the first pixel is (0,1) = blue. Offset 54 is its Blue channel byte.
    let data_start = 54;
    let s = img.describe(data_start, &bmp).unwrap();
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Pixel (x, y)" && v == "(0, 1)"));
    assert!(s.details.iter().any(|(k, v)| k == "Channel" && v == "Blue"));
    assert!(s
        .swatches
        .iter()
        .any(|w| w.color == Rgba::rgb(0x00, 0x00, 0xFF)));
}

#[test]
fn truncated_file_is_reported() {
    let bmp = fixtures::demo_indexed();
    let truncated = &bmp[..10];
    assert!(parse_auto(truncated).is_err());
}

#[test]
fn out_of_bounds_describe_returns_none() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();
    assert!(img.describe(bmp.len(), &bmp).is_none());
    assert!(img.describe(bmp.len() + 100, &bmp).is_none());
}

#[test]
fn pixel_info_top_down_orientation() {
    // Directly exercise top-down row ordering: row 0 == image row 0.
    let pi = PixelInfo {
        data_start: 0,
        width: 2,
        height: 2,
        top_down: true,
        row_stride: 8,
        encoding: PixelEncoding::BgrDirect { bytes: 3 },
    };
    let raw = vec![0u8; 16];
    let loc = pi.locate(0, &raw, &[]).unwrap();
    assert_eq!((loc.x, loc.y), (0, 0));
    // Second row starts at stride (8) → y = 1 when top-down.
    let loc = pi.locate(8, &raw, &[]).unwrap();
    assert_eq!((loc.x, loc.y), (0, 1));
}

#[test]
fn row_padding_is_flagged() {
    // 1×1 8-bpp image: stride padded from 1 to 4, so offsets 1..4 are padding.
    let bmp = fixtures::indexed_8bpp(1, 1, &[Rgba::rgb(1, 2, 3)], &[0]);
    let img = parse_auto(&bmp).unwrap();
    let data_start = img.pixel_info.as_ref().unwrap().data_start;
    let s = img.describe(data_start + 1, &bmp).unwrap();
    assert!(
        s.details.iter().any(|(_, v)| v.contains("padding")),
        "expected padding note, got {:?}",
        s.details
    );
}

#[test]
fn one_bpp_byte_exposes_all_eight_pixels() {
    // 8×1, alternating indices 1,0,1,0,… — one byte holds the whole row.
    let palette = [Rgba::rgb(0, 0, 0), Rgba::rgb(255, 255, 255)];
    let indices = [1u8, 0, 1, 0, 1, 0, 1, 0];
    let bmp = fixtures::indexed_1bpp(8, 1, &palette, &indices);
    let img = parse_auto(&bmp).unwrap();

    let ds = img.pixel_info.as_ref().unwrap().data_start;
    let samples = img
        .pixel_info
        .as_ref()
        .unwrap()
        .samples(ds, &bmp, &img.palette);
    assert_eq!(samples.len(), 8, "one 1-bpp byte encodes 8 pixels");
    assert_eq!(samples[0].palette_index, Some(1));
    assert_eq!(samples[1].palette_index, Some(0));
    assert_eq!(samples[0].color, Some(palette[1]));

    // The sidebar description exposes all eight colors as swatches.
    let s = img.describe(ds, &bmp).unwrap();
    assert_eq!(s.swatches.len(), 8);
}

#[test]
fn four_bpp_byte_exposes_two_pixels() {
    // 2×1, indices 3 then 5, packed high-then-low nibble into one byte.
    let mut palette = vec![Rgba::rgb(0, 0, 0); 16];
    palette[3] = Rgba::rgb(10, 20, 30);
    palette[5] = Rgba::rgb(40, 50, 60);
    let bmp = fixtures::indexed_4bpp(2, 1, &palette, &[3, 5]);
    let img = parse_auto(&bmp).unwrap();

    let ds = img.pixel_info.as_ref().unwrap().data_start;
    let samples = img
        .pixel_info
        .as_ref()
        .unwrap()
        .samples(ds, &bmp, &img.palette);
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0].palette_index, Some(3));
    assert_eq!(samples[1].palette_index, Some(5));
    assert_eq!(samples[0].color, Some(palette[3]));
    assert_eq!(samples[1].color, Some(palette[5]));
}

#[test]
fn renders_indexed_image_top_down() {
    // 2×2 indexed image; check render orients rows top-down and resolves colors.
    let palette = [
        Rgba::rgb(10, 0, 0),
        Rgba::rgb(0, 20, 0),
        Rgba::rgb(0, 0, 30),
        Rgba::rgb(40, 40, 40),
    ];
    // Source (top-to-bottom): (0,0)=0 (1,0)=1 / (0,1)=2 (1,1)=3
    let bmp = fixtures::indexed_8bpp(2, 2, &palette, &[0, 1, 2, 3]);
    let img = parse_auto(&bmp).unwrap();
    let r = img.render(&bmp).unwrap();
    assert_eq!((r.width, r.height), (2, 2));
    assert_eq!(r.pixel(0, 0), Some(palette[0]));
    assert_eq!(r.pixel(1, 0), Some(palette[1]));
    assert_eq!(r.pixel(0, 1), Some(palette[2]));
    assert_eq!(r.pixel(1, 1), Some(palette[3]));
}

#[test]
fn bit_plane_extracts_lsb() {
    use imghex_core::Channel;
    // Two gray-ish pixels via 24bpp: red channel 0x01 (LSB set) and 0x02 (clear).
    let px = [Rgba::rgb(0x01, 0, 0), Rgba::rgb(0x02, 0, 0)];
    let bmp = fixtures::bgr_24bpp(2, 1, &px);
    let img = parse_auto(&bmp).unwrap();
    let r = img.render(&bmp).unwrap();
    let plane = r.bit_plane(Channel::Red, 0);
    assert_eq!(plane.len(), 2);
    assert!(plane[0], "0x01 has LSB set");
    assert!(!plane[1], "0x02 has LSB clear");
}

#[test]
fn byte_color_resolves_pixel_and_palette() {
    let bmp = fixtures::demo_indexed();
    let img = parse_auto(&bmp).unwrap();
    let pal = demo_palette();

    // A pixel byte resolves to its palette color.
    let idx = bmp[DEMO_OFF_BITS] as usize;
    assert_eq!(img.byte_color(DEMO_OFF_BITS, &bmp), Some(pal[idx]));

    // A palette byte resolves to that entry's color (offset 54 = entry 0).
    assert_eq!(img.byte_color(54, &bmp), Some(pal[0]));

    // A header byte has no pixel color.
    assert_eq!(img.byte_color(0, &bmp), None);
}
