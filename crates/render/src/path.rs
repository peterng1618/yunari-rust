//! SVG path data becomes the display list's own path commands.
//!
//! The authored form is the one authors and icon sets already write, so the
//! reader is the only translation needed: every command becomes an absolute
//! `PathCommand`, and a command this renderer cannot draw refuses the whole
//! path rather than drawing a shape that is not the one authored.

use svgtypes::{PathParser, PathSegment};

use crate::display_list::PathCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// The text is not path data at all.
    Syntax(String),
    /// The text is path data this renderer does not draw yet.
    Unsupported(&'static str),
}

/// Read authored path data into absolute, drawable commands.
pub fn parse(data: &str) -> Result<Vec<PathCommand>, PathError> {
    let mut commands = Vec::new();
    let mut cursor = (0.0_f64, 0.0_f64);
    // The control point a smooth command reflects, and whether the command
    // before it was the kind that leaves one.
    let mut control: Option<(f64, f64)> = None;
    let mut control_is_cubic = false;

    for segment in PathParser::from(data) {
        let segment = segment.map_err(|error| PathError::Syntax(error.to_string()))?;
        match segment {
            PathSegment::MoveTo { abs, x, y } => {
                cursor = place(abs, cursor, x, y);
                commands.push(PathCommand::MoveTo(cursor.0 as f32, cursor.1 as f32));
                control = None;
            }
            PathSegment::LineTo { abs, x, y } => {
                cursor = place(abs, cursor, x, y);
                commands.push(PathCommand::LineTo(cursor.0 as f32, cursor.1 as f32));
                control = None;
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                let x = if abs { x } else { cursor.0 + x };
                cursor = (x, cursor.1);
                commands.push(PathCommand::LineTo(cursor.0 as f32, cursor.1 as f32));
                control = None;
            }
            PathSegment::VerticalLineTo { abs, y } => {
                let y = if abs { y } else { cursor.1 + y };
                cursor = (cursor.0, y);
                commands.push(PathCommand::LineTo(cursor.0 as f32, cursor.1 as f32));
                control = None;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let (first, second) = if abs {
                    ((x1, y1), (x2, y2))
                } else {
                    (
                        (cursor.0 + x1, cursor.1 + y1),
                        (cursor.0 + x2, cursor.1 + y2),
                    )
                };
                let end = place(abs, cursor, x, y);
                commands.push(PathCommand::CubicTo(
                    first.0 as f32,
                    first.1 as f32,
                    second.0 as f32,
                    second.1 as f32,
                    end.0 as f32,
                    end.1 as f32,
                ));
                control = Some(second);
                control_is_cubic = true;
                cursor = end;
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let first = match (control, control_is_cubic) {
                    (Some(control), true) => {
                        (cursor.0 * 2.0 - control.0, cursor.1 * 2.0 - control.1)
                    }
                    _ => cursor,
                };
                let second = if abs {
                    (x2, y2)
                } else {
                    (cursor.0 + x2, cursor.1 + y2)
                };
                let end = place(abs, cursor, x, y);
                commands.push(PathCommand::CubicTo(
                    first.0 as f32,
                    first.1 as f32,
                    second.0 as f32,
                    second.1 as f32,
                    end.0 as f32,
                    end.1 as f32,
                ));
                control = Some(second);
                control_is_cubic = true;
                cursor = end;
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let control = if abs {
                    (x1, y1)
                } else {
                    (cursor.0 + x1, cursor.1 + y1)
                };
                let end = place(abs, cursor, x, y);
                commands.push(PathCommand::QuadTo(
                    control.0 as f32,
                    control.1 as f32,
                    end.0 as f32,
                    end.1 as f32,
                ));
                cursor = end;
                control_is_cubic = false;
            }
            PathSegment::SmoothQuadratic { abs, x, y } => {
                let reflect = match (control, control_is_cubic) {
                    (Some(control), false) => {
                        (cursor.0 * 2.0 - control.0, cursor.1 * 2.0 - control.1)
                    }
                    _ => cursor,
                };
                let end = place(abs, cursor, x, y);
                commands.push(PathCommand::QuadTo(
                    reflect.0 as f32,
                    reflect.1 as f32,
                    end.0 as f32,
                    end.1 as f32,
                ));
                cursor = end;
            }
            PathSegment::EllipticalArc { .. } => {
                return Err(PathError::Unsupported(
                    "An elliptical arc is not drawn yet.",
                ));
            }
            PathSegment::ClosePath { .. } => {
                commands.push(PathCommand::Close);
                control = None;
            }
        }
    }
    Ok(commands)
}

fn place(abs: bool, cursor: (f64, f64), x: f64, y: f64) -> (f64, f64) {
    if abs {
        (x, y)
    } else {
        (cursor.0 + x, cursor.1 + y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_triangle_is_a_move_two_lines_and_a_close() {
        assert_eq!(
            parse("M0 0 L10 0 L10 10 Z"),
            Ok(vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
                PathCommand::Close,
            ])
        );
    }

    #[test]
    fn relative_commands_carry_the_cursor_forward() {
        assert_eq!(
            parse("m 5 5 l 10 0 l 0 10"),
            Ok(vec![
                PathCommand::MoveTo(5.0, 5.0),
                PathCommand::LineTo(15.0, 5.0),
                PathCommand::LineTo(15.0, 15.0),
            ])
        );
    }

    #[test]
    fn horizontal_and_vertical_commands_are_lines() {
        assert_eq!(
            parse("M0 0 H10 V10"),
            Ok(vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
            ])
        );
    }

    #[test]
    fn a_smooth_curve_reflects_the_previous_control_point() {
        let commands = parse("M0 0 C 10 0 10 10 20 10 S 30 20 40 10").expect("readable");
        let PathCommand::CubicTo(_, _, second_x, second_y, _, _) = commands[1] else {
            panic!("expected a cubic");
        };
        let PathCommand::CubicTo(first_x, first_y, _, _, _, _) = commands[2] else {
            panic!("expected a cubic");
        };
        assert!((first_x - (40.0 - second_x)).abs() < 1e-4);
        assert!((first_y - (20.0 - second_y)).abs() < 1e-4);
    }

    #[test]
    fn a_quadratic_keeps_its_control_point() {
        assert_eq!(
            parse("M0 0 Q 5 10 10 0"),
            Ok(vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::QuadTo(5.0, 10.0, 10.0, 0.0),
            ])
        );
    }

    #[test]
    fn an_arc_refuses_the_whole_path_rather_than_a_wrong_shape() {
        assert_eq!(
            parse("M0 0 A 5 5 0 0 1 10 0"),
            Err(PathError::Unsupported(
                "An elliptical arc is not drawn yet."
            ))
        );
    }

    #[test]
    fn text_that_is_not_path_data_is_refused() {
        assert!(matches!(parse("not a path"), Err(PathError::Syntax(_))));
    }
}
