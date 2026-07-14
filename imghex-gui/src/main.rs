//! `imghex` — a GUI hex editor specialized for image files.
//!
//! The window is split into a virtualized hex grid (left) whose bytes are
//! colored by the file's structural regions, and a sidebar (right) that decodes
//! whatever byte is selected — including resolving a pixel byte of an indexed
//! image to its palette color. All format knowledge lives in `imghex-core`; this
//! binary only renders the model it produces.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui::{Color32, ColorImage, FontId, Key, RichText, Stroke, TextureHandle, TextureOptions};
use imghex_core::color::Rgba;
use imghex_core::format::{parse_auto, ParseError};
use imghex_core::model::{Channel, ParsedImage, SelectionInfo};
use imghex_core::region::{Region, RegionKind};
use imghex_core::search;
use imghex_core::stats::{block_entropies, ByteStats};

/// How the hex grid tints its bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    /// Color each byte by its structural section (header, palette, pixels…).
    Sections,
    /// Color pixel/palette bytes by their actual decoded color.
    Pixels,
}

/// What the image-preview panel shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewMode {
    /// The decoded image.
    Image,
    /// A single bit plane of one channel (LSB steganography visualization).
    BitPlane,
}

/// Which text the find bar searches for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    Hex,
    Text,
}

/// Which hex-grid column typed characters edit. Set by clicking a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditPane {
    /// Type two hex nibbles to overwrite the byte under the cursor.
    Hex,
    /// Type a printable ASCII character to overwrite the byte under the cursor.
    Ascii,
}

/// Target number of entropy blocks across the whole-file strip.
const ENTROPY_BLOCKS: usize = 512;

const BYTES_PER_ROW: usize = 16;

// Fixed hex-grid geometry, shared by the column header and every data row so
// they line up exactly. All widths are in points.
const GUTTER_W: f32 = 78.0; // offset column ("00000000")
const CELL_W: f32 = 22.0; // one hex byte cell ("FF")
const CELL_SPACING: f32 = 4.0; // horizontal gap between cells
const GROUP_GAP: f32 = 8.0; // extra gap between the two 8-byte halves
const ASCII_GAP: f32 = 14.0; // gap between hex and ASCII columns
const ASCII_CELL_W: f32 = 9.0; // one ASCII character cell

// Cursor outline color for the column that keystrokes currently edit (the other
// column's cursor stays white), so it's obvious where a typed byte will land.
const EDIT_CURSOR: Color32 = Color32::from_rgb(0xFF, 0xC0, 0x4D);

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("imghex — image-aware hex editor"),
        ..Default::default()
    };
    eframe::run_native(
        "imghex",
        native_options,
        Box::new(|cc| {
            // Slightly larger default text for readability.
            cc.egui_ctx.set_pixels_per_point(1.1);
            Ok(Box::new(HexApp::new()))
        }),
    )
}

