//! Format parsers. Each submodule implements [`crate::format::ImageFormat`].
//!
//! To add a new format, create a module here and register it in
//! [`crate::format::registry`].

pub mod bmp;
pub mod netpbm;
