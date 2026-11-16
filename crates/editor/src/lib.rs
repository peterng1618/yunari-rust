//! The authoring state: one document, its history and its saved baseline.

pub mod document;
pub mod history;
pub mod persistence;
pub mod selection;
pub mod snap;
pub mod transform;
pub mod ui;
pub mod viewport;

pub use document::{Editor, Source};
pub use history::History;
pub use ui::open;