/// Convert a core color to an egui color.
fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Derive a display file name (the final path component) from a path.
fn name_from_path(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Overwrite the byte at `offset` with `value`, returning whether the buffer
/// actually changed (false when the offset is out of range or already equal).
/// A pure helper so the edit logic can be unit tested without any GUI state.
fn overwrite_byte(bytes: &mut [u8], offset: usize, value: u8) -> bool {
    match bytes.get_mut(offset) {
        Some(slot) if *slot != value => {
            *slot = value;
            true
        }
        _ => false,
    }
}

/// Pick black or white text for legibility on top of `bg`.
fn text_on(bg: Rgba) -> Color32 {
    if bg.luminance() > 140 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// The outcome of running the native file dialog on a background thread.
enum DialogResult {
    Picked(std::path::PathBuf),
    Cancelled,
    /// The dialog code panicked; carries the panic message for display.
    Failed(String),
}

struct HexApp {
    file_name: Option<String>,
    /// Full path of the loaded file, if it came from disk; the target for a
    /// plain Save. `None` for the demo and byte-only drops, which force Save As.
    file_path: Option<std::path::PathBuf>,
    bytes: Vec<u8>,
    parsed: Option<Result<ParsedImage, ParseError>>,
    /// Incremented each time a new file is loaded; used to invalidate caches.
    generation: u64,
    /// The fixed end of the selection (where a click/drag began).
    anchor: Option<usize>,
    /// The moving end / active byte of the selection.
    cursor: Option<usize>,
    color_mode: ColorMode,
    status: String,
    /// Receives the result of an in-flight native file dialog running on a
    /// background thread. `Some` while an Open dialog is open.
    open_rx: Option<std::sync::mpsc::Receiver<DialogResult>>,
    /// As `open_rx`, but for a Save As dialog.
    save_rx: Option<std::sync::mpsc::Receiver<DialogResult>>,

    // Editing state.
    /// Unsaved edits since the last load/save.
    dirty: bool,
    /// Which column typed characters overwrite (set by clicking a cell).
    edit_pane: EditPane,
    /// The first of two hex nibbles, typed but not yet committed. The hex cell
    /// renders it as "N_" until the second digit lands.
    pending_nibble: Option<u8>,
    /// OS window title last pushed to the viewport (avoids resending each frame).
    shown_title: String,
    /// Showing the "unsaved changes" confirmation before quitting.
    confirm_quit: bool,
    /// A Save is in flight because the user chose "Save & quit"; close once done.
    quit_after_save: bool,
    /// Set once the user has confirmed quitting so the close guard stops firing.
    force_quit: bool,

    /// Per-block entropy of the whole file, computed once per load.
    entropy_blocks: Vec<f64>,
    entropy_block_size: usize,
    /// Requested scroll target (byte offset) applied on the next frame.
    scroll_to_offset: Option<usize>,

    // Image preview / bit-plane viewer.
    show_preview: bool,
    preview_mode: PreviewMode,
    preview_channel: Channel,
    preview_bit: u8,
    preview_tex: Option<TextureHandle>,
    /// Cache key: (generation, mode, channel, bit) the current texture reflects.
    preview_key: Option<(u64, u8, u8, u8)>,

    // Find bar.
    search_query: String,
    search_mode: SearchMode,
    goto_query: String,
}

impl HexApp {
    fn new() -> Self {
        let mut app = Self {
            file_name: None,
            file_path: None,
            bytes: Vec::new(),
            parsed: None,
            generation: 0,
            anchor: None,
            cursor: None,
            color_mode: ColorMode::Sections,
            status: "Open a BMP file, drop one onto the window, or load the demo.".into(),
            open_rx: None,
            save_rx: None,
            dirty: false,
            edit_pane: EditPane::Hex,
            pending_nibble: None,
            shown_title: String::new(),
            confirm_quit: false,
            quit_after_save: false,
            force_quit: false,
            entropy_blocks: Vec::new(),
            entropy_block_size: 1,
            scroll_to_offset: None,
            show_preview: true,
            preview_mode: PreviewMode::Image,
            preview_channel: Channel::Luma,
            preview_bit: 0,
            preview_tex: None,
            preview_key: None,
            search_query: String::new(),
            search_mode: SearchMode::Hex,
            goto_query: String::new(),
        };
        // Start with the built-in demo so the UI is never empty.
        app.load_bytes("demo.bmp".into(), imghex_core::fixtures::demo_indexed());
        app
    }

    fn load_bytes(&mut self, name: String, bytes: Vec<u8>) {
        self.bytes = bytes;
        self.reparse();
        self.status = format!("Loaded {} ({} bytes).", name, self.bytes.len());
        if let Some(Err(e)) = &self.parsed {
            self.status = format!("{name}: could not parse — {e}");
        }
        self.file_name = Some(name);
        self.file_path = None;
        let init = if self.bytes.is_empty() { None } else { Some(0) };
        self.anchor = init;
        self.cursor = init;
        self.dirty = false;
        self.pending_nibble = None;
        self.edit_pane = EditPane::Hex;
    }

    /// Re-derive every piece of state that depends on `self.bytes`: the parsed
    /// model, the preview-cache key, and the entropy strip. Shared by the initial
    /// load and by each committed edit — because the parsers are pure functions,
    /// re-running `parse_auto` after a mutation keeps regions, fields, the preview
    /// and the entropy strip live with no incremental-parse machinery.
    fn reparse(&mut self) {
        self.parsed = Some(parse_auto(&self.bytes));
        // Invalidate the preview cache and recompute the entropy strip.
        self.generation = self.generation.wrapping_add(1);
        self.preview_key = None;
        self.entropy_block_size = self.bytes.len().div_ceil(ENTROPY_BLOCKS).max(1);
        self.entropy_blocks = block_entropies(&self.bytes, self.entropy_block_size);
    }

    /// Apply text typed this frame as overwrite edits at the cursor, routed to
    /// the hex or ASCII column per `edit_pane`. Same-length edits, so all offsets
    /// stay valid and only a reparse is needed.
    fn handle_edit_input(&mut self, ctx: &egui::Context) {
        if self.cursor.is_none() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Text(text) = event {
                for ch in text.chars() {
                    match self.edit_pane {
                        EditPane::Hex => self.type_hex_nibble(ch),
                        EditPane::Ascii => self.type_ascii_char(ch),
                    }
                }
            }
        }
    }

    /// Handle one typed character in the hex column. The first hex digit is held
    /// as the high nibble; the second commits the full byte.
    fn type_hex_nibble(&mut self, ch: char) {
        let Some(cursor) = self.cursor else { return };
        let Some(nibble) = ch.to_digit(16) else {
            return;
        };
        let nibble = nibble as u8;
        match self.pending_nibble.take() {
            None => {
                self.pending_nibble = Some(nibble);
                self.status = format!("0x{cursor:08X}: type the second hex digit…");
            }
            Some(high) => {
                self.commit_overwrite(cursor, (high << 4) | nibble);
                self.advance_cursor();
            }
        }
    }

    /// Handle one typed character in the ASCII column: overwrite with any
    /// printable ASCII byte (the same range the ASCII column renders as text).
    fn type_ascii_char(&mut self, ch: char) {
        let Some(cursor) = self.cursor else { return };
        let code = ch as u32;
        if !ch.is_ascii() || !(0x20..0x7f).contains(&code) {
            return;
        }
        self.commit_overwrite(cursor, code as u8);
        self.advance_cursor();
    }

    /// Write `value` at `offset` and re-derive the model. Marks the buffer dirty
    /// only when the byte actually changed.
    fn commit_overwrite(&mut self, offset: usize, value: u8) {
        self.pending_nibble = None;
        if !overwrite_byte(&mut self.bytes, offset, value) {
            return; // out of range or already equal — nothing to reparse
        }
        self.dirty = true;
        self.reparse();
        self.status = format!("Set 0x{offset:08X} = 0x{value:02X}.");
    }

    /// Move the cursor one byte forward after an edit (clamped to the last byte),
    /// collapsing any selection so the next keystroke targets the new byte.
    fn advance_cursor(&mut self) {
        if let Some(c) = self.cursor {
            let next = (c + 1).min(self.bytes.len().saturating_sub(1));
            self.select_single(next);
        }
    }

    /// Select a single byte and scroll the hex view to it.
    fn jump_to(&mut self, offset: usize) {
        if offset < self.bytes.len() {
            self.select_single(offset);
            self.scroll_to_offset = Some(offset);
        }
    }

    /// Run the find bar's query, jumping to the next match after the cursor.
    fn run_search(&mut self) {
        let needle = match self.search_mode {
            SearchMode::Hex => match search::parse_hex(&self.search_query) {
                Some(n) => n,
                None => {
                    self.status = "Search: enter valid hex (e.g. \"42 4D\").".into();
                    return;
                }
            },
            SearchMode::Text => self.search_query.clone().into_bytes(),
        };
        if needle.is_empty() {
            self.status = "Search: empty query.".into();
            return;
        }
        let from = self.cursor.map(|c| c + 1).unwrap_or(0);
        match search::find_next(&self.bytes, &needle, from) {
            Some(off) => {
                let total = search::find_all(&self.bytes, &needle).len();
                let end = (off + needle.len() - 1).min(self.bytes.len() - 1);
                self.anchor = Some(off);
                self.cursor = Some(end);
                self.scroll_to_offset = Some(off);
                self.status = format!("Match at 0x{off:08X} ({total} total).");
            }
            None => self.status = "No matches.".into(),
        }
    }

    /// Jump to the offset typed in the go-to field (hex with `0x`, else decimal).
    fn run_goto(&mut self) {
        let q = self.goto_query.trim();
        let parsed = if let Some(hex) = q.strip_prefix("0x").or_else(|| q.strip_prefix("0X")) {
            usize::from_str_radix(hex, 16).ok()
        } else {
            q.parse::<usize>().ok()
        };
        match parsed {
            Some(off) if off < self.bytes.len() => {
                self.jump_to(off);
                self.status = format!("Jumped to 0x{off:08X}.");
            }
            Some(off) => self.status = format!("Offset 0x{off:08X} is past end of file."),
            None => self.status = "Go to: enter a decimal or 0x-prefixed offset.".into(),
        }
    }

    fn open_dialog(&mut self) {
        if self.open_rx.is_some() {
            return; // a dialog is already open
        }
        // The dialog MUST run on its own thread. eframe/winit initializes OLE
        // on the main thread for drag-and-drop, which makes a synchronous rfd
        // dialog on that same thread fail immediately (returning None). A fresh
        // thread initializes COM cleanly for the dialog.
        let (tx, rx) = std::sync::mpsc::channel();
        self.open_rx = Some(rx);
        self.status = "Opening file dialog…".into();
        std::thread::spawn(move || {
            // Capture any panic so the UI can report the real error instead of
            // silently looking like a cancellation.
            let picked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rfd::FileDialog::new()
                    .add_filter("Images (*.bmp)", &["bmp"])
                    .add_filter("All files", &["*"])
                    .set_title("Open image")
                    .pick_file()
            }));
            let result = match picked {
                Ok(Some(path)) => DialogResult::Picked(path),
                Ok(None) => DialogResult::Cancelled,
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    DialogResult::Failed(msg)
                }
            };
            // If the receiver is gone (app closing) the send simply fails.
            let _ = tx.send(result);
        });
    }

    /// Poll the background file dialog, if one is open, and act on its result.
    fn poll_open_dialog(&mut self, ctx: &egui::Context) {
        let result = match &self.open_rx {
            Some(rx) => match rx.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Keep repainting so we notice the result promptly.
                    ctx.request_repaint();
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    DialogResult::Failed("dialog thread ended unexpectedly".into())
                }
            },
            None => return,
        };
        self.open_rx = None;
        match result {
            DialogResult::Picked(path) => self.load_path(&path),
            DialogResult::Cancelled => self.status = "Open cancelled.".into(),
            DialogResult::Failed(msg) => self.status = format!("File dialog failed: {msg}"),
        }
    }

    /// Read a file from disk and load it, surfacing any I/O error.
    fn load_path(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                self.load_bytes(name_from_path(path), bytes);
                // `load_bytes` clears the path (demo/drop cases); restore the real
                // one so a plain Save can write straight back here.
                self.file_path = Some(path.to_path_buf());
            }
            Err(e) => self.status = format!("Failed to read {}: {e}", path.display()),
        }
    }

    /// Save to the current file path, or fall back to Save As when there isn't
    /// one yet (the demo buffer and byte-only drops have no path).
    fn save(&mut self, ctx: &egui::Context) {
        match self.file_path.clone() {
            Some(path) => self.write_to(&path, ctx),
            None => self.save_as_dialog(),
        }
    }

    /// Open a native "save file" dialog on a background thread (same COM/threading
    /// rationale as `open_dialog`).
    fn save_as_dialog(&mut self) {
        if self.save_rx.is_some() {
            return; // a dialog is already open
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.save_rx = Some(rx);
        self.status = "Choose where to save…".into();
        let start_name = self
            .file_name
            .clone()
            .unwrap_or_else(|| "untitled.bin".into());
        std::thread::spawn(move || {
            let picked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rfd::FileDialog::new()
                    .add_filter("Images (*.bmp)", &["bmp"])
                    .add_filter("All files", &["*"])
                    .set_title("Save image as")
                    .set_file_name(start_name)
                    .save_file()
            }));
            let result = match picked {
                Ok(Some(path)) => DialogResult::Picked(path),
                Ok(None) => DialogResult::Cancelled,
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    DialogResult::Failed(msg)
                }
            };
            let _ = tx.send(result);
        });
    }

    /// Poll the background save dialog, if one is open, and write on success.
    fn poll_save_dialog(&mut self, ctx: &egui::Context) {
        let result = match &self.save_rx {
            Some(rx) => match rx.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    DialogResult::Failed("dialog thread ended unexpectedly".into())
                }
            },
            None => return,
        };
        self.save_rx = None;
        match result {
            DialogResult::Picked(path) => self.write_to(&path, ctx),
            DialogResult::Cancelled => {
                self.quit_after_save = false;
                self.status = "Save cancelled.".into();
            }
            DialogResult::Failed(msg) => {
                self.quit_after_save = false;
                self.status = format!("Save dialog failed: {msg}");
            }
        }
    }

    /// Write the current buffer to `path` via `std::fs::write`, clear the dirty
    /// flag, and adopt the path as the new save target. If this save was for a
    /// pending quit, close the window once the bytes are on disk.
    fn write_to(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        match std::fs::write(path, &self.bytes) {
            Ok(()) => {
                let name = name_from_path(path);
                self.status = format!("Saved {} ({} bytes).", name, self.bytes.len());
                self.file_name = Some(name);
                self.file_path = Some(path.to_path_buf());
                self.dirty = false;
                if self.quit_after_save {
                    self.quit_after_save = false;
                    self.force_quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(e) => {
                self.quit_after_save = false;
                self.status = format!("Failed to save {}: {e}", path.display());
            }
        }
    }

    fn region_at(&self, offset: usize) -> Option<&Region> {
        match &self.parsed {
            Some(Ok(img)) => img.region_at(offset),
            _ => None,
        }
    }

    /// Background color for a byte at `offset`, honoring the current color mode.
    fn byte_bg(&self, offset: usize) -> Rgba {
        match self.color_mode {
            ColorMode::Sections => self
                .region_at(offset)
                .map(|r| r.kind.color())
                .unwrap_or(Rgba::rgb(0x30, 0x30, 0x30)),
            ColorMode::Pixels => {
                if let Some(Ok(img)) = &self.parsed {
                    if let Some(c) = img.byte_color(offset, &self.bytes) {
                        return c;
                    }
                }
                // Non-pixel bytes (headers, gaps) get a dim neutral in this mode.
                Rgba::rgb(0x28, 0x28, 0x28)
            }
        }
    }

    /// The inclusive `[start, end]` selection range, if any.
    fn sel_range(&self) -> Option<(usize, usize)> {
        match (self.anchor, self.cursor) {
            (Some(a), Some(c)) => Some((a.min(c), a.max(c))),
            _ => None,
        }
    }

    /// Number of bytes currently selected.
    fn sel_len(&self) -> usize {
        self.sel_range().map(|(a, b)| b - a + 1).unwrap_or(0)
    }

    fn is_selected(&self, offset: usize) -> bool {
        self.sel_range()
            .map(|(a, b)| offset >= a && offset <= b)
            .unwrap_or(false)
    }

    /// Select a single byte (collapse the range).
    fn select_single(&mut self, offset: usize) {
        self.anchor = Some(offset);
        self.cursor = Some(offset);
    }

    /// Extend the selection to `offset`, keeping the existing anchor.
    fn select_extend(&mut self, offset: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(offset);
        }
        self.cursor = Some(offset);
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.bytes.is_empty() {
            return;
        }
        // Don't hijack the keyboard while a text field (find / go-to) is focused,
        // or arrow keys and typed digits would leak into the hex grid.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let last = self.bytes.len() - 1;
        let mut cur = self.cursor.unwrap_or(0);
        let mut moved = false;
        let shift = ctx.input(|i| i.modifiers.shift);
        ctx.input(|i| {
            if i.key_pressed(Key::ArrowRight) && cur < last {
                cur += 1;
                moved = true;
            }
            if i.key_pressed(Key::ArrowLeft) && cur > 0 {
                cur -= 1;
                moved = true;
            }
            if i.key_pressed(Key::ArrowDown) && cur + BYTES_PER_ROW <= last {
                cur += BYTES_PER_ROW;
                moved = true;
            }
            if i.key_pressed(Key::ArrowUp) && cur >= BYTES_PER_ROW {
                cur -= BYTES_PER_ROW;
                moved = true;
            }
        });
        if moved {
            // Moving abandons a half-typed hex byte.
            self.pending_nibble = None;
            // Shift+arrows extend the selection; plain arrows move a single byte.
            if shift {
                self.select_extend(cur);
            } else {
                self.select_single(cur);
            }
        }

        // Characters typed this frame overwrite the byte under the cursor.
        self.handle_edit_input(ctx);
    }
}

