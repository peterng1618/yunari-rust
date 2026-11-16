//! Valve KeyValues text, as `libraryfolders.vdf` and `appmanifest_*.acf` are.
//!
//! Only the handful of fields this host reads pass through, so the grammar
//! lives here rather than in a dependency: quoted strings with backslash
//! escapes, nested blocks, and `//` comments.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Object(Block),
}

pub type Block = BTreeMap<String, Value>;

impl Value {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            Self::Object(_) => None,
        }
    }

    pub fn block(&self) -> Option<&Block> {
        match self {
            Self::Object(block) => Some(block),
            Self::String(_) => None,
        }
    }
}

/// The file as one block, or nothing when it is not KeyValues this build reads.
pub fn parse(text: &str) -> Option<Block> {
    let mut reader = Reader {
        chars: text.chars().collect(),
        at: 0,
    };
    reader.pairs(false)
}

struct Reader {
    chars: Vec<char>,
    at: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// Index past whitespace and comments.
    fn skip(&mut self) {
        while let Some(char) = self.peek() {
            if matches!(char, ' ' | '\t' | '\r' | '\n' | '\u{feff}') {
                self.at += 1;
                continue;
            }
            if char == '/' && self.chars.get(self.at + 1) == Some(&'/') {
                match self.chars[self.at + 2..]
                    .iter()
                    .position(|char| *char == '\n')
                {
                    Some(offset) => self.at += 2 + offset + 1,
                    None => self.at = self.chars.len(),
                }
                continue;
            }
            break;
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.peek() != Some('"') {
            return None;
        }
        self.at += 1;
        let mut value = String::new();
        while let Some(char) = self.peek() {
            match char {
                '"' => {
                    self.at += 1;
                    return Some(value);
                }
                '\\' => {
                    let next = *self.chars.get(self.at + 1)?;
                    value.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    self.at += 2;
                }
                other => {
                    value.push(other);
                    self.at += 1;
                }
            }
        }
        None
    }

    /// Pairs until the closing brace (`until_brace`) or the end of the file.
    fn pairs(&mut self, until_brace: bool) -> Option<Block> {
        let mut block = Block::new();
        loop {
            self.skip();
            match self.peek() {
                None => return (!until_brace).then_some(block),
                Some('}') => {
                    if !until_brace {
                        return None;
                    }
                    self.at += 1;
                    return Some(block);
                }
                Some(_) => {}
            }
            let key = self.string()?;
            self.skip();
            let value = if self.peek() == Some('"') {
                Value::String(self.string()?)
            } else if self.peek() == Some('{') {
                self.at += 1;
                Value::Object(self.pairs(true)?)
            } else {
                return None;
            };
            block.insert(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_reads_its_three_fields() {
        let text = r#""AppState"
{
	"appid"		"730"
	"Universe"		"1"
	"name"		"Counter-Strike 2"
	"installdir"		"Counter-Strike Global Offensive"
	"StateFlags"		"4"
}
"#;
        let block = parse(text).expect("a manifest");
        let state = block["AppState"].block().expect("AppState");
        assert_eq!(state["appid"].text(), Some("730"));
        assert_eq!(state["name"].text(), Some("Counter-Strike 2"));
        assert_eq!(
            state["installdir"].text(),
            Some("Counter-Strike Global Offensive")
        );
    }

    #[test]
    fn a_library_file_lists_each_folder() {
        let text = r##""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary" // a second drive
		"apps" { "730" "123" }
	}
}
"##;
        let block = parse(text).expect("a library file");
        let folders = block["libraryfolders"].block().expect("folders");
        assert_eq!(
            folders["0"].block().unwrap()["path"].text(),
            Some(r"C:\Program Files (x86)\Steam")
        );
        assert_eq!(
            folders["1"].block().unwrap()["path"].text(),
            Some(r"D:\SteamLibrary")
        );
    }

    #[test]
    fn a_file_that_is_not_keyvalues_is_nothing() {
        assert_eq!(parse("{\"kind\":\"json\"}"), None);
        assert_eq!(
            parse("\"a\" \"b\""),
            Some(Block::from([(
                "a".to_owned(),
                Value::String("b".to_owned())
            )]))
        );
        assert_eq!(parse("\"a\" {"), None);
    }
}
