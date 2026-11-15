#![forbid(unsafe_code)]
//! Renderer-independent decisions: what a reading says, where a run sits, and
//! what geometry a frame paints.

mod affine;
mod format;

pub use affine::Affine;
pub use format::{format_number, format_unit};
