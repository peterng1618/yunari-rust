//! What a value run reads once readings exist.
//!
//! A reading stays as measured until here: the run's binding applies its own
//! scale and offset, the consumer's unit preferences convert the figure, and
//! the author's precision and unit display decide how it is written. A gap is
//! the product's em dash with a reason, never a zero.

use std::collections::HashMap;

use yunari_locale::Locale;
use yunari_model::document::{Binding, Node, NodeKind, TextRun, ThemeDocument, UnitDisplay};
use yunari_providers::sample::{Reading, Status};
use yunari_providers::units::{self, Overrides, UnitPreferences};
use yunari_providers::vocabulary;

use crate::format::{format_number, format_unit};
use crate::instant;
use crate::plan::{IssueCode, MISSING_VALUE_TEXT, PlanIssue};

/// Every reading this frame has, by semantic key.
#[derive(Debug, Clone, Default)]
pub struct Readings {
    by_key: HashMap<String, Reading>,
}

impl Readings {
    pub fn new(readings: impl IntoIterator<Item = Reading>) -> Self {
        Self {
            by_key: readings
                .into_iter()
                .map(|reading| (reading.semantic_key.clone(), reading))
                .collect(),
        }
    }

    pub fn get(&self, semantic_key: &str) -> Option<&Reading> {
        self.by_key.get(semantic_key)
    }

    /// Every reading this frame holds, by key. Callers that keep a series need
    /// all of them, not just the ones a node asked about.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Reading)> {
        self.by_key
            .iter()
            .map(|(key, reading)| (key.as_str(), reading))
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// The text one value run shows.
pub fn value_text(
    document: &ThemeDocument,
    node: &Node,
    run: &TextRun,
    readings: &Readings,
    preferences: &UnitPreferences,
    issues: &mut Vec<PlanIssue>,
) -> String {
    let TextRun::Value {
        binding_id,
        precision,
        unit_display,
        ..
    } = run
    else {
        return String::new();
    };
    let Some(binding) = binding_of(document, node, binding_id) else {
        issues.push(PlanIssue {
            code: IssueCode::UnmappedKey,
            node_id: Some(node.id.clone()),
            detail: format!(
                "Text run references binding {binding_id}, which this node does not declare."
            ),
        });
        return MISSING_VALUE_TEXT.to_owned();
    };
    let Some(reading) = readings.get(&binding.semantic_key) else {
        issues.push(PlanIssue {
            code: IssueCode::MissingReading,
            node_id: Some(node.id.clone()),
            detail: format!("Nothing has read {}.", binding.semantic_key),
        });
        return MISSING_VALUE_TEXT.to_owned();
    };
    if reading.status != Status::Ok {
        issues.push(PlanIssue {
            code: IssueCode::MissingReading,
            node_id: Some(node.id.clone()),
            detail: match &reading.message {
                Some(message) => format!(
                    "{} reads {MISSING_VALUE_TEXT}: {message}",
                    binding.semantic_key
                ),
                None => format!("{} has no reading.", binding.semantic_key),
            },
        });
        return MISSING_VALUE_TEXT.to_owned();
    }
    let authored_precision = precision.or(binding.precision);
    let display = unit_display
        .or(binding.unit_display)
        .unwrap_or(UnitDisplay::Short);

    if let Some(value) = reading.value.filter(|value| value.is_finite()) {
        // The author's scale and offset act on the measurement, once; the
        // consumer's unit preferences then convert what is displayed.
        let scaled = value * binding.scale.unwrap_or(1.0) + binding.offset.unwrap_or(0.0);
        let overrides = Overrides {
            unit: binding.unit.as_deref(),
            auto_scale: binding.auto_scale.unwrap_or(false),
        };
        let converted = units::convert(
            &binding.semantic_key,
            scaled,
            reading.unit.as_deref(),
            preferences,
            overrides,
        );
        if converted.unsupported {
            issues.push(PlanIssue {
                code: IssueCode::UnsupportedUnit,
                node_id: Some(node.id.clone()),
                detail: format!(
                    "Unit {:?} is not displayable for {}; showing the measured form.",
                    binding.unit.as_deref().unwrap_or_default(),
                    binding.semantic_key
                ),
            });
        }
        let precision = authored_precision.or(converted.precision);
        // The rounding is the source's; the digits and separator are the
        // theme's language, as `Intl.NumberFormat` writes them there.
        let locale = Locale::of(document.metadata.theme_language.as_deref());
        let text = locale.number(&format_number(converted.value, precision, '.'));
        return text + &format_unit(converted.unit, display, None);
    }

    let text = if let Some(instant_value) = reading.instant.as_deref() {
        format_text(document, &binding.semantic_key, binding, instant_value)
    } else if let Some(text) = reading.text.as_deref() {
        format_text(document, &binding.semantic_key, binding, text)
    } else {
        issues.push(PlanIssue {
            code: IssueCode::MissingReading,
            node_id: Some(node.id.clone()),
            detail: format!(
                "{} carries neither a figure nor text.",
                binding.semantic_key
            ),
        });
        return MISSING_VALUE_TEXT.to_owned();
    };
    text + &format_unit(reading.unit.as_deref(), display, None)
}

fn binding_of<'a>(
    document: &'a ThemeDocument,
    node: &Node,
    binding_id: &str,
) -> Option<&'a Binding> {
    document
        .bindings
        .get(&node.id)?
        .iter()
        .find(|binding| binding.id == binding_id)
}

/// An instant is written with the author's format; any other text is shown
/// exactly as the provider sent it. A reading no formatter can read is shown
/// raw rather than blanked.
fn format_text(
    document: &ThemeDocument,
    semantic_key: &str,
    binding: &Binding,
    value: &str,
) -> String {
    let declared = vocabulary::describe(semantic_key).and_then(|key| key.instant);
    let Some(declared) = declared else {
        return value.to_owned();
    };
    let format = binding.format.as_deref().unwrap_or(declared.default_format);
    instant::format_instant(
        value,
        format,
        binding.time_zone.as_deref(),
        document.metadata.theme_language.as_deref(),
    )
    .unwrap_or_else(|| value.to_owned())
}

/// The keys a document's value runs need, so a caller can ask for exactly them.
pub fn wanted_keys(document: &ThemeDocument) -> Vec<String> {
    let mut keys = Vec::new();
    for node in &document.nodes {
        let NodeKind::Text { content } = &node.kind else {
            continue;
        };
        let Some(bindings) = document.bindings.get(&node.id) else {
            continue;
        };
        for run in &content.runs {
            let TextRun::Value { binding_id, .. } = run else {
                continue;
            };
            if let Some(binding) = bindings.iter().find(|binding| &binding.id == binding_id)
                && !keys.contains(&binding.semantic_key)
            {
                keys.push(binding.semantic_key.clone());
            }
        }
    }
    keys
}
