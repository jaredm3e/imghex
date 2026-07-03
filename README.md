# imghex — an image-aware hex editor

A GUI hex editor built around image files. Instead of a flat sea of bytes, it
colors each byte by the file-format structure it belongs to (file header, info
header, palette, pixel data, …) and gives you a sidebar that **decodes the
selected byte in context** — for example, selecting a byte in the pixel data of
an indexed (paletted) image tells you the pixel coordinate, the palette index,
and resolves the actual color from the palette.

It understands **BMP** and **Netpbm (PPM/PGM)**, and the architecture is
format-agnostic: adding a new format is one trait implementation plus one line
in the registry.

## Features

- **Structure-colored hex grid** — every byte tinted by its region; a legend
  maps colors to sections. Toggle between **section colors** and **pixel colors**
  (bytes painted by their decoded color).
- **Context-aware inspector** — offset, raw byte (hex/dec/bin/ascii), the
  section it lives in, the decoded header field (name, value, description), and
  format-specific decoding.
- **Palette resolution** — pixel bytes in indexed images resolve to color
  swatches via the palette; a byte packing several pixels (1- and 4-bpp) shows
  **all** of its colors at once. The full palette is shown as a swatch grid.
- **Image preview + bit-plane viewer** — render the decoded image beside the
  hex; click a pixel to jump to its bytes. The bit-plane mode isolates any bit
  of R/G/B/luminance — bit 0 (LSB) exposes LSB steganography.
- **Multi-byte selection & statistics** — shift-click / drag / shift-arrows to
  select a range; the sidebar shows count, min/max/mean, distinct values,
  **Shannon entropy**, and a 256-bin byte histogram.
- **Whole-file entropy strip** — a heat bar over the file (blue→red); click to
  jump. High-entropy regions flag compressed/embedded data.
- **Find & go-to** — search for hex or text patterns; jump to any offset.
- **Handles real BMP variants** — `BITMAPCOREHEADER` and the
  `BITMAPINFOHEADER`/V4/V5 family; 1/4/8-bpp indexed and 24/32-bpp direct color;
  bottom-up and top-down row order; row padding; bit-field masks.
- **Virtualized rendering** so large files stay responsive.
- **Open via dialog, drag-and-drop, or the built-in demo images** (every depth).
- Keyboard navigation (arrow keys move the selection; Shift extends).

## Workspace layout

```
imghex-core/   # pure logic, no GUI deps — the tested core
  src/
    color.rs     Rgba color type (GUI-agnostic)
    region.rs    coarse colored sections (RegionKind + Region)
    field.rs     fine-grained decoded named fields
    model.rs     ParsedImage, per-offset describe(), pixel/palette decoding
    format.rs    ImageFormat trait + registry + parse_auto()
    formats/
      bmp.rs     the BMP parser
      netpbm.rs  the PPM/PGM parser (second format — proves modularity)
    stats.rs     ByteStats + block entropy for the stats panel / entropy strip
    search.rs    byte-pattern search + hex query parsing
    fixtures.rs  image builders used by tests and the demo images
  tests/
    bmp_tests.rs, netpbm_tests.rs   integration tests over the public API

imghex-gui/    # egui/eframe frontend; renders whatever the core produces
  src/main.rs
```

The GUI contains **no format knowledge** — it only draws `Region`s, `Field`s and
`SelectionInfo` values. All parsing and decoding is in `imghex-core` and is fully
unit/integration tested without needing a display.

## Adding a new format

1. Implement `imghex_core::format::ImageFormat` (a `name`, a cheap `detect`, and
   a `parse` that returns a `ParsedImage`).
2. Add it to `format::registry()`.

`ParsedImage` carries everything the GUI needs (regions, fields, summary,
optional palette and `PixelInfo`), so no UI changes are required.

## Building & running

Requires a stable Rust toolchain (`rustup`).

```sh
# run all tests (core logic; no display required)
cargo test -p imghex-core

# lint + format checks
cargo clippy --workspace --all-targets
cargo fmt --all --check

# run the GUI
cargo run -p imghex-gui --release
```

### Windows

**Native (recommended):** on a Windows machine with the MSVC toolchain
installed, build the release binary directly:

```powershell
cargo build -p imghex-gui --release
# → target\release\imghex.exe
```

The binary is marked `windows_subsystem = "windows"` in release builds, so it
launches without a console window. `eframe` uses native Win32 windowing and
`rfd` uses the native file-open dialog.

**Cross-compiling from Linux** (produces an `.exe`):

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt-get install mingw-w64        # provides the gnu linker
cargo build -p imghex-gui --release --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/imghex.exe
```

## Development notes / best practices

- **Test-driven core.** The `imghex-core` crate is developed against the tests
  in `tests/bmp_tests.rs` and the unit tests in the modules; the BMP layout is
  asserted byte-for-byte using fixtures built by `fixtures.rs`.
- **No GUI in the tested path.** Because the core has zero GUI dependencies, the
  full decoding pipeline is testable in CI headlessly.
- Clippy-clean and rustfmt-formatted.
