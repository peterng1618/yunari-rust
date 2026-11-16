//! A one-line text prompt: the smallest dialog the shell needs.
//!
//! The key handling is plain data so it can be tested without a window; the
//! rendering lives with the shell.

/// What a keystroke did to the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Submit(String),
    Cancel,
}

/// A value typed longer than this is a mistake, not an author's name.
const MAX_VALUE: usize = 512;

pub struct Prompt {
    pub title: String,
    pub hint: String,
    pub value: String,
}

impl Prompt {
    pub fn new(
        title: impl Into<String>,
        hint: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            hint: hint.into(),
            value: value.into(),
        }
    }

    /// Applies one keystroke. A key the prompt does not use leaves it pending,
    /// so a shortcut can never fire underneath an open prompt.
    pub fn key(&mut self, key: &str, key_char: Option<&str>) -> Outcome {
        match key {
            "enter" => return Outcome::Submit(self.value.trim().to_owned()),
            "escape" => return Outcome::Cancel,
            "backspace" => {
                self.value.pop();
                return Outcome::Pending;
            }
            _ => {}
        }
        if let Some(text) = key_char.filter(|text| !text.chars().any(char::is_control))
            && self.value.chars().count() < MAX_VALUE
        {
            self.value.push_str(text);
        }
        Outcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_enter_submits_the_trimmed_value() {
        let mut prompt = Prompt::new("Save as", "", "");
        for key in ["t", "h", "e", "m", "e"] {
            assert_eq!(prompt.key(key, Some(key)), Outcome::Pending);
        }
        assert_eq!(
            prompt.key("enter", None),
            Outcome::Submit("theme".to_owned())
        );
    }

    #[test]
    fn backspace_removes_the_last_character_and_escape_cancels() {
        let mut prompt = Prompt::new("Save as", "", "ab");
        assert_eq!(prompt.key("backspace", None), Outcome::Pending);
        assert_eq!(prompt.value, "a");
        assert_eq!(prompt.key("escape", None), Outcome::Cancel);
    }

    #[test]
    fn a_control_key_types_nothing() {
        let mut prompt = Prompt::new("Save as", "", "");
        assert_eq!(prompt.key("s", None), Outcome::Pending);
        assert_eq!(prompt.value, "");
        assert_eq!(prompt.key("s", Some("\u{1}")), Outcome::Pending);
        assert_eq!(prompt.value, "");
    }

    #[test]
    fn a_value_is_capped() {
        let mut prompt = Prompt::new("Save as", "", "");
        for _ in 0..MAX_VALUE + 10 {
            prompt.key("a", Some("a"));
        }
        assert_eq!(prompt.value.chars().count(), MAX_VALUE);
    }
}
