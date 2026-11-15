#![forbid(unsafe_code)]
//! Renderer-independent decisions: what a reading says, where a run sits, and
//! what geometry a frame paints.

mod affine;
mod color;
mod display_list;
mod format;
mod plan;

pub use affine::Affine;
pub use color::{Color, parse_color};
pub use display_list::{DisplayList, GradientStop, Op, Paint, PathCommand, Shape, Stroke};
pub use format::{format_number, format_unit};
pub use plan::{IssueCode, PlanIssue, build};
