//! A chart display-list op carries its family, raw settings and bindings, and
//! stays a still image for the video scheduler.

use yunari_render::{Affine, ChartFamily, DisplayList, Op, Paint, Shape, Stroke};

fn chart_op(family: ChartFamily) -> Op {
    Op {
        transform: Affine::IDENTITY,
        shape: Shape::Chart {
            family,
            settings: serde_json::json!({ "min": 0, "max": 100 }),
            bindings: vec!["cpu".to_owned(), "gpu".to_owned()],
        },
        fill: Some(Paint::Solid(yunari_render::Color::rgb(20, 30, 40))),
        stroke: Some(Stroke {
            paint: Paint::Solid(yunari_render::Color::rgb(200, 210, 220)),
            width: 2.0,
        }),
    }
}

#[test]
fn all_chart_families_round_trip_through_json_without_video() {
    let families = [
        ChartFamily::Gauge,
        ChartFamily::Line,
        ChartFamily::Bar,
        ChartFamily::Pie,
        ChartFamily::Radar,
        ChartFamily::PolarBar,
    ];
    let list = DisplayList {
        width: 320.0,
        height: 180.0,
        background: None,
        backdrop: None,
        ops: families.iter().copied().map(chart_op).collect(),
    };

    assert!(!list.has_video(), "a chart is not a playing clip");

    let json = serde_json::to_string(&list).expect("list serializes");
    assert_eq!(
        json.matches("\"Chart\"").count(),
        6,
        "every chart op names its shape: {json}"
    );
    let back: serde_json::Value = serde_json::from_str(&json).expect("list JSON parses");
    let families_in_json = back["ops"]
        .as_array()
        .expect("ops")
        .iter()
        .map(|op| op["shape"]["Chart"]["family"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        families_in_json,
        ["gauge", "line", "bar", "pie", "radar", "polarBar"],
        "families use the six authored names"
    );
}
