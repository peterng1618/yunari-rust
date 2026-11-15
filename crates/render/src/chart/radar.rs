//! Radar chart.

use crate::chart::ChartRequest;
use crate::{Op, PlanIssue};

pub fn build(
    request: &ChartRequest<'_>,
    _fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    issues.push(PlanIssue {
        code: crate::IssueCode::UnsupportedContent,
        node_id: Some(request.node_id.to_owned()),
        detail: "This chart family is not drawn yet.".into(),
    });
    Vec::new()
}
