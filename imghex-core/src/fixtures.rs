//! Builders for well-formed BMP byte streams.
//!
//! These are used by the crate's tests and can also generate a demo file for
//! the GUI. Everything here emits a standard 40-byte `BITMAPINFOHEADER` BMP.

use crate::color::Rgba;

fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn push_i32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn row_stride(bit_count: u16, width: u32) -> usize {
    (bit_count as usize * width as usize).div_ceil(32) * 4
}

/// Assemble a BMP from an already-laid-out palette and bottom-up pixel rows.
fn assemble(
    width: u32,
    height: u32,
    bit_count: u16,
    palette: &[Rgba],
    pixel_bytes: &[u8],
) -> Vec<u8> {
    let palette_bytes = palette.len() * 4;
    let off_bits = 14 + 40 + palette_bytes;
    let file_size = off_bits + pixel_bytes.len();

    let mut out = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER
    out.extend_from_slice(b"BM");
    push_u32(&mut out, file_size as u32);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u32(&mut out, off_bits as u32);
    // BITMAPINFOHEADER
    push_u32(&mut out, 40);
    push_i32(&mut out, width as i32);
    push_i32(&mut out, height as i32);
    push_u16(&mut out, 1);
    push_u16(&mut out, bit_count);
    push_u32(&mut out, 0); // BI_RGB
    push_u32(&mut out, pixel_bytes.len() as u32);
    push_i32(&mut out, 2835); // 72 DPI
    push_i32(&mut out, 2835);
    push_u32(&mut out, palette.len() as u32);
    push_u32(&mut out, 0);
    // Color table (RGBQUAD: B, G, R, reserved)
    for c in palette {
        out.push(c.b);
        out.push(c.g);
        out.push(c.r);
        out.push(0);
    }
    // Pixel array
    out.extend_from_slice(pixel_bytes);
    out
}

/// Build an 8-bpp indexed BMP.
///
/// `indices` is row-major, top-to-bottom, `width * height` entries. The output
/// stores rows bottom-up (standard BMP) with 4-byte row padding.
pub fn indexed_8bpp(width: u32, height: u32, palette: &[Rgba], indices: &[u8]) -> Vec<u8> {
    assert_eq!(
        indices.len(),
        (width * height) as usize,
        "index count mismatch"
    );
    let stride = row_stride(8, width);
    let mut pixels = vec![0u8; stride * height as usize];
    for y in 0..height {
        let src_row = &indices[(y * width) as usize..((y + 1) * width) as usize];
        // Bottom-up: source row y lands at destination row (height-1-y).
        let dst = (height - 1 - y) as usize * stride;
        pixels[dst..dst + width as usize].copy_from_slice(src_row);
    }
    assemble(width, height, 8, palette, &pixels)
}

/// Build a 1-bpp indexed BMP. `indices` is row-major, top-to-bottom, each 0/1.
pub fn indexed_1bpp(width: u32, height: u32, palette: &[Rgba], indices: &[u8]) -> Vec<u8> {
    assert_eq!(
        indices.len(),
        (width * height) as usize,
        "index count mismatch"
    );
    let stride = row_stride(1, width);
    let mut pixels = vec![0u8; stride * height as usize];
    for y in 0..height {
        let src = &indices[(y * width) as usize..((y + 1) * width) as usize];
        let dst_row = (height - 1 - y) as usize * stride;
        for (x, &v) in src.iter().enumerate() {
            if v != 0 {
                // Pixels are packed most-significant-bit first.
                pixels[dst_row + x / 8] |= 1 << (7 - (x % 8));
            }
        }
    }
    assemble(width, height, 1, palette, &pixels)
}

/// Build a 4-bpp indexed BMP. `indices` is row-major, top-to-bottom, each 0..=15.
pub fn indexed_4bpp(width: u32, height: u32, palette: &[Rgba], indices: &[u8]) -> Vec<u8> {
    assert_eq!(
        indices.len(),
        (width * height) as usize,
        "index count mismatch"
    );
    let stride = row_stride(4, width);
    let mut pixels = vec![0u8; stride * height as usize];
    for y in 0..height {
        let src = &indices[(y * width) as usize..((y + 1) * width) as usize];
        let dst_row = (height - 1 - y) as usize * stride;
        for (x, &v) in src.iter().enumerate() {
            let nibble = v & 0x0F;
            // The high nibble is the left pixel of each byte.
            if x.is_multiple_of(2) {
                pixels[dst_row + x / 2] |= nibble << 4;
            } else {
                pixels[dst_row + x / 2] |= nibble;
            }
        }
    }
    assemble(width, height, 4, palette, &pixels)
}

