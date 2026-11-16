//! Installing the agent surface: the stdio bridge is registered with Claude
//! Code and the theme-designer skill is copied where agent runtimes read it.
//!
//! Every write is one the author asked for by running `yunari-mcp install`, and
//! `uninstall` reverses exactly those. A config that is not JSON is refused
//! rather than rewritten: this tool never discards settings it cannot read.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub const SERVER_NAME: &str = "yunari";
pub const SKILL_NAME: &str = "yunari-theme-designer";
/// The one permission rule an install adds: this server's tools are approved,
/// nothing else.
pub const AUTO_APPROVAL: &str = "mcp__yunari";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillState {
    Current,
    Drifted,
    Missing,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub server: bool,
    pub approval: bool,
    pub skill: SkillState,
    pub config: PathBuf,
    pub settings: PathBuf,
    pub skills: Vec<PathBuf>,
    pub message: Option<String>,
}

impl Report {
    pub fn json(&self) -> Value {
        json!({
            "server": self.server,
            "approval": self.approval,
            "skill": match self.skill {
                SkillState::Current => "current",
                SkillState::Drifted => "drifted",
                SkillState::Missing => "not-installed",
            },
            "config": self.config.display().to_string(),
            "settings": self.settings.display().to_string(),
            "skills": self.skills.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "message": self.message,
        })
    }
}

/// The skill this build ships: beside the executable in a staged package, or
/// in the checkout it was built from.
fn source() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()?
        .parent()?
        .join("skills")
        .join(SKILL_NAME);
    if beside.is_dir() {
        return Some(beside);
    }
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("skills")
        .join(SKILL_NAME);
    checkout.is_dir().then_some(checkout)
}

/// Every file below a skill folder, as a relative path and its bytes.
fn files_under(directory: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    collect(directory, &mut Vec::new(), &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect(
    directory: &Path,
    relative: &mut Vec<std::ffi::OsString>,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            relative.push(entry.file_name());
            collect(&path, relative, files);
            relative.pop();
        } else if let Ok(bytes) = std::fs::read(&path) {
            let mut relative_to = PathBuf::new();
            for segment in relative.iter() {
                relative_to.push(segment);
            }
            relative_to.push(entry.file_name());
            files.push((relative_to, bytes));
        }
    }
}

/// The two skill directories agent runtimes scan: Claude Code's own, and the
/// universal one every other runtime reads.
fn skill_targets(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("skills").join(SKILL_NAME),
        home.join(".agents").join("skills").join(SKILL_NAME),
    ]
}