impl eframe::App for HexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pick up the result of a background file dialog, if one is running.
        self.poll_open_dialog(ctx);
        self.poll_save_dialog(ctx);

        // Guard the window close against unsaved edits: cancel it and raise the
        // confirmation dialog until the user has resolved it (force_quit).
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.force_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }

        // Keep the OS window title reflecting the file and its dirty state.
        let title = match &self.file_name {
            Some(name) => format!("imghex — {name}{}", if self.dirty { " •" } else { "" }),
            None => "imghex — image-aware hex editor".to_string(),
        };
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }

        // Accept drag-and-drop of a file.
        let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped.into_iter().next() {
            if let Some(path) = &file.path {
                // Native platforms deliver a real filesystem path.
                self.load_path(path);
            } else if let Some(bytes) = &file.bytes {
                // Web / some platforms deliver the bytes directly.
                let name = if file.name.is_empty() {
                    "dropped file".to_string()
                } else {
                    file.name.clone()
                };
                self.load_bytes(name, bytes.to_vec());
            } else {
                self.status = "Could not read the dropped file.".into();
            }
        }

        // Overlay + feedback while files are hovering over the window. This also
        // keeps the UI repainting so the drop event is processed promptly.
        let hovering = ctx.input(|i| i.raw.hovered_files.len());
        if hovering > 0 {
            ctx.request_repaint();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dnd_overlay"),
            ));
            let screen = ctx.screen_rect();
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(180));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop image to open",
                FontId::proportional(28.0),
                Color32::WHITE,
            );
        }

        self.handle_keys(ctx);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("📂 Open…").clicked() {
                    self.open_dialog();
                }
                let has_bytes = !self.bytes.is_empty();
                if ui
                    .add_enabled(has_bytes, egui::Button::new("💾 Save"))
                    .on_hover_text(
                        "Write changes back to the file (Save As if it has no path yet).",
                    )
                    .clicked()
                {
                    self.save(ctx);
                }
                if ui
                    .add_enabled(has_bytes, egui::Button::new("Save As…"))
                    .clicked()
                {
                    self.save_as_dialog();
                }
                ui.menu_button("🎨 Load demo ▾", |ui| {
                    use imghex_core::fixtures;
                    if ui.button("8-bpp gradient (256-color)").clicked() {
                        self.load_bytes("demo-8bpp.bmp".into(), fixtures::demo_indexed());
                        ui.close_menu();
                    }
                    if ui.button("4-bpp ramp (16-color)").clicked() {
                        self.load_bytes("demo-4bpp.bmp".into(), fixtures::demo_4bpp());
                        ui.close_menu();
                    }
                    if ui.button("1-bpp checkerboard (2-color)").clicked() {
                        self.load_bytes("demo-1bpp.bmp".into(), fixtures::demo_1bpp());
                        ui.close_menu();
                    }
                    if ui.button("24-bpp true color").clicked() {
                        self.load_bytes("demo-24bpp.bmp".into(), fixtures::demo_24bpp());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("JPEG marker structure").clicked() {
                        self.load_bytes("demo.jpg".into(), fixtures::demo_jpeg());
                        ui.close_menu();
                    }
                });
                ui.separator();
                ui.label("Color:");
                ui.selectable_value(&mut self.color_mode, ColorMode::Sections, "Sections")
                    .on_hover_text("Tint bytes by file structure (header, palette, pixels…).");
                ui.selectable_value(&mut self.color_mode, ColorMode::Pixels, "Pixels")
                    .on_hover_text("Tint pixel and palette bytes by their decoded color.");
                ui.separator();
                ui.checkbox(&mut self.show_preview, "Preview");
                ui.separator();
                if let Some(name) = &self.file_name {
                    ui.label(RichText::new(name).strong());
                    if self.dirty {
                        ui.label(RichText::new("• unsaved").color(EDIT_CURSOR))
                            .on_hover_text("This buffer has edits that aren't saved to disk.");
                    }
                }
            });

            // Find + go-to row.
            ui.horizontal(|ui| {
                ui.label("Find:");
                ui.selectable_value(&mut self.search_mode, SearchMode::Hex, "Hex");
                ui.selectable_value(&mut self.search_mode, SearchMode::Text, "Text");
                let hint = match self.search_mode {
                    SearchMode::Hex => "e.g. 42 4D",
                    SearchMode::Text => "e.g. BM",
                };
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .desired_width(140.0)
                        .hint_text(hint),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.button("Next").clicked() || enter {
                    self.run_search();
                }
                ui.separator();
                ui.label("Go to:");
                let goto = ui.add(
                    egui::TextEdit::singleline(&mut self.goto_query)
                        .desired_width(90.0)
                        .hint_text("0x… / dec"),
                );
                let goto_enter = goto.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.button("Jump").clicked() || goto_enter {
                    self.run_goto();
                }
            });
            ui.add_space(2.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.label(&self.status);
            ui.add_space(2.0);
        });

        // Whole-file entropy strip, just under the toolbar.
        if !self.entropy_blocks.is_empty() {
            egui::TopBottomPanel::top("entropy").show(ctx, |ui| {
                self.draw_entropy_strip(ui);
            });
        }

        egui::SidePanel::right("sidebar")
            .resizable(true)
            .default_width(340.0)
            .width_range(260.0..=520.0)
            .show(ctx, |ui| {
                self.draw_sidebar(ui);
            });

        if self.show_preview {
            egui::SidePanel::left("preview")
                .resizable(true)
                .default_width(300.0)
                .width_range(200.0..=560.0)
                .show(ctx, |ui| {
                    self.draw_preview(ui, ctx);
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_hex(ui);
        });

        // Unsaved-changes confirmation, shown when a quit was intercepted above.
        if self.confirm_quit {
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "“{}” has unsaved edits.",
                        self.file_name.as_deref().unwrap_or("This file")
                    ));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save & quit").clicked() {
                            self.confirm_quit = false;
                            // Close once the save completes (direct write closes
                            // immediately; Save As closes when the dialog returns).
                            self.quit_after_save = true;
                            self.save(ctx);
                        }
                        if ui.button("Discard & quit").clicked() {
                            self.confirm_quit = false;
                            self.force_quit = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_quit = false;
                            self.quit_after_save = false;
                        }
                    });
                });
        }
    }
}

