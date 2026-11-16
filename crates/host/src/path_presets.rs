//! The custom shapes the Add pane offers: built-ins from code, the author's own
//! rows and an installed pack's rows, all merged from one settings file.
//!
//! A pack is read and every path measured before the file is touched, so a
//! refused pack leaves the stored list byte-for-byte as it was. Paint follows
//! geometry — a drawing that closes every subpath fills, one with an open
//! subpath is stroked — and a pack whose declared paint disagrees is refused
//! rather than drawn wrong.
//!
//! ponytail: the extent is measured from path endpoints and control points, not
//! from arc curvature; a preset whose only extent is an arc's bulge is refused
//! as empty. The upgrade is a real path flattener.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FILE: &str = "path-presets.json";
pub const MAX_PACK_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_COUNT: usize = 1_000;
pub const MAX_LABEL_CHARS: usize = 200;
pub const MAX_DATA_CHARS: usize = 200_000;
const FORMAT: &str = "yunari-path-preset-pack";
const VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Stroke,
    Fill,
}

impl Paint {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "stroke" => Some(Self::Stroke),
            "fill" => Some(Self::Fill),
            _ => None,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Stroke => "stroke",
            Self::Fill => "fill",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub data: String,
    pub paint: Paint,
}

impl Preset {
    pub fn json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "label": self.label,
            "data": self.data,
            "paint": self.paint.text(),
        })
    }
}

impl State {
    pub fn json(&self) -> Value {
        serde_json::json!({
            "version": VERSION,
            "presets": self.presets.iter().map(Preset::json).collect::<Vec<_>>(),
            "removedBuiltins": self.removed_builtins,
            "migrated": self.migrated,
        })
    }
}

/// The shapes every editor ships, in the order the Add pane offers them.
pub fn builtins() -> Vec<Preset> {
    [
        (
            "line",
            "Horizontal line",
            "M 0 100 L 360 100",
            Paint::Stroke,
        ),
        ("arc", "Arc", "M 200 0 A 200 200 0 0 1 0 200", Paint::Stroke),
        (
            "wedge",
            "Wedge",
            "M 0 0 L 200 0 A 200 200 0 0 1 0 200 Z",
            Paint::Fill,
        ),
    ]
    .into_iter()
    .map(|(id, label, data, paint)| Preset {
        id: id.to_owned(),
        label: label.to_owned(),
        data: data.to_owned(),
        paint,
    })
    .collect()
}

/// The presets a host serves: built-ins the author kept, then stored rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub presets: Vec<Preset>,
    pub removed_builtins: Vec<String>,
    pub migrated: bool,
}

struct Stored {
    presets: Vec<Preset>,
    removed: Vec<String>,
    migrated: bool,
}

