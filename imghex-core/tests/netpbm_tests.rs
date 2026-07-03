//! Integration tests for the Netpbm (PPM/PGM) format — the crate's second
//! format, proving the `ImageFormat` abstraction generalizes.

use imghex_core::region::RegionKind;
use imghex_core::{fixtures, parse_auto, Rgba};

#[test]
fn detects_and_parses_p6() {
    let px = [
        Rgba::rgb(10, 20, 30),
        Rgba::rgb(40, 50, 60),
        Rgba::rgb(70, 80, 90),
        Rgba::rgb(100, 110, 120),
    ];
    let ppm = fixtures::netpbm_p6(2, 2, &px);
    let img = parse_auto(&ppm).unwrap();
    assert_eq!(img.format_name, "Netpbm");

    // Header "P6\n2 2\n255\n" is 11 bytes, then RGB triples begin.
    let data_start = img.pixel_info.as_ref().unwrap().data_start;
    assert_eq!(img.region_at(0).unwrap().kind, RegionKind::InfoHeader);
    assert_eq!(
        img.region_at(data_start).unwrap().kind,
        RegionKind::PixelData
    );

    // First pixel (top-left) decodes to the first RGB triple, in R,G,B order.
    let s = img.describe(data_start, &ppm).unwrap();
    assert!(s
        .details
        .iter()
        .any(|(k, v)| k == "Pixel (x, y)" && v == "(0, 0)"));
    assert!(s.details.iter().any(|(k, v)| k == "Channel" && v == "Red"));
    assert!(s.swatches.iter().any(|w| w.color == Rgba::rgb(10, 20, 30)));
}

#[test]
fn parses_p5_grayscale() {
    let gray = [0u8, 64, 128, 255];
    let pgm = fixtures::netpbm_p5(2, 2, &gray);
    let img = parse_auto(&pgm).unwrap();

    let r = img.render(&pgm).unwrap();
    assert_eq!((r.width, r.height), (2, 2));
    // Grayscale replicates the sample across R, G, B.
    assert_eq!(r.pixel(0, 0), Some(Rgba::rgb(0, 0, 0)));
    assert_eq!(r.pixel(1, 1), Some(Rgba::rgb(255, 255, 255)));
}

#[test]
fn header_fields_decode() {
    let ppm = fixtures::netpbm_p6(3, 4, &[Rgba::rgb(0, 0, 0); 12]);
    let img = parse_auto(&ppm).unwrap();

    let width = img.fields.iter().find(|f| f.name == "width").unwrap();
    assert_eq!(width.value, "3");
    let height = img.fields.iter().find(|f| f.name == "height").unwrap();
    assert_eq!(height.value, "4");
}