impl HexApp {
    fn draw_hex(&mut self, ui: &mut egui::Ui) {
        if self.bytes.is_empty() {
            ui.centered_and_justified(|ui| ui.label("No file loaded."));
            return;
        }

        let font = FontId::monospace(14.0);
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
        let num_rows = self.bytes.len().div_ceil(BYTES_PER_ROW);

        // Column header — laid out with the exact same geometry as a data row.
        let header_color = ui.visuals().weak_text_color();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CELL_SPACING;
            let (gutter, _) =
                ui.allocate_exact_size(egui::vec2(GUTTER_W, row_h), egui::Sense::hover());
            ui.painter().text(
                gutter.left_center() + egui::vec2(2.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "Offset",
                font.clone(),
                header_color,
            );
            for col in 0..BYTES_PER_ROW {
                if col == 8 {
                    ui.allocate_exact_size(egui::vec2(GROUP_GAP, row_h), egui::Sense::hover());
                }
                let (cell, _) =
                    ui.allocate_exact_size(egui::vec2(CELL_W, row_h), egui::Sense::hover());
                ui.painter().text(
                    cell.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{col:02X}"),
                    font.clone(),
                    header_color,
                );
            }
        });
        ui.separator();

        // Pointer state for click / shift-click / drag selection. We hit-test
        // cell rectangles manually so a drag keeps extending even though egui
        // routes hover to the drag's origin widget.
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let shift = ui.input(|i| i.modifiers.shift);
        let mut hot: Option<usize> = None;
        // Which column the hovered cell is in, so a click also picks the edit pane.
        let mut hot_pane = self.edit_pane;