/// The one file this store owns, beside the other settings.
pub struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    /// Built-ins the author has not removed, ahead of every stored row.
    pub fn read(&self) -> State {
        let stored = self.stored();
        let removed = &stored.removed;
        let mut presets: Vec<Preset> = builtins()
            .into_iter()
            .filter(|preset| !removed.contains(&preset.id))
            .collect();
        presets.extend(stored.presets);
        State {
            presets,
            removed_builtins: stored.removed,
            migrated: stored.migrated,
        }
    }

    pub fn add_user(&self, label: &str, data: &str) -> Result<Preset, String> {
        let label = checked_label(label)?;
        let data = checked_data(data)?;
        let paint = geometry(&data)?;
        let mut preset = None;
        self.update(|stored| {
            let mut taken: Vec<String> = builtins().into_iter().map(|row| row.id).collect();
            taken.extend(stored.presets.iter().map(|row| row.id.clone()));
            let row = Preset {
                id: fresh_user_id(&slug_of(&label), &taken),
                label,
                data,
                paint,
            };
            stored.presets.push(row.clone());
            preset = Some(row);
        })?;
        preset.ok_or_else(|| "The preset was not stored.".to_owned())
    }

    /// Installs a pack, replacing rows an earlier install of the same pack left.
    pub fn add_pack(&self, bytes: &[u8]) -> Result<(String, Vec<Preset>), String> {
        if bytes.len() > MAX_PACK_BYTES {
            return Err("That path preset pack is too large.".to_owned());
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "That path preset pack is not UTF-8.".to_owned())?;
        let value: Value = serde_json::from_str(text)
            .map_err(|_| "That path preset pack is not JSON.".to_owned())?;
        let (pack_id, rows) = read_pack(&value)?;
        let prefix = format!("pack:{pack_id}:");
        self.update(|stored| {
            stored
                .presets
                .retain(|preset| !preset.id.starts_with(&prefix));
            stored.presets.extend(rows.iter().cloned());
        })?;
        Ok((pack_id, rows))
    }

    /// Removes a stored row. A built-in is removed through [`Self::remove_builtin`].
    pub fn remove(&self, id: &str) -> Result<(), String> {
        if builtins().iter().any(|preset| preset.id == id) {
            return Ok(());
        }
        self.update(|stored| stored.presets.retain(|preset| preset.id != id))
    }

    pub fn remove_builtin(&self, id: &str) -> Result<(), String> {
        if !builtins().iter().any(|preset| preset.id == id) {
            return Ok(());
        }
        self.update(|stored| {
            if !stored.removed.iter().any(|removed| removed == id) {
                stored.removed.push(id.to_owned());
            }
        })
    }

    pub fn restore_builtins(&self) -> Result<(), String> {
        self.update(|stored| stored.removed.clear())
    }

    pub fn remove_pack(&self, pack_id: &str) -> Result<(), String> {
        let prefix = format!("pack:{pack_id}:");
        self.update(|stored| {
            stored
                .presets
                .retain(|preset| !preset.id.starts_with(&prefix));
        })
    }

    fn stored(&self) -> Stored {
        let Ok(text) = std::fs::read_to_string(self.directory.join(FILE)) else {
            return Stored {
                presets: Vec::new(),
                removed: Vec::new(),
                migrated: false,
            };
        };
        let Ok(Value::Object(file)) = serde_json::from_str::<Value>(&text) else {
            return Stored {
                presets: Vec::new(),
                removed: Vec::new(),
                migrated: false,
            };
        };
        let builtin_ids: Vec<String> = builtins().into_iter().map(|row| row.id).collect();
        let mut seen = builtin_ids;
        let presets = file
            .get("presets")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(stored_preset)
                    .filter(|preset| {
                        if seen.contains(&preset.id) {
                            return false;
                        }
                        seen.push(preset.id.clone());
                        true
                    })
                    .collect()
            })
            .unwrap_or_default();
        let removed = file
            .get("removedBuiltins")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Stored {
            presets,
            removed,
            migrated: file.get("migrated").and_then(Value::as_bool) == Some(true),
        }
    }

    fn update(&self, change: impl FnOnce(&mut Stored)) -> Result<(), String> {
        let mut stored = self.stored();
        change(&mut stored);
        self.write(&stored)
    }

    fn write(&self, stored: &Stored) -> Result<(), String> {
        let preset_row = |preset: &Preset| {
            let mut row = Map::new();
            row.insert("id".to_owned(), Value::String(preset.id.clone()));
            row.insert("label".to_owned(), Value::String(preset.label.clone()));
            row.insert("data".to_owned(), Value::String(preset.data.clone()));
            row.insert(
                "paint".to_owned(),
                Value::String(preset.paint.text().to_owned()),
            );
            Value::Object(row)
        };
        let file = serde_json::json!({
            "version": VERSION,
            "presets": stored.presets.iter().map(preset_row).collect::<Vec<_>>(),
            "removedBuiltins": stored.removed,
            "migrated": stored.migrated,
        });
        let text = serde_json::to_string_pretty(&file)
            .map_err(|error| format!("The path presets could not be written: {error}"))?;
        std::fs::create_dir_all(&self.directory)
            .map_err(|error| format!("The path presets could not be written: {error}"))?;
        let temp = self.directory.join("path-presets.json.new");
        std::fs::write(&temp, format!("{text}\n"))
            .map_err(|error| format!("The path presets could not be written: {error}"))?;
        std::fs::rename(&temp, self.directory.join(FILE))
            .map_err(|error| format!("The path presets could not be written: {error}"))
    }
}

/// One stored row, or nothing when the file's row cannot be read.
fn stored_preset(value: &Value) -> Option<Preset> {
    let row = value.as_object()?;
    let id = row.get("id")?.as_str()?.trim();
    if !valid_stored_id(id) {
        return None;
    }
    Some(Preset {
        id: id.to_owned(),
        label: checked_label(row.get("label")?.as_str()?).ok()?,
        data: checked_data(row.get("data")?.as_str()?).ok()?,
        paint: Paint::parse(row.get("paint")?.as_str()?)?,
    })
}