fn config_path(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

/// The file Claude Code reads its permission rules from. The server entry and
/// the approval live in different files, and each is written on its own.
fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

/// The profile `install` writes into: `%USERPROFILE%`, or `$HOME` elsewhere.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn read_config(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).map_err(|error| {
            format!(
                "{} is not JSON, so nothing was written: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn write_config(path: &Path, config: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    std::fs::write(&temporary, text)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| format!("{}: {error}", path.display()))
}

fn server_entry(executable: &Path) -> Value {
    json!({
        "type": "stdio",
        "command": executable.display().to_string(),
        "args": [],
    })
}

/// Reads a JSON file, hands it to `edit`, and writes the result only when the
/// edit succeeded. An unreadable or non-object file is refused unchanged.
fn edit_json(
    path: &Path,
    edit: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    let mut held = read_config(path)?;
    if !held.is_object() {
        return Err(format!(
            "{} is not a JSON object, so nothing was written.",
            path.display()
        ));
    }
    edit(&mut held)?;
    write_config(path, &held)
}

fn push_message(slot: &mut Option<String>, message: String) {
    match slot {
        Some(held) => {
            held.push('\n');
            held.push_str(&message);
        }
        None => *slot = Some(message),
    }
}

fn object_field<'a>(
    held: &'a mut Value,
    key: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, String> {
    let object = held
        .as_object_mut()
        .ok_or_else(|| "the file is not a JSON object".to_owned())?;
    let field = object.entry(key).or_insert_with(|| json!({}));
    field
        .as_object_mut()
        .ok_or_else(|| format!("\"{key}\" is not an object"))
}

/// Installs the server entry, the approval rule and the skill for one home
/// folder. Each half is attempted even when another fails, and a partial
/// result never reads as success.
pub fn install(home: &Path, executable: &Path) -> Report {
    let config = config_path(home);
    let settings_file = settings_path(home);
    let mut report = Report {
        server: false,
        approval: false,
        skill: SkillState::Missing,
        config: config.clone(),
        settings: settings_file.clone(),
        skills: skill_targets(home),
        message: None,
    };

    if let Err(message) = edit_json(&config, |held| {
        object_field(held, "mcpServers")?.insert(SERVER_NAME.to_owned(), server_entry(executable));
        Ok(())
    }) {
        push_message(&mut report.message, message);
    } else {
        report.server = true;
    }

    if let Err(message) = edit_json(&settings_file, |held| {
        let permissions = object_field(held, "permissions")?;
        let allow = permissions.entry("allow").or_insert_with(|| json!([]));
        let allow = allow
            .as_array_mut()
            .ok_or_else(|| "\"permissions.allow\" is not a list".to_owned())?;
        if !allow.iter().any(|rule| rule == AUTO_APPROVAL) {
            allow.push(Value::String(AUTO_APPROVAL.to_owned()));
        }
        Ok(())
    }) {
        push_message(&mut report.message, message);
    } else {
        report.approval = true;
    }

    match install_skill(home) {
        Ok((state, targets)) => {
            report.skill = state;
            report.skills = targets;
        }
        Err(message) => push_message(&mut report.message, message),
    }
    report
}

/// Copies the skill over both target folders. A drifted or missing folder is
/// replaced file by file, so an install never leaves a half-updated skill.
fn install_skill(home: &Path) -> Result<(SkillState, Vec<PathBuf>), String> {
    let Some(source) = source() else {
        return Ok((
            SkillState::Missing,
            vec![String::from("the skill is not in this package").into()],
        ));
    };
    let files = files_under(&source);
    if files.is_empty() {
        return Err("The skill folder is empty.".to_owned());
    }
    let targets = skill_targets(home);
    for target in &targets {
        for (relative, bytes) in &files {
            let path = target.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("{}: {error}", parent.display()))?;
            }
            std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    let state = if same_as(&targets[0], &files) {
        SkillState::Current
    } else {
        SkillState::Drifted
    };
    Ok((state, targets))
}

fn same_as(directory: &Path, files: &[(PathBuf, Vec<u8>)]) -> bool {
    files.iter().all(|(relative, bytes)| {
        std::fs::read(directory.join(relative)).is_ok_and(|held| &held == bytes)
    })
}

/// Removes the server entry, the permission rule and both skill folders.
pub fn uninstall(home: &Path) -> Report {
    let config = config_path(home);
    let settings_file = settings_path(home);
    let mut report = Report {
        server: false,
        approval: false,
        skill: SkillState::Missing,
        config: config.clone(),
        settings: settings_file.clone(),
        skills: skill_targets(home),
        message: None,
    };

    if config.exists() {
        if let Err(message) = edit_json(&config, |held| {
            if let Some(servers) = held.get_mut("mcpServers").and_then(Value::as_object_mut) {
                servers.remove(SERVER_NAME);
            }
            Ok(())
        }) {
            push_message(&mut report.message, message);
        }
    }

    if settings_file.exists() {
        if let Err(message) = edit_json(&settings_file, |held| {
            if let Some(allow) = held
                .get_mut("permissions")
                .and_then(Value::as_object_mut)
                .and_then(|permissions| permissions.get_mut("allow"))
                .and_then(Value::as_array_mut)
            {
                allow.retain(|rule| rule != AUTO_APPROVAL);
            }
            Ok(())
        }) {
            push_message(&mut report.message, message);
        }
    }

    for target in skill_targets(home) {
        if target.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
    }
    report
}

/// Whether the config holds this server.
fn server_registered(config: &Value) -> bool {
    config
        .get("mcpServers")
        .and_then(|servers| servers.get(SERVER_NAME))
        .is_some()
}

fn approved(settings: &Value) -> bool {
    settings
        .get("permissions")
        .and_then(|permissions| permissions.get("allow"))
        .and_then(Value::as_array)
        .is_some_and(|allow| allow.iter().any(|rule| rule == AUTO_APPROVAL))
}

/// What an install would find now, without writing anything.
pub fn status(home: &Path) -> Report {
    let config = config_path(home);
    let settings_file = settings_path(home);
    let held = read_config(&config).unwrap_or_else(|_| json!({}));
    let permissions = read_config(&settings_file).unwrap_or_else(|_| json!({}));
    let skill = match source() {
        Some(source) => {
            let files = files_under(&source);
            if skill_targets(home)
                .iter()
                .any(|target| same_as(target, &files))
            {
                SkillState::Current
            } else if skill_targets(home).iter().any(|target| target.is_dir()) {
                SkillState::Drifted
            } else {
                SkillState::Missing
            }
        }
        None => SkillState::Missing,
    };
    Report {
        server: server_registered(&held),
        approval: approved(&permissions),
        skill,
        config,
        settings: settings_file,
        skills: skill_targets(home),
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yunari-mcp-install-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary home");
        path
    }

    #[test]
    fn an_install_registers_the_server_and_copies_the_skill_everywhere() {
        let home = home("install");
        let executable = home.join("yunari-mcp.exe");
        std::fs::write(&executable, b"").unwrap();
        let report = install(&home, &executable);
        assert!(report.server, "{report:?}");
        assert!(report.approval, "{report:?}");
        assert_eq!(report.skill, SkillState::Current);
        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(config_path(&home)).unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"][SERVER_NAME]["command"],
            executable.display().to_string()
        );
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(settings["permissions"]["allow"][0], AUTO_APPROVAL);
        for target in skill_targets(&home) {
            assert!(target.join("SKILL.md").is_file(), "{}", target.display());
        }
        // Status agrees, and a second install is idempotent.
        let found = status(&home);
        assert!(found.server);
        assert!(found.approval);
        assert_eq!(found.skill, SkillState::Current);
        install(&home, &executable);
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(
            settings["permissions"]["allow"].as_array().unwrap().len(),
            1
        );

        uninstall(&home);
        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(config_path(&home)).unwrap()).unwrap();
        assert!(config["mcpServers"].get(SERVER_NAME).is_none());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&home)).unwrap()).unwrap();
        assert!(
            settings["permissions"]["allow"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for target in skill_targets(&home) {
            assert!(!target.exists());
        }
        assert!(!status(&home).server);
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_config_that_is_not_json_is_left_alone() {
        let home = home("invalid");
        std::fs::write(config_path(&home), b"{ not json").unwrap();
        let report = install(&home, &home.join("yunari-mcp.exe"));
        assert!(!report.server);
        assert!(report.message.is_some());
        assert_eq!(
            std::fs::read(config_path(&home)).unwrap(),
            b"{ not json".to_vec()
        );
        std::fs::remove_dir_all(&home).ok();
    }
}