        let mut scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .drag_to_scroll(false); // dragging selects bytes instead of scrolling
                                    // Honor a pending jump (from search, go-to, or the entropy strip).
        if let Some(off) = self.scroll_to_offset.take() {
            let row = off / BYTES_PER_ROW;
            // Aim a little above the target so it isn't jammed against the top.
            let target = (row as f32 * row_h - 2.0 * row_h).max(0.0);
            scroll = scroll.vertical_scroll_offset(target);
        }
        scroll.show_rows(ui, row_h, num_rows, |ui, row_range| {
            ui.spacing_mut().item_spacing = egui::vec2(CELL_SPACING, 2.0);
            for row in row_range {
                let base = row * BYTES_PER_ROW;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CELL_SPACING;

                    // Offset gutter.
                    let (gutter, _) =
                        ui.allocate_exact_size(egui::vec2(GUTTER_W, row_h), egui::Sense::hover());
                    ui.painter().text(
                        gutter.left_center() + egui::vec2(2.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{base:08X}"),
                        font.clone(),
                        header_color,
                    );

                    // Hex byte cells.
                    for col in 0..BYTES_PER_ROW {
                        if col == 8 {
                            ui.allocate_exact_size(
                                egui::vec2(GROUP_GAP, row_h),
                                egui::Sense::hover(),
                            );
                        }
                        let off = base + col;
                        if off >= self.bytes.len() {
                            // Keep columns aligned on the final partial row.
                            ui.allocate_exact_size(egui::vec2(CELL_W, row_h), egui::Sense::hover());
                            continue;
                        }
                        let resp = self.hex_cell(ui, &font, row_h, off);
                        if let Some(p) = pointer_pos {
                            if resp.rect.contains(p) {
                                hot = Some(off);
                                hot_pane = EditPane::Hex;
                            }
                        }
                    }

                    ui.add_space(ASCII_GAP);

                    // ASCII column.
                    for col in 0..BYTES_PER_ROW {
                        let off = base + col;
                        if off >= self.bytes.len() {
                            break;
                        }
                        let resp = self.ascii_cell(ui, &font, row_h, off);
                        if let Some(p) = pointer_pos {
                            if resp.rect.contains(p) {
                                hot = Some(off);
                                hot_pane = EditPane::Ascii;
                            }
                        }
                    }
                });
            }
        });

        // Apply selection from the pointer.
        if let Some(off) = hot {
            if primary_pressed {
                // Clicking a cell also chooses which column edits target, and
                // abandons any half-typed hex byte.
                self.edit_pane = hot_pane;
                self.pending_nibble = None;
                if shift {
                    self.select_extend(off);
                } else {
                    self.select_single(off);
                }
            } else if primary_down && self.anchor.is_some() {
                // Drag extends from the anchor set on press.
                self.cursor = Some(off);
            }
        }
    }

    /// Draw one fixed-width, clickable hex byte cell with a region-colored
    /// background. Painting directly (rather than using a `Button`) keeps every
    /// cell an identical width so the header and rows align.
    fn hex_cell(
        &self,
        ui: &mut egui::Ui,
        font: &FontId,
        row_h: f32,
        offset: usize,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELL_W, row_h), egui::Sense::click());
        let bg = self.byte_bg(offset);
        ui.painter().rect_filled(rect, 2.0, to_color32(bg));
        if self.is_selected(offset) {
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_white_alpha(60));
        }
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_white_alpha(28));
        }
        if self.cursor == Some(offset) {
            // Highlight the cursor when the hex column is the active edit target.
            let color = if self.edit_pane == EditPane::Hex {
                EDIT_CURSOR
            } else {
                Color32::WHITE
            };
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(2.0, color));
        }
        // Show a half-typed hex byte ("N_") until the second nibble commits it.
        let label = match (self.cursor == Some(offset), self.pending_nibble) {
            (true, Some(high)) => format!("{high:X}_"),
            _ => format!("{:02X}", self.bytes[offset]),
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            text_on(bg),
        );
        resp
    }

    /// Draw one fixed-width ASCII cell for the given byte.
    fn ascii_cell(
        &self,
        ui: &mut egui::Ui,
        font: &FontId,
        row_h: f32,
        offset: usize,
    ) -> egui::Response {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ASCII_CELL_W, row_h), egui::Sense::click());
        let b = self.bytes[offset];
        let bg = self.byte_bg(offset);
        if self.is_selected(offset) {
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_white_alpha(60));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_white_alpha(28));
        }
        if self.cursor == Some(offset) {
            let color = if self.edit_pane == EditPane::Ascii {
                EDIT_CURSOR
            } else {
                Color32::WHITE
            };
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.5, color));
        }
        let ch = if (0x20..0x7f).contains(&b) {
            b as char
        } else {
            '.'
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            ch,
            font.clone(),
            to_color32(bg),
        );
        resp
    }

    /// Draw the whole-file entropy strip; clicking jumps to that offset.
    fn draw_entropy_strip(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
        let n = self.entropy_blocks.len();
        if n == 0 {
            return;
        }
        let painter = ui.painter();
        let bar_w = rect.width() / n as f32;
        for (i, &e) in self.entropy_blocks.iter().enumerate() {
            let x0 = rect.left() + i as f32 * bar_w;
            let bar = egui::Rect::from_min_max(
                egui::pos2(x0, rect.top()),
                egui::pos2(x0 + bar_w.max(1.0), rect.bottom()),
            );
            painter.rect_filled(bar, 0.0, entropy_color((e / 8.0) as f32));
        }
        // Marker at the current cursor position.
        if let (Some(c), false) = (self.cursor, self.bytes.is_empty()) {
            let frac = c as f32 / self.bytes.len() as f32;
            let x = rect.left() + frac * rect.width();
            painter.vline(x, rect.y_range(), Stroke::new(1.0, Color32::WHITE));
        }

        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let frac = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let off = ((frac * self.bytes.len() as f32) as usize)
                    .min(self.bytes.len().saturating_sub(1));
                self.jump_to(off);
            }
        }
        resp.on_hover_text("File entropy per block (blue = low, red = high). Click to jump.");
    }

    /// Draw the image preview / bit-plane viewer.
    fn draw_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Preview");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.preview_mode, PreviewMode::Image, "Image");
            ui.selectable_value(&mut self.preview_mode, PreviewMode::BitPlane, "Bit plane")
                .on_hover_text("Show one bit plane — bit 0 (LSB) reveals LSB steganography.");
        });
        if self.preview_mode == PreviewMode::BitPlane {
            ui.horizontal(|ui| {
                ui.label("Channel:");
                egui::ComboBox::from_id_source("preview_channel")
                    .selected_text(channel_name(self.preview_channel))
                    .show_ui(ui, |ui| {
                        for ch in [Channel::Red, Channel::Green, Channel::Blue, Channel::Luma] {
                            ui.selectable_value(&mut self.preview_channel, ch, channel_name(ch));
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Bit:");
                ui.add(egui::Slider::new(&mut self.preview_bit, 0..=7));
                let tag = match self.preview_bit {
                    0 => "(LSB)",
                    7 => "(MSB)",
                    _ => "",
                };
                ui.label(RichText::new(tag).weak());
            });
        }
        ui.separator();

        let tex = self.ensure_preview_texture(ctx);
        match tex {
            Some(tex) => {
                let img_size = tex.size_vec2();
                let avail = ui.available_size();
                let scale = (avail.x / img_size.x).min(avail.y / img_size.y).max(0.01);
                let disp = img_size * scale;
                let sized = egui::load::SizedTexture::new(tex.id(), disp);
                let resp = ui.add(egui::Image::new(sized).sense(egui::Sense::click()));

                // Click a pixel → select the byte that encodes it.
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let rel = p - resp.rect.min;
                        let px = (rel.x / resp.rect.width() * img_size.x).floor() as u32;
                        let py = (rel.y / resp.rect.height() * img_size.y).floor() as u32;
                        let mut jump = None;
                        if let Some(Ok(image)) = &self.parsed {
                            if let Some(pi) = &image.pixel_info {
                                jump = pi.byte_offset_of(px, py);
                            }
                        }
                        if let Some(off) = jump {
                            self.jump_to(off);
                        }
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} × {} px", img_size.x as u32, img_size.y as u32))
                        .weak(),
                );
            }
            None => {
                ui.label("No preview available for this file.");
            }
        }
    }

    /// Build (or reuse a cached) texture for the preview panel.
    fn ensure_preview_texture(&mut self, ctx: &egui::Context) -> Option<TextureHandle> {
        let img = match &self.parsed {
            Some(Ok(img)) => img,
            _ => return None,
        };
        let mode_k = match self.preview_mode {
            PreviewMode::Image => 0,
            PreviewMode::BitPlane => 1,
        };
        let chan_k = match self.preview_channel {
            Channel::Red => 0,
            Channel::Green => 1,
            Channel::Blue => 2,
            Channel::Luma => 3,
        };
        let key = (self.generation, mode_k, chan_k, self.preview_bit);
        if self.preview_key == Some(key) {
            return self.preview_tex.clone();
        }

        let rendered = img.render(&self.bytes)?;
        let size = [rendered.width as usize, rendered.height as usize];
        let color_image = match self.preview_mode {
            PreviewMode::Image => ColorImage {
                size,
                pixels: rendered.pixels.iter().map(|c| to_color32(*c)).collect(),
            },
            PreviewMode::BitPlane => {
                let plane = rendered.bit_plane(self.preview_channel, self.preview_bit);
                ColorImage {
                    size,
                    pixels: plane
                        .iter()
                        .map(|&b| if b { Color32::WHITE } else { Color32::BLACK })
                        .collect(),
                }
            }
        };
        let tex = ctx.load_texture("preview", color_image, TextureOptions::NEAREST);
        self.preview_key = Some(key);
        self.preview_tex = Some(tex);
        self.preview_tex.clone()
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Inspector");
            ui.add_space(4.0);

            match &self.parsed {
                Some(Ok(img)) => {
                    // File summary.
                    egui::CollapsingHeader::new("File summary")
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::Grid::new("summary_grid")
                                .num_columns(2)
                                .spacing([12.0, 4.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for (k, v) in &img.summary {
                                        ui.label(RichText::new(k).weak());
                                        ui.label(v);
                                        ui.end_row();
                                    }
                                });
                        });

                    ui.add_space(6.0);
                    draw_legend(ui, img);

                    if !img.palette.is_empty() {
                        ui.add_space(6.0);
                        draw_palette(ui, &img.palette);
                    }

                    ui.add_space(6.0);
                    ui.separator();

                    // Multi-byte selection → statistics; single byte → decode.
                    if self.sel_len() > 1 {
                        let (a, b) = self.sel_range().unwrap();
                        ui.heading(format!("Selection · {} bytes", b - a + 1));
                        ui.label(
                            RichText::new(format!("0x{a:08X} – 0x{b:08X}"))
                                .monospace()
                                .weak(),
                        );
                        if let Some(stats) = ByteStats::compute(&self.bytes[a..=b]) {
                            ui.add_space(4.0);
                            draw_stats(ui, &stats);
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Tip: click a single byte for full field decoding.")
                                .weak()
                                .italics(),
                        );
                    } else {
                        ui.heading("Selection");
                        match self.cursor.and_then(|off| img.describe(off, &self.bytes)) {
                            Some(info) => draw_selection(ui, &info),
                            None => {
                                ui.label("Click a byte to inspect it. Shift-click or drag to select a range.");
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    ui.colored_label(Color32::LIGHT_RED, format!("Parse error: {e}"));
                    ui.label("The raw bytes are still shown on the left.");
                }
                None => {
                    ui.label("No file loaded.");
                }
            }
        });
    }
}

