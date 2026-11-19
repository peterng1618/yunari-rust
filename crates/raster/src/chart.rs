//! Chart ops painted natively.
//!
//! Geometry never originates here: `yunari_render::chart` owns every family's
//! formulas, so this module builds a `ChartRequest` from the op and hands the
//! family's `build` the same readings, samples and palette a node owns. The
//! resulting display-list ops are painted by the ordinary backend paths; only
//! the missing readings' issue reasons are collected here.

use std::collections::HashMap;

use yunari_model::document::ThemeDocument;
use yunari_render::chart::{ChartRequest, bar, gauge, line, pie, polar_bar, radar};
use yunari_render::{Affine, ChartFamily, Fonts, IssueCode, Op, PlanIssue};

/// One frame's readings and recorded window samples, by semantic key.
pub struct ChartSamples<'a> {
    pub readings: &'a HashMap<String, Option<f64>>,
    pub series: &'a HashMap<String, Vec<(u64, f64)>>,
}

impl ChartSamples<'_> {
    /// An empty frame: a chart with no documented readings draws the structure
    /// its settings describe, never a zero invented from a missing sample.
    pub fn empty() -> Self {
        static READINGS: std::sync::LazyLock<HashMap<String, Option<f64>>> =
            std::sync::LazyLock::new(HashMap::new);
        static SERIES: std::sync::LazyLock<HashMap<String, Vec<(u64, f64)>>> =
            std::sync::LazyLock::new(HashMap::new);
        Self {
            readings: &READINGS,
            series: &SERIES,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn expand(
    family: ChartFamily,
    settings: &serde_json::Value,
    bindings: &[String],
    width: f32,
    height: f32,
    transform: Affine,
    document: Option<&ThemeDocument>,
    samples: &ChartSamples<'_>,
    fonts: Option<&mut Fonts>,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    // A settings bag is an object by document contract. Anything else is a
    // malformed chart: refuse it rather than reading a default out of an array
    // or a string and painting marks the author never described.
    let Some(settings) = settings.as_object() else {
        issues.push(PlanIssue {
            code: IssueCode::UnsupportedContent,
            node_id: Some("chart".to_owned()),
            detail: "Chart settings are not an object.".into(),
        });
        return Vec::new();
    };

    // A missing reading is an absence, not a zero. The backend has no node id
    // here, so the semantic key names the reason.
    let readings: Vec<Option<f64>> = bindings
        .iter()
        .map(|key| match samples.readings.get(key).copied().flatten() {
            Some(value) if value.is_finite() => Some(value),
            _ => {
                issues.push(PlanIssue {
                    code: IssueCode::MissingReading,
                    node_id: Some(key.clone()),
                    detail: format!("No reading for {key}."),
                });
                None
            }
        })
        .collect();
    let series: Vec<Vec<(u64, f64)>> = bindings
        .iter()
        .map(|key| {
            samples
                .series
                .get(key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, value)| value.is_finite())
                .collect()
        })
        .collect();

    let request = ChartRequest {
        document,
        node_id: "chart",
        width,
        height,
        readings: &readings,
        series: &series,
        opacity: 1.0,
        settings,
    };
    // A backend without fonts still draws every family's geometry; only the
    // axis labels need shaping, and a missing font database leaves them out.
    let mut owned_fonts;
    let fonts = match fonts {
        Some(fonts) => fonts,
        None => {
            owned_fonts = Fonts::new();
            &mut owned_fonts
        }
    };
    let drawn = match family {
        ChartFamily::Gauge => gauge::build(&request, fonts, issues),
        ChartFamily::Line => line::build(&request, fonts, issues),
        ChartFamily::Bar => bar::build(&request, fonts, issues),
        ChartFamily::Pie => pie::build(&request, fonts, issues),
        ChartFamily::Radar => radar::build(&request, fonts, issues),
        ChartFamily::PolarBar => polar_bar::build(&request, fonts, issues),
    };
    drawn
        .into_iter()
        .map(|mut op| {
            op.transform = transform.then(op.transform);
            op
        })
        .collect()
}
