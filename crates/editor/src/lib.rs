//! The authoring state: one document, its history and its saved baseline.

pub mod document;
pub mod history;
pub mod persistence;

pub use document::{Editor, Source};
pub use history::History;