fn valid_stored_id(id: &str) -> bool {
    let Some(rest) = id
        .strip_prefix("user:")
        .or_else(|| id.strip_prefix("pack:"))
    else {
        return false;
    };
    let segments: Vec<&str> = rest.split(':').collect();
    (segments.len() == 1 || segments.len() == 2)
        && segments.iter().all(|segment| segment_id(segment))
}

fn checked_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!(
            "A path preset needs a name of at most {MAX_LABEL_CHARS} characters."
        ));
    }
    Ok(label.to_owned())
}

fn checked_data(data: &str) -> Result<String, String> {
    let data = data.trim();
    if data.is_empty() || data.chars().count() > MAX_DATA_CHARS {
        return Err(format!(
            "A path preset needs path data of at most {MAX_DATA_CHARS} characters."
        ));
    }
    Ok(data.to_owned())
}

fn slug_of(label: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in label.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(character);
        } else {
            separator = true;
        }
    }
    if slug.is_empty() {
        "preset".to_owned()
    } else {
        slug
    }
}

fn fresh_user_id(base: &str, taken: &[String]) -> String {
    let first = format!("user:{base}");
    if !taken.contains(&first) {
        return first;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{first}-{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// A pack's id and its rows, each with the `pack:<id>:` prefix, or the sentence
/// that refused it.
fn read_pack(value: &Value) -> Result<(String, Vec<Preset>), String> {
    let Some(file) = value.as_object() else {
        return Err("A path preset pack must be a JSON object.".to_owned());
    };
    if file.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("Unsupported path preset pack format.".to_owned());
    }
    if file.get("version").and_then(Value::as_i64) != Some(VERSION) {
        return Err("Unsupported path preset pack version.".to_owned());
    }
    let pack_id = file
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| segment_id(id))
        .ok_or_else(|| {
            "A path preset pack needs an id of 1-200 letters, digits, - or _.".to_owned()
        })?;
    checked_label(file.get("name").and_then(Value::as_str).unwrap_or_default())
        .map_err(|_| "A path preset pack needs a name of at most 200 characters.".to_owned())?;
    let Some(declared) = file.get("presets").and_then(Value::as_array) else {
        return Err("A path preset pack needs a list of presets.".to_owned());
    };
    if declared.len() > MAX_COUNT {
        return Err(format!(
            "A path preset pack may hold at most {MAX_COUNT} presets."
        ));
    }
    let mut rows = Vec::new();
    for entry in declared {
        let Some(row) = entry.as_object() else {
            return Err("A path preset must be an object.".to_owned());
        };
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| segment_id(id))
            .ok_or_else(|| {
                "A path preset needs an id of 1-200 letters, digits, - or _.".to_owned()
            })?;
        if rows
            .iter()
            .any(|preset: &Preset| preset.id.ends_with(&format!(":{id}")))
        {
            return Err(format!(
                "The path preset id \"{id}\" appears twice in the pack."
            ));
        }
        let label = checked_label(row.get("label").and_then(Value::as_str).unwrap_or_default())?;
        let data = checked_data(row.get("data").and_then(Value::as_str).unwrap_or_default())?;
        let declared_paint = row
            .get("paint")
            .and_then(Value::as_str)
            .and_then(Paint::parse)
            .ok_or_else(|| "A path preset paint must be \"stroke\" or \"fill\".".to_owned())?;
        // Geometry owns paint; a declared value that disagrees would draw a
        // filled shape as a line.
        let measured = geometry(&data)?;
        if measured != declared_paint {
            return Err(format!(
                "The path preset \"{id}\" declares {} paint, but its data is {}.",
                declared_paint.text(),
                measured.text()
            ));
        }
        rows.push(Preset {
            id: format!("pack:{pack_id}:{id}"),
            label,
            data,
            paint: measured,
        });
    }
    Ok((pack_id.to_owned(), rows))
}

fn segment_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= 200
        && id
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Whether a drawing fills or is stroked, refusing one that draws nothing or
/// cannot be read. Filling needs every subpath closed.
fn geometry(data: &str) -> Result<Paint, String> {
    let path = parse_path(data).ok_or_else(|| "That path data cannot be read.".to_owned())?;
    if path.width <= 0.0 && path.height <= 0.0 {
        return Err("That path data draws nothing.".to_owned());
    }
    Ok(if path.all_closed {
        Paint::Fill
    } else {
        Paint::Stroke
    })
}