fn draw_legend(ui: &mut egui::Ui, img: &ParsedImage) {
    egui::CollapsingHeader::new("Legend")
        .default_open(true)
        .show(ui, |ui| {
            // Show only the region kinds actually present, in file order.
            let mut seen: Vec<RegionKind> = Vec::new();
            for r in &img.regions {
                if !seen.contains(&r.kind) {
                    seen.push(r.kind);
                }
            }
            for kind in seen {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 3.0, to_color32(kind.color()));
                    ui.label(kind.label());
                });
            }
        });
}

fn draw_selection(ui: &mut egui::Ui, info: &SelectionInfo) {
    egui::Grid::new("sel_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Offset").weak());
            ui.label(format!("0x{:08X} ({})", info.offset, info.offset));
            ui.end_row();

            ui.label(RichText::new("Byte").weak());
            ui.label(format!(
                "0x{0:02X} · {0} · 0b{0:08b} · '{1}'",
                info.byte,
                if (0x20..0x7f).contains(&info.byte) {
                    info.byte as char
                } else {
                    '.'
                }
            ));
            ui.end_row();

            if let Some(name) = &info.region_name {
                ui.label(RichText::new("Section").weak());
                let kind = info.region_kind.map(|k| k.label()).unwrap_or("");
                ui.label(format!("{name} ({kind})"));
                ui.end_row();
            }
        });

    if let Some(field) = &info.field {
        ui.add_space(6.0);
        ui.label(RichText::new(&field.name).strong());
        ui.label(RichText::new(&field.value).monospace());
        ui.label(RichText::new(&field.description).weak().italics());
    }

    if !info.details.is_empty() {
        ui.add_space(6.0);
        egui::Grid::new("detail_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in &info.details {
                    ui.label(RichText::new(k).weak());
                    ui.label(RichText::new(v).monospace());
                    ui.end_row();
                }
            });
    }

    if !info.swatches.is_empty() {
        ui.add_space(6.0);
        let label = if info.swatches.len() > 1 {
            format!("Colors in this byte ({})", info.swatches.len())
        } else {
            "Color".to_string()
        };
        ui.label(RichText::new(label).weak());
        for s in &info.swatches {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 20.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, to_color32(s.color));
                ui.painter()
                    .rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::GRAY));
                ui.label(RichText::new(s.color.to_hex()).monospace());
                if !s.label.is_empty() {
                    ui.label(RichText::new(&s.label).weak().small());
                }
            });
        }
    }
}

