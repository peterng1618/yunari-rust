#![forbid(unsafe_code)]
//! Renderer-independent decisions: what a reading says, where a run sits, and
//! what geometry a frame paints.

mod affine;
mod color;
mod display_list;
mod format;
mod path;
mod plan;
pub mod text;

pub use affine::Affine;
pub use color::{Color, parse_color};
pub use display_list::{DisplayList, GradientStop, Op, Paint, PathCommand, Shape, Stroke};
pub use format::{format_number, format_unit};
pub use path::{PathError, parse as parse_path};
pub use plan::{IssueCode, PlanIssue, build};
pub use text::{Fonts, TextStyle};
