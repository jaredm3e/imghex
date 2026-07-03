//! The pluggable image-format interface.
//!
//! Each supported format implements [`ImageFormat`]. The [`registry`] returns
//! every known format; [`parse_auto`] picks the first one that recognizes the
//! bytes. Adding a new format (PNG, GIF, ...) means implementing the trait and
//! adding one line to [`registry`] — no GUI changes required.

use crate::model::ParsedImage;
use std::fmt;

/// An error produced while parsing a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The bytes are not (or not enough of) this format.
    NotRecognized,
    /// The file is truncated: needed at least `needed` bytes but had `got`.
    Truncated { needed: usize, got: usize },
    /// A field held a value the parser cannot handle.
    Unsupported(String),
    /// A structural inconsistency (e.g. a header pointing outside the file).
    Malformed(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NotRecognized => write!(f, "not recognized as this format"),
            ParseError::Truncated { needed, got } => {
                write!(f, "file truncated: needed {needed} bytes, got {got}")
            }
            ParseError::Unsupported(m) => write!(f, "unsupported: {m}"),
            ParseError::Malformed(m) => write!(f, "malformed: {m}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// A parser for one image format.
pub trait ImageFormat {
    /// A stable, human-readable name (e.g. "BMP").
    fn name(&self) -> &'static str;

    /// Cheaply test whether `bytes` look like this format (magic number check).
    fn detect(&self, bytes: &[u8]) -> bool;

    /// Fully parse `bytes` into a [`ParsedImage`].
    fn parse(&self, bytes: &[u8]) -> Result<ParsedImage, ParseError>;
}

/// Every format the core knows about, in detection priority order.
pub fn registry() -> Vec<Box<dyn ImageFormat>> {
    vec![
        Box::new(crate::formats::bmp::BmpFormat),
        Box::new(crate::formats::netpbm::NetpbmFormat),
    ]
}

/// Parse `bytes` with the first registered format that recognizes them.
///
/// Returns [`ParseError::NotRecognized`] if no format matches.
pub fn parse_auto(bytes: &[u8]) -> Result<ParsedImage, ParseError> {
    for fmt in registry() {
        if fmt.detect(bytes) {
            return fmt.parse(bytes);
        }
    }
    Err(ParseError::NotRecognized)
}
