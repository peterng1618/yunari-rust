#![forbid(unsafe_code)]
//! Renderer-independent decisions: what a reading says, where a run sits, and
//! what geometry a frame paints.

mod format;

pub use format::{format_number, format_unit};
