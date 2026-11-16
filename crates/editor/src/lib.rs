//! The authoring state: one document, its history and its saved baseline.

pub mod arrange;
pub mod assets;
pub mod bindings;
pub mod clipboard;
pub mod document;
#[cfg(test)]
pub mod fixtures;
pub mod grouping;
pub mod history;
pub mod inspector;
pub mod layers;
pub mod palette;
pub mod persistence;
pub mod selection;
pub mod snap;
pub mod tokens;
pub mod transform;
pub mod typography;
pub mod ui;
pub mod viewport;
pub mod widgets;

pub use document::{Editor, Source};
pub use history::History;
pub use ui::menus::Command;
pub use ui::{open, open_command, open_with};