struct Measured {
    width: f64,
    height: f64,
    all_closed: bool,
}

#[derive(PartialEq)]
enum Token {
    Command(u8),
    Number(f64),
}

fn parse_path(data: &str) -> Option<Measured> {
    let tokens = tokenize(data)?;
    let mut index = 0;
    let mut point = (0.0f64, 0.0f64);
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    let mut subpaths: Vec<bool> = Vec::new();
    let mut drew = false;
    let mut command = None;
    while index < tokens.len() {
        if let Token::Command(byte) = tokens[index] {
            command = Some(byte);
            index += 1;
        }
        let letter = command?;
        if letter == b'Z' || letter == b'z' {
            if let Some(open) = subpaths.last_mut() {
                *open = false;
            }
            drew = true;
            // SVG has no parameters after a close.
            if matches!(tokens.get(index), Some(Token::Number(_))) {
                return None;
            }
            continue;
        }
        let relative = letter.is_ascii_lowercase();
        let arity = match letter.to_ascii_uppercase() {
            b'M' | b'L' | b'T' => 2,
            b'H' | b'V' => 1,
            b'C' => 6,
            b'S' | b'Q' => 4,
            b'A' => 7,
            _ => return None,
        };
        let mut first = true;
        loop {
            if index + arity > tokens.len() {
                return None;
            }
            let mut values = [0.0f64; 7];
            for (offset, value) in values.iter_mut().take(arity).enumerate() {
                match tokens[index + offset] {
                    Token::Number(number) => *value = number,
                    Token::Command(_) => return None,
                }
            }
            index += arity;
            point = match letter.to_ascii_uppercase() {
                b'H' => (
                    if relative {
                        point.0 + values[0]
                    } else {
                        values[0]
                    },
                    point.1,
                ),
                b'V' => (
                    point.0,
                    if relative {
                        point.1 + values[0]
                    } else {
                        values[0]
                    },
                ),
                b'C' => {
                    if relative {
                        include(&mut bounds, point.0 + values[0], point.1 + values[1]);
                        include(&mut bounds, point.0 + values[2], point.1 + values[3]);
                    } else {
                        include(&mut bounds, values[0], values[1]);
                        include(&mut bounds, values[2], values[3]);
                    }
                    let end = (values[4], values[5]);
                    move_to(point, end, relative)
                }
                b'S' | b'Q' => {
                    if relative {
                        include(&mut bounds, point.0 + values[0], point.1 + values[1]);
                    } else {
                        include(&mut bounds, values[0], values[1]);
                    }
                    move_to(point, (values[2], values[3]), relative)
                }
                b'A' => move_to(point, (values[5], values[6]), relative),
                _ => move_to(point, (values[0], values[1]), relative),
            };
            include(&mut bounds, point.0, point.1);
            if letter.eq_ignore_ascii_case(&b'M') {
                if first {
                    subpaths.push(true);
                } else {
                    // Repeated moveto coordinates are a lineto.
                    drew = true;
                }
            } else {
                drew = true;
            }
            first = false;
            if !matches!(tokens.get(index), Some(Token::Number(_))) {
                break;
            }
        }
    }
    if !drew {
        return None;
    }
    let all_closed = !subpaths.is_empty() && subpaths.iter().all(|open| !open);
    Some(Measured {
        width: bounds.2 - bounds.0,
        height: bounds.3 - bounds.1,
        all_closed,
    })
}

fn move_to(from: (f64, f64), end: (f64, f64), relative: bool) -> (f64, f64) {
    if relative {
        (from.0 + end.0, from.1 + end.1)
    } else {
        end
    }
}

fn include(bounds: &mut (f64, f64, f64, f64), x: f64, y: f64) {
    bounds.0 = bounds.0.min(x);
    bounds.1 = bounds.1.min(y);
    bounds.2 = bounds.2.max(x);
    bounds.3 = bounds.3.max(y);
}

