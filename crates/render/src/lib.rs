#![forbid(unsafe_code)]
//! Renderer-independent decisions: what a reading says, where a run sits, and
//! what geometry a frame paints.

mod affine;
pub mod background;
pub mod chart;
mod color;
mod display_list;
mod format;
pub mod history;
pub mod icons;
pub mod image;
pub mod instant;
mod path;
pub mod plan;
pub mod text;
pub mod video;

pub use affine::Affine;
pub use color::{Color, parse_color};
pub use display_list::{
    Backdrop, ChartFamily, DisplayList, GradientStop, Op, Paint, PathCommand, Shape, Stroke,
};
pub use format::{format_number, format_unit};
pub use history::History;
pub use image::{Placement, placement};
pub use path::{PathError, parse as parse_path};
pub use plan::{IssueCode, PlanIssue, build, build_with, build_with_history};
pub mod reading;
pub use reading::Readings;
pub use text::{Fonts, TextStyle};
