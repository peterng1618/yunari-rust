//! The authoring state: one document, its history and its saved baseline.

pub mod document;
#[cfg(test)]
pub mod fixtures;
pub mod history;
pub mod layers;
pub mod persistence;
pub mod selection;
pub mod snap;
pub mod transform;
pub mod ui;
pub mod viewport;

pub use document::{Editor, Source};
pub use history::History;
pub use ui::open;
