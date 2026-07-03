//! Fine-grained, named fields decoded from the byte stream.
//!
//! Where [`crate::region::Region`] answers "what section is this byte in?", a
//! [`Field`] answers "what specific value does this byte belong to, and what
//! does it mean?" — e.g. the `bfSize` field of a BMP file header. Fields are
//! leaf-level and non-overlapping; the sidebar shows the innermost field that
//! contains the selected offset.

/// A decoded, named span of bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub start: usize,
    pub end: usize,
    /// Field name, e.g. "bfType" or "biWidth".
    pub name: String,
    /// The decoded value, formatted for display.
    pub value: String,
    /// A human-readable explanation of what the field means.
    pub description: String,
}

impl Field {
    pub fn new(
        start: usize,
        end: usize,
        name: impl Into<String>,
        value: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        debug_assert!(end >= start, "field end must be >= start");
        Self {
            start,
            end,
            name: name.into(),
            value: value.into(),
            description: description.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }
}