/// Build a 24-bpp BGR BMP from row-major, top-to-bottom RGB pixels.
pub fn bgr_24bpp(width: u32, height: u32, pixels_rgb: &[Rgba]) -> Vec<u8> {
    assert_eq!(
        pixels_rgb.len(),
        (width * height) as usize,
        "pixel count mismatch"
    );
    let stride = row_stride(24, width);
    let mut pixels = vec![0u8; stride * height as usize];
    for y in 0..height {
        let dst_row = (height - 1 - y) as usize * stride;
        for x in 0..width {
            let c = pixels_rgb[(y * width + x) as usize];
            let o = dst_row + x as usize * 3;
            pixels[o] = c.b;
            pixels[o + 1] = c.g;
            pixels[o + 2] = c.r;
        }
    }
    assemble(width, height, 24, &[], &pixels)
}

/// A small, colorful 8-bpp demo image (a 4×4 gradient over a 4-color palette).
pub fn demo_indexed() -> Vec<u8> {
    let palette = [
        Rgba::rgb(0xFF, 0x00, 0x00),
        Rgba::rgb(0x00, 0xFF, 0x00),
        Rgba::rgb(0x00, 0x00, 0xFF),
        Rgba::rgb(0xFF, 0xFF, 0x00),
    ];
    let indices: Vec<u8> = (0..16).map(|i| (i % 4) as u8).collect();
    indexed_8bpp(4, 4, &palette, &indices)
}

/// A 1-bpp (2-color) 16×8 checkerboard — one byte spans 8 pixels.
pub fn demo_1bpp() -> Vec<u8> {
    let palette = [Rgba::rgb(0x10, 0x10, 0x10), Rgba::rgb(0xF0, 0xF0, 0xF0)];
    let (w, h) = (16u32, 8u32);
    let mut indices = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            indices.push(((x + y) % 2) as u8);
        }
    }
    indexed_1bpp(w, h, &palette, &indices)
}

/// A 4-bpp (16-color) 16×16 image whose columns walk the whole palette.
pub fn demo_4bpp() -> Vec<u8> {
    let palette: Vec<Rgba> = (0..16u32)
        .map(|i| {
            let v = (i * 17) as u8;
            Rgba::rgb(v, 255 - v, ((i * 40) % 256) as u8)
        })
        .collect();
    let (w, h) = (16u32, 16u32);
    let mut indices = Vec::with_capacity((w * h) as usize);
    for _y in 0..h {
        for x in 0..w {
            indices.push((x % 16) as u8);
        }
    }
    indexed_4bpp(w, h, &palette, &indices)
}

/// Build a binary P6 (RGB) Netpbm image from row-major, top-to-bottom pixels.
pub fn netpbm_p6(width: u32, height: u32, pixels_rgb: &[Rgba]) -> Vec<u8> {
    assert_eq!(
        pixels_rgb.len(),
        (width * height) as usize,
        "pixel count mismatch"
    );
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    for c in pixels_rgb {
        out.push(c.r);
        out.push(c.g);
        out.push(c.b);
    }
    out
}

/// Build a binary P5 (grayscale) Netpbm image from row-major gray samples.
pub fn netpbm_p5(width: u32, height: u32, gray: &[u8]) -> Vec<u8> {
    assert_eq!(
        gray.len(),
        (width * height) as usize,
        "sample count mismatch"
    );
    let mut out = format!("P5\n{width} {height}\n255\n").into_bytes();
    out.extend_from_slice(gray);
    out
}

/// A 16×16 P6 demo (RGB gradient), for exercising the second format.
pub fn demo_ppm() -> Vec<u8> {
    let (w, h) = (16u32, 16u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push(Rgba::rgb((x * 16) as u8, (y * 16) as u8, 0x60));
        }
    }
    netpbm_p6(w, h, &pixels)
}

/// A 24-bpp true-color 16×16 RGB gradient.
pub fn demo_24bpp() -> Vec<u8> {
    let (w, h) = (16u32, 16u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push(Rgba::rgb((x * 16) as u8, (y * 16) as u8, 0x80));
        }
    }
    bgr_24bpp(w, h, &pixels)
}