/// Draw the whole palette as a wrapped grid of swatches. Hovering a swatch
/// shows its index and hex value.
fn draw_palette(ui: &mut egui::Ui, palette: &[Rgba]) {
    egui::CollapsingHeader::new(format!("Palette ({} colors)", palette.len()))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                for (i, c) in palette.iter().enumerate() {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, to_color32(*c));
                    ui.painter()
                        .rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(40)));
                    resp.on_hover_text(format!("index {i} · {}", c.to_hex()));
                }
            });
        });
}

/// Draw summary statistics and a byte-value histogram for a selection.
fn draw_stats(ui: &mut egui::Ui, stats: &ByteStats) {
    egui::Grid::new("stats_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            ui.label(RichText::new("Count").weak());
            ui.label(format!("{}", stats.count));
            ui.end_row();
            ui.label(RichText::new("Range").weak());
            ui.label(format!("{} – {}", stats.min, stats.max));
            ui.end_row();
            ui.label(RichText::new("Mean").weak());
            ui.label(format!("{:.2}", stats.mean));
            ui.end_row();
            ui.label(RichText::new("Distinct").weak());
            ui.label(format!("{} / 256", stats.distinct));
            ui.end_row();
            ui.label(RichText::new("Entropy").weak());
            ui.label(format!("{:.3} bits/byte", stats.entropy));
            ui.end_row();
            ui.label(RichText::new("Most common").weak());
            ui.label(format!(
                "0x{:02X} (×{})",
                stats.most_common, stats.most_common_count
            ));
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.label(RichText::new("Byte histogram (0x00 – 0xFF)").weak());
    draw_histogram(ui, &stats.histogram);
}

/// Paint a 256-bin byte histogram scaled to its tallest bar.
fn draw_histogram(ui: &mut egui::Ui, histogram: &[u32; 256]) {
    let max = histogram.iter().copied().max().unwrap_or(1).max(1);
    let height = 90.0;
    let width = ui.available_width().max(64.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_gray(18));
    let bar_w = rect.width() / 256.0;
    let bar_color = Color32::from_rgb(0x66, 0x99, 0xCC);
    for (i, &count) in histogram.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let frac = count as f32 / max as f32;
        let bar_h = frac * (height - 2.0);
        let x0 = rect.left() + i as f32 * bar_w;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x0, rect.bottom() - bar_h),
            egui::pos2(x0 + bar_w.max(1.0), rect.bottom()),
        );
        painter.rect_filled(bar, 0.0, bar_color);
    }
}