fn tokenize(data: &str) -> Option<Vec<Token>> {
    let bytes = data.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() || byte == b',' {
            index += 1;
        } else if byte.is_ascii_alphabetic() {
            if !b"MmLlHhVvCcSsQqTtAaZz".contains(&byte) {
                return None;
            }
            tokens.push(Token::Command(byte));
            index += 1;
        } else {
            let start = index;
            index += usize::from(matches!(byte, b'+' | b'-'));
            while index < bytes.len()
                && (bytes[index].is_ascii_digit()
                    || bytes[index] == b'.'
                    || matches!(bytes[index], b'e' | b'E' | b'+' | b'-'))
            {
                if matches!(bytes[index], b'e' | b'E')
                    && !bytes
                        .get(index + 1)
                        .is_some_and(|next| next.is_ascii_digit() || matches!(next, b'+' | b'-'))
                {
                    return None;
                }
                index += 1;
            }
            let text = std::str::from_utf8(&bytes[start..index]).ok()?;
            tokens.push(Token::Number(text.parse::<f64>().ok()?));
        }
    }
    match tokens.first() {
        Some(Token::Command(b'M' | b'm')) => Some(tokens),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yunari-path-presets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn geometry_decides_the_paint() {
        assert_eq!(geometry("M 0 100 L 360 100"), Ok(Paint::Stroke));
        assert_eq!(geometry("M 0 0 L 10 0 L 10 10 L 0 10 Z"), Ok(Paint::Fill));
        assert_eq!(geometry("M 0 0 L 10 0 Z M 20 0 L 30 0"), Ok(Paint::Stroke));
        assert_eq!(geometry("  M 0 0 L 10 10 Z  "), Ok(Paint::Fill));
        // Nothing drawn, or nothing readable, is a sentence not a shape.
        for data in ["", "M 0 0", "M 0 0 L", "not a path"] {
            assert!(geometry(data).is_err(), "{data}");
        }
    }

    #[test]
    fn a_pack_is_read_whole_before_anything_is_written() {
        let directory = directory();
        let store = Store::at(&directory);
        let pack = serde_json::json!({
            "format": FORMAT,
            "version": 1,
            "id": "space",
            "name": "Space shapes",
            "presets": [
                {"id": "orbit", "label": "Orbit", "data": "M 0 0 L 10 10 Z", "paint": "fill"}
            ]
        });
        let (pack_id, rows) = store.add_pack(pack.to_string().as_bytes()).unwrap();
        assert_eq!(pack_id, "space");
        assert_eq!(rows[0].id, "pack:space:orbit");
        assert!(
            store
                .read()
                .presets
                .iter()
                .any(|row| row.id == "pack:space:orbit")
        );

        // A declared paint geometry contradicts refuses the whole pack.
        let wrong = serde_json::json!({
            "format": FORMAT, "version": 1, "id": "bad", "name": "Bad",
            "presets": [{"id": "line", "label": "Line", "data": "M 0 0 L 10 10", "paint": "fill"}]
        });
        assert!(store.add_pack(wrong.to_string().as_bytes()).is_err());
        assert!(
            !store
                .read()
                .presets
                .iter()
                .any(|row| row.id.starts_with("pack:bad:"))
        );

        // Removing the pack takes only its own rows.
        store.remove_pack("space").unwrap();
        assert!(
            !store
                .read()
                .presets
                .iter()
                .any(|row| row.id.starts_with("pack:space:"))
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_user_row_is_minted_deduplicated_and_kept_across_a_reload() {
        let directory = directory();
        let store = Store::at(&directory);
        let first = store.add_user("My arc", "M 0 0 L 10 10").unwrap();
        let second = store.add_user("my  arc!", "M 0 0 L 10 20").unwrap();
        assert_eq!(first.id, "user:my-arc");
        assert_eq!(second.id, "user:my-arc-2");
        assert_eq!(first.paint, Paint::Stroke);
        // The merged view is built-ins then stored rows, read back from disk.
        let reloaded = Store::at(&directory).read();
        assert_eq!(reloaded.presets[0].id, "line");
        assert!(reloaded.presets.iter().any(|row| row.id == "user:my-arc-2"));
        assert!(store.add_user("", "M 0 0 L 1 1").is_err());
        assert!(store.add_user("Nothing", "M 0 0").is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_builtin_is_removed_by_record_and_restored_by_list() {
        let directory = directory();
        let store = Store::at(&directory);
        store.remove_builtin("arc").unwrap();
        let state = store.read();
        assert!(!state.presets.iter().any(|row| row.id == "arc"));
        assert_eq!(state.removed_builtins, vec!["arc".to_owned()]);
        // Removing a built-in row itself changes nothing: it lives in code.
        store.remove("arc").unwrap();
        assert_eq!(store.read().removed_builtins, vec!["arc".to_owned()]);
        store.restore_builtins().unwrap();
        assert!(store.read().presets.iter().any(|row| row.id == "arc"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