/// Append a length-prefixed JPEG marker segment (`FF <marker> <len:2> payload`).
/// The big-endian length counts the two length bytes plus the payload.
fn push_jpeg_segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    out.push(0xFF);
    out.push(marker);
    let len = (payload.len() + 2) as u16;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
}

/// Build a structurally valid baseline JPEG (JFIF) byte stream.
///
/// The image is not meant to be *decoded* — imghex reads JPEG marker structure,
/// not its DCT-compressed pixels — so the quantization/Huffman tables and the
/// entropy-coded scan data hold placeholder bytes. What is faithful is the
/// framing: SOI, an APP0/JFIF segment, a comment, a DQT, a baseline SOF0 that
/// carries the real `width`/`height`, a DHT, an SOS, some scan data (including a
/// stuffed `FF00` and an `FFD0` restart marker, to exercise the scan walker),
/// and EOI.
pub fn jpeg_baseline(width: u16, height: u16) -> Vec<u8> {
    let mut out = Vec::new();
    // SOI
    out.extend_from_slice(&[0xFF, 0xD8]);
    // APP0 / JFIF: identifier, version 1.01, no density, no thumbnail.
    push_jpeg_segment(
        &mut out,
        0xE0,
        &[
            b'J', b'F', b'I', b'F', 0x00, // "JFIF\0"
            0x01, 0x01, // version 1.01
            0x00, // density units: none
            0x00, 0x01, 0x00, 0x01, // X/Y density 1×1
            0x00, 0x00, // thumbnail 0×0
        ],
    );
    // Comment
    push_jpeg_segment(&mut out, 0xFE, b"imghex demo");
    // DQT: table id 0, 8-bit precision, 64 placeholder quantization values.
    let mut dqt = vec![0x00];
    dqt.extend(std::iter::repeat_n(0x10, 64));
    push_jpeg_segment(&mut out, 0xDB, &dqt);
    // SOF0 (baseline DCT): precision 8, dimensions, one grayscale component.
    let mut sof = vec![0x08];
    sof.extend_from_slice(&height.to_be_bytes());
    sof.extend_from_slice(&width.to_be_bytes());
    sof.push(0x01); // component count
    sof.extend_from_slice(&[0x01, 0x11, 0x00]); // id, sampling factors, quant table
    push_jpeg_segment(&mut out, 0xC0, &sof);
    // DHT: class 0, id 0, an all-zero counts table (no symbols) — placeholder.
    let mut dht = vec![0x00];
    dht.extend(std::iter::repeat_n(0x00, 16));
    push_jpeg_segment(&mut out, 0xC4, &dht);
    // SOS: one component.
    push_jpeg_segment(&mut out, 0xDA, &[0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    // Entropy-coded data: an ordinary byte, a stuffed FF00, and an FFD0 restart.
    out.extend_from_slice(&[0xAA, 0xBB, 0xFF, 0x00, 0xCC, 0xFF, 0xD0, 0x12, 0x34]);
    // EOI
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A 16×16 baseline JPEG demo, exercising the marker-structure view.
pub fn demo_jpeg() -> Vec<u8> {
    jpeg_baseline(16, 16)
}

/// Build a minimal JPEG (SOI, one DQT, EOI) whose single DQT segment defines
/// two quantization tables: table 0 with 8-bit values `0..64`, and table 1 with
/// 16-bit values `1000..1064` — exercising multi-table and 16-bit DQT decoding.
pub fn jpeg_dual_dqt() -> Vec<u8> {
    let mut dqt = Vec::new();
    // Table 0: Pq = 0 (8-bit values), Tq = 0.
    dqt.push(0x00);
    for k in 0..64u16 {
        dqt.push(k as u8);
    }
    // Table 1: Pq = 1 (16-bit values), Tq = 1.
    dqt.push(0x11);
    for k in 0..64u16 {
        dqt.extend_from_slice(&(1000 + k).to_be_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    push_jpeg_segment(&mut out, 0xDB, &dqt);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

/// Wrap a TIFF payload in an APP1/EXIF JPEG: `SOI · APP1("Exif\0\0" + tiff) · EOI`.
///
/// The TIFF header therefore begins at file offset 12 (SOI = 2, `FF E1` + 2-byte
/// length = 4, `Exif\0\0` = 6), which is the `base` the parser decodes against.
fn wrap_exif_jpeg(tiff: &[u8]) -> Vec<u8> {
    let mut app1 = Vec::new();
    app1.extend_from_slice(b"Exif\0\0");
    app1.extend_from_slice(tiff);
    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    push_jpeg_segment(&mut out, 0xE1, &app1);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

/// Append a 12-byte little-endian TIFF/IFD entry.
fn push_ifd_entry_le(v: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: u32) {
    push_u16(v, tag);
    push_u16(v, typ);
    push_u32(v, count);
    push_u32(v, value);
}

/// Build a real little-endian (`II`) APP1/EXIF JPEG.
///
/// IFD0 carries Make="imghex" (ASCII, out-of-line), Orientation=1 (SHORT,
/// inline), DateTime (ASCII, out-of-line) and an ExifIFD pointer (tag 0x8769).
/// The ExifIFD carries ExposureTime=1/100 (RATIONAL) and ISOSpeedRatings=400
/// (SHORT). Offsets are relative to the TIFF header start; the fixed layout is
/// documented inline so tests can assert exact positions.
pub fn jpeg_exif_le() -> Vec<u8> {
    const MAKE: &[u8] = b"imghex\0"; // 7 bytes
    const DATETIME: &[u8] = b"2026:07:14 12:00:00\0"; // 20 bytes

    // Layout (relative to TIFF start):
    //   0  header (II, magic, IFD0 offset = 8)
    //   8  IFD0: count(2) + 4 entries(48) + next-IFD(4) → 10..62
    //   62 Make data (7)          → 62..69
    //   69 DateTime data (20)     → 69..89
    //   89 ExifIFD: count(2) + 2 entries(24) + next-IFD(4) → 89..119
    //   119 ExposureTime rational (8) → 119..127
    let make_off = 62u32;
    let datetime_off = 69u32;
    let exififd_off = 89u32;
    let exposure_off = 119u32;

    let mut t = Vec::new();
    // TIFF header.
    t.extend_from_slice(b"II");
    push_u16(&mut t, 42);
    push_u32(&mut t, 8); // IFD0 at offset 8
                         // IFD0.
    push_u16(&mut t, 4); // entry count
    push_ifd_entry_le(&mut t, 0x010F, 2, MAKE.len() as u32, make_off); // Make (ASCII)
    push_ifd_entry_le(&mut t, 0x0112, 3, 1, 1); // Orientation (SHORT, inline = 1)
    push_ifd_entry_le(&mut t, 0x0132, 2, DATETIME.len() as u32, datetime_off); // DateTime
    push_ifd_entry_le(&mut t, 0x8769, 4, 1, exififd_off); // ExifIFD pointer (LONG)
    push_u32(&mut t, 0); // next-IFD offset (none)
                         // Out-of-line IFD0 data.
    t.extend_from_slice(MAKE);
    t.extend_from_slice(DATETIME);
    // ExifIFD.
    push_u16(&mut t, 2); // entry count
    push_ifd_entry_le(&mut t, 0x829A, 5, 1, exposure_off); // ExposureTime (RATIONAL)
    push_ifd_entry_le(&mut t, 0x8827, 3, 1, 400); // ISOSpeedRatings (SHORT, inline)
    push_u32(&mut t, 0); // next-IFD offset (none)
                         // ExposureTime rational data: 1/100.
    push_u32(&mut t, 1);
    push_u32(&mut t, 100);

    debug_assert_eq!(t.len(), 127, "unexpected EXIF TIFF layout size");
    wrap_exif_jpeg(&t)
}

/// Build a real big-endian (`MM`) APP1/EXIF JPEG with Make="imghex" (ASCII,
/// out-of-line) and Orientation=6 (SHORT, inline) in IFD0 — the endianness
/// counterpart of [`jpeg_exif_le`].
pub fn jpeg_exif_be() -> Vec<u8> {
    // IFD0: count(2)@8 + 2 entries(24)@10..34 + next-IFD(4)@34..38 → data at 38.
    const MAKE: &[u8] = b"imghex\0"; // 7 bytes, out-of-line at offset 38
    let make_off = 38u32;

    let mut t = Vec::new();
    // TIFF header (big-endian).
    t.extend_from_slice(b"MM");
    t.extend_from_slice(&42u16.to_be_bytes());
    t.extend_from_slice(&8u32.to_be_bytes()); // IFD0 at offset 8
                                              // IFD0: count(2) + 2 entries(24) + next-IFD(4) → data at 34.
    t.extend_from_slice(&2u16.to_be_bytes()); // entry count
                                              // Make (ASCII), out-of-line.
    t.extend_from_slice(&0x010Fu16.to_be_bytes());
    t.extend_from_slice(&2u16.to_be_bytes());
    t.extend_from_slice(&(MAKE.len() as u32).to_be_bytes());
    t.extend_from_slice(&make_off.to_be_bytes());
    // Orientation (SHORT, inline = 6). The value occupies the first 2 value bytes.
    t.extend_from_slice(&0x0112u16.to_be_bytes());
    t.extend_from_slice(&3u16.to_be_bytes());
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&6u16.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes()); // pad the 4-byte value slot
    t.extend_from_slice(&0u32.to_be_bytes()); // next-IFD offset (none)
    t.extend_from_slice(MAKE);

    wrap_exif_jpeg(&t)
}

/// Build an APP1/EXIF JPEG whose TIFF header declares a bogus IFD0 offset far
/// past the payload — for asserting the parser neither panics nor loops on
/// untrusted offsets.
pub fn jpeg_exif_bad_offset() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"II");
    push_u16(&mut t, 42);
    push_u32(&mut t, 0xFFFF_FFFF); // IFD0 offset way out of bounds
    wrap_exif_jpeg(&t)
}

/// Build a minimal JPEG (SOI, one DHT, EOI) whose single DHT segment defines two
/// Huffman tables back to back: a DC table (id 0) with 3 symbols, and an AC
/// table (id 0) with 3 symbols — exercising multi-table decoding and the
/// symbol-count math (counts summing to the length of the symbol list).
pub fn jpeg_dual_dht() -> Vec<u8> {
    let mut dht = Vec::new();
    // DC table 0: Tc = 0 (DC), Th = 0. Counts say 1 code of length 2 and 2 codes
    // of length 3, so 3 symbols follow.
    dht.push(0x00);
    let dc_counts = [0u8, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    dht.extend_from_slice(&dc_counts);
    dht.extend_from_slice(&[0x01, 0x02, 0x03]);
    // AC table 0: Tc = 1 (AC), Th = 0. Counts say 2 codes of length 2 and 1 of
    // length 3, so 3 symbols follow.
    dht.push(0x10);
    let ac_counts = [0u8, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    dht.extend_from_slice(&ac_counts);
    dht.extend_from_slice(&[0x11, 0x12, 0x21]);
    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    push_jpeg_segment(&mut out, 0xC4, &dht);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

/// Build a minimal three-component (YCbCr) JPEG that exercises the per-component
/// SOF and SOS decoding: SOI, a progressive SOF2 whose three components carry
/// distinct sampling factors and quantization-table selectors, an SOS whose
/// three scan entries carry distinct DC/AC Huffman-table selectors and
/// non-baseline spectral-selection bytes, then EOI. There is no DQT/DHT or
/// entropy-coded scan data — the fixture exists to pin down the frame- and
/// scan-header component fields, not to be decoded.
///
/// Byte layout (used by the tests' offset assertions): SOI at 0, the SOF2
/// payload begins at offset 6 (SOI=2, `FF C2` + 2-byte length = 4); the SOS
/// payload begins at offset 25.
pub fn jpeg_ycbcr() -> Vec<u8> {
    let (width, height) = (32u16, 16u16);
    // SOF2 (progressive DCT): precision 8, dimensions, three components.
    let mut sof = vec![0x08];
    sof.extend_from_slice(&height.to_be_bytes());
    sof.extend_from_slice(&width.to_be_bytes());
    sof.push(0x03); // component count
    sof.extend_from_slice(&[0x01, 0x22, 0x00]); // Y:  id 1, 2×2 sampling, quant table 0
    sof.extend_from_slice(&[0x02, 0x11, 0x01]); // Cb: id 2, 1×1 sampling, quant table 1
    sof.extend_from_slice(&[0x03, 0x21, 0x01]); // Cr: id 3, 2×1 sampling, quant table 1

    // SOS: three scan components with distinct DC/AC table selectors, and a
    // non-baseline spectral selection (Ss=1, Se=63, Ah=1, Al=2).
    let sos = [
        0x03, // component count
        0x01, 0x00, // component 1: selector 1, DC table 0, AC table 0
        0x02, 0x11, // component 2: selector 2, DC table 1, AC table 1
        0x03, 0x12, // component 3: selector 3, DC table 1, AC table 2
        0x01, 0x3F, 0x12, // Ss=1, Se=63, Ah=1, Al=2
    ];

    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    push_jpeg_segment(&mut out, 0xC2, &sof);
    push_jpeg_segment(&mut out, 0xDA, &sos);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}