/// Map an entropy fraction (0.0..=1.0) to a blue→green→red heat color.
fn entropy_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    // 0.0 blue → 0.5 green → 1.0 red.
    let (r, g, b) = if t < 0.5 {
        let u = t / 0.5;
        (0.0, u, 1.0 - u)
    } else {
        let u = (t - 0.5) / 0.5;
        (u, 1.0 - u, 0.0)
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn channel_name(c: Channel) -> &'static str {
    match c {
        Channel::Red => "Red",
        Channel::Green => "Green",
        Channel::Blue => "Blue",
        Channel::Luma => "Luminance",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imghex_core::fixtures;

    #[test]
    fn overwrite_reports_whether_it_changed() {
        let mut bytes = vec![0x10, 0x20, 0x30];
        // A real change returns true and mutates the buffer.
        assert!(overwrite_byte(&mut bytes, 1, 0x99));
        assert_eq!(bytes[1], 0x99);
        // Writing the same value is a no-op.
        assert!(!overwrite_byte(&mut bytes, 1, 0x99));
        // Out-of-range offsets are ignored, not a panic.
        assert!(!overwrite_byte(&mut bytes, 99, 0x00));
        assert_eq!(bytes, vec![0x10, 0x99, 0x30]);
    }

    #[test]
    fn edit_changes_what_the_parser_sees() {
        // The core sanity test called out in the ticket: an overwrite edit feeds
        // straight back into `parse_auto`, so mutating a structural byte changes
        // the decoded model. Corrupting the BMP magic makes the parser reject it.
        let mut bytes = fixtures::demo_indexed();
        assert!(parse_auto(&bytes).is_ok(), "demo fixture should parse");
        assert!(overwrite_byte(&mut bytes, 0, 0x00)); // was 'B' (0x42)
        assert!(
            parse_auto(&bytes).is_err(),
            "corrupting the magic must break the parse"
        );
    }

    #[test]
    fn edit_preserves_length_so_offsets_stay_valid() {
        // Phase 1 is overwrite-only: the buffer length is invariant, which is why
        // no cursor/selection clamping is needed (that's phase 4's problem).
        let mut bytes = fixtures::demo_indexed();
        let before = bytes.len();
        overwrite_byte(&mut bytes, before / 2, 0xAB);
        assert_eq!(bytes.len(), before);
    }
}
