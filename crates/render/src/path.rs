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
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let end = place(abs, cursor, x, y);
                for curve in arc_to_cubics(cursor, rx, ry, x_axis_rotation, large_arc, sweep, end) {
                    commands.push(curve);
                }
                cursor = end;
                control = None;
            }
            PathSegment::ClosePath { .. } => {
                commands.push(PathCommand::Close);
                control = None;
            }
        }
    }
    Ok(commands)
}

/// An SVG elliptical arc as up to four cubic curves, the form the display
/// list draws. A radius of zero is the straight line the spec says it is.
fn arc_to_cubics(
    from: (f64, f64),
    rx: f64,
    ry: f64,
    rotation_degrees: f64,
    large_arc: bool,
    sweep: bool,
    to: (f64, f64),
) -> Vec<PathCommand> {
    let (rx, ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || from == to {
        return vec![PathCommand::LineTo(to.0 as f32, to.1 as f32)];
    }
    let phi = rotation_degrees.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let x1 = cos_phi * dx + sin_phi * dy;
    let y1 = -sin_phi * dx + cos_phi * dy;
    // The radii the authored numbers cannot reach are scaled up until they
    // can, as the spec requires.
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    let (rx, ry) = if lambda > 1.0 {
        let scale = lambda.sqrt();
        (rx * scale, ry * scale)
    } else {
        (rx, ry)
    };
    let numerator = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coefficient = if denominator == 0.0 {
        0.0
    } else {
        (numerator / denominator).sqrt()
    };
    if large_arc == sweep {
        coefficient = -coefficient;
    }
    let cx1 = coefficient * rx * y1 / ry;
    let cy1 = -coefficient * ry * x1 / rx;
    let center = (
        cos_phi * cx1 - sin_phi * cy1 + (from.0 + to.0) / 2.0,
        sin_phi * cx1 + cos_phi * cy1 + (from.1 + to.1) / 2.0,
    );
    let start_angle = ((y1 - cy1) / ry).atan2((x1 - cx1) / rx);
    let end_angle = ((-y1 - cy1) / ry).atan2((-x1 - cx1) / rx);
    let mut delta = end_angle - start_angle;
    if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    } else if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    }
    let steps = (delta.abs() / (std::f64::consts::FRAC_PI_2))
        .ceil()
        .max(1.0);
    let step = delta / steps;
    let alpha = 4.0 / 3.0 * (step / 4.0).tan();
    let mut curves = Vec::new();
    let mut angle = start_angle;
    let point = |angle: f64| {
        (
            center.0 + rx * angle.cos() * cos_phi - ry * angle.sin() * sin_phi,
            center.1 + rx * angle.cos() * sin_phi + ry * angle.sin() * cos_phi,
        )
    };
    for _ in 0..(steps as usize) {
        let next = angle + step;
        let (start, start_tangent) = (
            point(angle),
            (
                -rx * angle.sin() * cos_phi - ry * angle.cos() * sin_phi,
                -rx * angle.sin() * sin_phi + ry * angle.cos() * cos_phi,
            ),
        );
        let (end, end_tangent) = (
            point(next),
            (
                -rx * next.sin() * cos_phi - ry * next.cos() * sin_phi,
                -rx * next.sin() * sin_phi + ry * next.cos() * cos_phi,
            ),
        );
        curves.push(PathCommand::CubicTo(
            (start.0 + alpha * start_tangent.0) as f32,
            (start.1 + alpha * start_tangent.1) as f32,
            (end.0 - alpha * end_tangent.0) as f32,
            (end.1 - alpha * end_tangent.1) as f32,
            end.0 as f32,
            end.1 as f32,
        ));
        angle = next;
    }
    curves
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
    fn an_arc_is_drawn_as_cubics_that_end_where_it_says() {
        let commands = parse("M0 0 A 5 5 0 0 1 10 0").expect("an arc");
        assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
        assert!(matches!(commands[1], PathCommand::CubicTo(..)));
        let PathCommand::CubicTo(_, _, _, _, x, y) = *commands.last().expect("a curve") else {
            panic!("a cubic ends the arc");
        };
        assert!((x - 10.0).abs() < 1e-4 && y.abs() < 1e-4, "({x}, {y})");
    }

    #[test]
    fn text_that_is_not_path_data_is_refused() {
        assert!(matches!(parse("not a path"), Err(PathError::Syntax(_))));
    }
}
