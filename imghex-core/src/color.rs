//! A minimal, GUI-agnostic color type used across the core crate.
//!
//! The core crate deliberately avoids depending on any GUI toolkit, so it
//! defines its own color type. Frontends convert this into their own color
//! representation (e.g. `egui::Color32`).

/// An 8-bit-per-channel RGBA color.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[allow(clippy::self_named_constructors)] // `rgb`/`rgba` mirror egui's Color32 API.
impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// A `#RRGGBB` (or `#RRGGBBAA` when not fully opaque) hex string.
    pub fn to_hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// Perceived luminance in the 0..=255 range (Rec. 601 weights).
    ///
    /// Useful for deciding whether to draw text in black or white on top of
    /// this color.
    pub fn luminance(&self) -> u8 {
        let l = 0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32;
        l.round().clamp(0.0, 255.0) as u8
    }
}
