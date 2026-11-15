//! One reading: what a provider saw, as it was measured.

/// How a reading came out. An absence is its own state so that a zero can never
/// stand in for "nothing could read this".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Missing,
    Error,
}

/// What one provider saw for one semantic key.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub semantic_key: String,
    pub status: Status,
    pub value: Option<f64>,
    pub unit: Option<String>,
    /// The rendered form of a text-kind reading.
    pub text: Option<String>,
    /// An ISO 8601 instant, for keys whose vocabulary says they are instants.
    pub instant: Option<String>,
    /// Why a reading is missing or in error. Never a value.
    pub message: Option<String>,
}

impl Reading {
    pub fn number(semantic_key: impl Into<String>, value: f64, unit: impl Into<String>) -> Self {
        Self {
            semantic_key: semantic_key.into(),
            status: Status::Ok,
            value: Some(value),
            unit: Some(unit.into()),
            text: None,
            instant: None,
            message: None,
        }
    }

    pub fn text(semantic_key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            semantic_key: semantic_key.into(),
            status: Status::Ok,
            value: None,
            unit: None,
            text: Some(text.into()),
            instant: None,
            message: None,
        }
    }

    pub fn instant(semantic_key: impl Into<String>, instant: impl Into<String>) -> Self {
        Self {
            semantic_key: semantic_key.into(),
            status: Status::Ok,
            value: None,
            unit: None,
            text: None,
            instant: Some(instant.into()),
            message: None,
        }
    }

    /// A reading nothing could produce. The reason travels with it so a display
    /// can say why the gap is there.
    pub fn missing(semantic_key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            semantic_key: semantic_key.into(),
            status: Status::Missing,
            value: None,
            unit: None,
            text: None,
            instant: None,
            message: Some(message.into()),
        }
    }

    pub fn failed(semantic_key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: Status::Error,
            ..Self::missing(semantic_key, message)
        }
    }

    /// The figure to show, or `None` when there is nothing measured to show.
    pub fn figure(&self) -> Option<f64> {
        (self.status == Status::Ok).then_some(self.value).flatten()
    }
}
