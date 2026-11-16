//! Installed games, from every launcher this PC has.
//!
//! Each reader answers one launcher's own record of an install: Epic's
//! manifests, GOG's registry keys, itch's app folders and Xbox's packages. A
//! launcher that is absent, or a record that does not name a game, is skipped;
//! none of them fails the catalogue.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::steam;

/// Xbox titles and GDK Appx packages share this config file name.
pub const GAME_CONFIG_FILE: &str = "MicrosoftGame.config";

/// A GOG install registers itself under this key.
const GOG_SCRIPT: &str = "$games = Get-ItemProperty -Path 'HKLM:\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\*' -ErrorAction SilentlyContinue; ConvertTo-Json -InputObject @($games) -Compress";

/// Every package, with the config probe in the same call.
const APPX_SCRIPT: &str = "$packages = Get-AppxPackage -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ Name = $_.Name; PackageFullName = $_.PackageFullName; InstallLocation = $_.InstallLocation; IsFramework = $_.IsFramework; HasConfig = [bool]($_.InstallLocation -and (Test-Path -LiteralPath (Join-Path $_.InstallLocation 'MicrosoftGame.config'))) } }; ConvertTo-Json -InputObject @($packages) -Compress";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Steam,
    Epic,
    Gog,
    Itch,
    Xbox,
}

/// One game this PC has installed, however its launcher records it.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub source: Source,
    pub name: String,
    /// Every executable below this folder belongs to the game.
    pub install_dir: PathBuf,
    /// Launchers whose install folder holds more than one program name the one
    /// they start.
    pub executables: Vec<PathBuf>,
    /// Steam's app id, which is also what its cached cover is filed under.
    pub steam_app_id: Option<String>,
}

impl Game {
    /// A Steam app's own record, without the executables Steam never writes.
    pub fn from_steam(app: steam::SteamApp) -> Self {
        Self {
            source: Source::Steam,
            name: app.name,
            install_dir: app.install_dir,
            executables: Vec::new(),
            steam_app_id: Some(app.app_id),
        }
    }
}

/// The game whose install folder holds `executable`, or that names it.
pub fn app_for<'a>(games: &'a [Game], executable: &Path) -> Option<&'a Game> {
    let wanted = comparable(executable);
    let wanted = wanted.trim_end_matches('/').to_owned();
    games.iter().find(|game| {
        let named = game
            .executables
            .iter()
            .any(|candidate| comparable(candidate).trim_end_matches('/') == wanted);
        if named {
            return true;
        }
        if game.install_dir.as_os_str().is_empty() {
            return false;
        }
        let install = comparable(&game.install_dir);
        comparable(executable).starts_with(&install)
    })
}

/// Lower-case, `/`-separated, with a trailing `/` so a sibling folder whose
/// name merely starts the same is not a match.
fn comparable(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if !text.ends_with('/') {
        text.push('/');
    }
    text
}

/// Every game any launcher on this PC knows about.
pub fn read_all() -> Vec<Game> {
    let mut games: Vec<Game> = steam::root()
        .as_deref()
        .map(steam::apps)
        .unwrap_or_default()
        .into_iter()
        .map(Game::from_steam)
        .collect();
    if let Some(program_data) = std::env::var_os("ProgramData") {
        let dir = Path::new(&program_data)
            .join("Epic")
            .join("EpicGamesLauncher")
            .join("Data")
            .join("Manifests");
        games.extend(epic_games(&dir));
    }
    if let Ok(answer) = crate::winps::run(GOG_SCRIPT) {
        games.extend(gog_games(&answer));
    }
    if let Some(app_data) = std::env::var_os("APPDATA") {
        games.extend(itch_games(&Path::new(&app_data).join("itch").join("apps")));
    }
    if let Ok(answer) = crate::winps::run(APPX_SCRIPT) {
        games.extend(xbox_games(&answer, |dir| {
            std::fs::read_to_string(dir.join(GAME_CONFIG_FILE)).ok()
        }));
    }
    games
}

/// Epic's launcher writes one `.item` per install.
pub fn epic_games(manifests: &Path) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(manifests) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("item"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(Value::Object(record)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if record.get("bIsIncompleteInstall").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let text_field = |key: &str| text_of(&record, key);
        // A DLC names its parent; its own folder is not a game.
        if let (Some(app), Some(main)) = (text_field("AppName"), text_field("MainGameAppName"))
            && !app.eq_ignore_ascii_case(&main)
        {
            continue;
        }
        let (Some(name), Some(install), Some(executable)) = (
            text_field("DisplayName"),
            text_field("InstallLocation"),
            text_field("LaunchExecutable"),
        ) else {
            continue;
        };
        let install_dir = PathBuf::from(install);
        let executable = PathBuf::from(executable);
        let executable = if executable.is_absolute() {
            executable
        } else {
            install_dir.join(executable)
        };
        games.push(Game {
            source: Source::Epic,
            name,
            install_dir,
            executables: vec![executable],
            steam_app_id: None,
        });
    }
    games
}

/// GOG's launcher keeps one registry key per install; `dependsOn` names a DLC.
pub fn gog_games(answer: &str) -> Vec<Game> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(answer) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries {
        let Value::Object(record) = entry else {
            continue;
        };
        if record.contains_key("dependsOn") {
            continue;
        }
        let (Some(name), Some(install)) = (text_of(&record, "gameName"), text_of(&record, "path"))
        else {
            continue;
        };
        let install_dir = PathBuf::from(install);
        let executables = text_of(&record, "exe")
            .map(|exe| {
                let exe = PathBuf::from(exe);
                if exe.is_absolute() {
                    exe
                } else {
                    install_dir.join(exe)
                }
            })
            .into_iter()
            .collect();
        games.push(Game {
            source: Source::Gog,
            name,
            install_dir,
            executables,
            steam_app_id: None,
        });
    }
    games
}

/// itch manages one child directory per app, each with an optional manifest.
pub fn itch_games(root: &Path) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let directory = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let manifest = std::fs::read_to_string(directory.join(".itch.toml")).ok();
        let mut executables = Vec::new();
        let mut label = name;
        if let Some(manifest) = &manifest {
            let parsed = itch_manifest(manifest);
            if let Some(declared) = parsed.name {
                label = declared;
            }
            if let Some(exe) = parsed.executable {
                let exe = PathBuf::from(exe);
                executables.push(if exe.is_absolute() {
                    exe
                } else {
                    directory.join(exe)
                });
            }
        }
        if executables.is_empty() {
            if let Ok(files) = std::fs::read_dir(&directory) {
                for file in files.flatten() {
                    let path = file.path();
                    if path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                    {
                        executables.push(path);
                    }
                }
            }
            executables.sort();
        }
        if executables.is_empty() {
            continue;
        }
        games.push(Game {
            source: Source::Itch,
            name: label,
            install_dir: directory,
            executables,
            steam_app_id: None,
        });
    }
    games
}

struct ItchManifest {
    name: Option<String>,
    executable: Option<String>,
}

/// `.itch.toml`, read for the three fields this build uses. Any other line is
/// ignored; a TOML dependency is not warranted for a file this small.
fn itch_manifest(text: &str) -> ItchManifest {
    let mut table = String::new();
    let mut name = None;
    let mut exe = None;
    let mut action_path = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            table = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            if table.starts_with('[') {
                table = table
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches('"').trim().replace("\\\\", "\\");
        if value.is_empty() {
            continue;
        }
        if table.is_empty() || table == "app" {
            match key.as_str() {
                "name" => name.get_or_insert(value),
                "exe" => exe.get_or_insert(value),
                _ => continue,
            };
        } else if table == "actions" && key == "path" {
            action_path.get_or_insert(value);
        }
    }
    ItchManifest {
        name,
        executable: exe.or(action_path),
    }
}

/// Microsoft Store and Game Pass packages: GDK titles only, recognised by
/// their `MicrosoftGame.config`.
pub fn xbox_games(answer: &str, read_config: impl Fn(&Path) -> Option<String>) -> Vec<Game> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(answer) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries {
        let Value::Object(record) = entry else {
            continue;
        };
        if is_true(record.get("IsFramework")) || !is_true(record.get("HasConfig")) {
            continue;
        }
        let (Some(name), Some(install)) = (
            text_of(&record, "Name"),
            text_of(&record, "InstallLocation"),
        ) else {
            continue;
        };
        let install_dir = PathBuf::from(install);
        let Some(config) = read_config(&install_dir) else {
            continue;
        };
        let Some(executable) = game_executable(&config) else {
            continue;
        };
        let executable = PathBuf::from(executable);
        let executable = if executable.is_absolute() {
            executable
        } else {
            install_dir.join(executable)
        };
        games.push(Game {
            source: Source::Xbox,
            name,
            install_dir,
            executables: vec![executable],
            steam_app_id: None,
        });
    }
    games
}

/// `MicrosoftGame.config` is XML; the executable is taken without a parser.
/// Comments are dropped first so a commented-out entry names nothing.
pub fn game_executable(text: &str) -> Option<String> {
    let mut text = text.to_owned();
    while let (Some(start), Some(end)) = (text.find("<!--"), text.find("-->")) {
        if end < start {
            break;
        }
        text.replace_range(start..end + 3, "");
    }
    let lower = text.to_ascii_lowercase();
    // `<ExecutableList>` also starts with `<Executable`, so only a tag that
    // ends at the tag name counts.
    let mut from = 0;
    let start = loop {
        let at = lower[from..].find("<executable")? + from;
        let after = lower[at + "<executable".len()..].chars().next();
        if matches!(after, None | Some('>' | '/')) || after.is_some_and(char::is_whitespace) {
            break at;
        }
        from = at + "<executable".len();
    };
    let tag = &text[start..];
    let end = tag.find('>')?;
    let heading = &tag[..end];
    let name_at = heading.to_ascii_lowercase().find("name")?;
    let rest = &heading[name_at + "name".len()..];
    let equals = rest.find('=')?;
    let value = rest[equals + 1..].trim_start();
    let value = value.trim_start_matches('"');
    let close = value.find('"')?;
    let value = value[..close].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn text_of(record: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    let value = record.get(key)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// PowerShell booleans arrive as JSON booleans or as the string `"True"`.
fn is_true(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-catalogue-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn epic_manifests_name_games_and_skip_dlc_and_partial_installs() {
        let dir = temp_dir("epic");
        std::fs::write(
            dir.join("game.item"),
            r#"{"AppName":"Fortnite","MainGameAppName":"Fortnite","DisplayName":"Fortnite",
                "InstallLocation":"D:\\Epic\\Fortnite","LaunchExecutable":"Fortnite\\x.exe"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("dlc.item"),
            r#"{"AppName":"FortniteExtra","MainGameAppName":"Fortnite","DisplayName":"Extra",
                "InstallLocation":"D:\\Epic\\Extra","LaunchExecutable":"x.exe"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("partial.item"),
            r#"{"bIsIncompleteInstall":true,"DisplayName":"Half",
                "InstallLocation":"D:\\Epic\\Half","LaunchExecutable":"x.exe"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "not a manifest").unwrap();
        let games = epic_games(&dir);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Fortnite");
        assert_eq!(games[0].source, Source::Epic);
        assert_eq!(
            games[0].executables,
            [PathBuf::from(r"D:\Epic\Fortnite").join(r"Fortnite\x.exe")]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gog_registry_entries_skip_dlc_and_keep_their_own_executable() {
        let games = gog_games(
            r#"[{"gameName":"The Witcher 3","path":"D:\\GOG\\Witcher","exe":"bin\\witcher3.exe"},
                {"gameName":"Hearts of Stone","path":"D:\\GOG\\Witcher\\dlc","dependsOn":"The Witcher 3"},
                {"path":"D:\\GOG\\Nameless"}]"#,
        );
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "The Witcher 3");
        assert_eq!(
            games[0].executables,
            [PathBuf::from(r"D:\GOG\Witcher\bin\witcher3.exe")]
        );
    }

    #[test]
    fn an_itch_app_uses_its_manifest_or_its_own_executables() {
        let dir = temp_dir("itch");
        let declared = dir.join("declared");
        std::fs::create_dir_all(&declared).unwrap();
        std::fs::write(
            declared.join(".itch.toml"),
            "name = \"Blue Sky\"\nexe = \"game.exe\"\n[[actions]]\npath = \"other.exe\"\n",
        )
        .unwrap();
        std::fs::write(declared.join("game.exe"), "x").unwrap();
        let bare = dir.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("Launch.EXE"), "x").unwrap();
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();

        let games = itch_games(&dir);
        assert_eq!(games.len(), 2);
        let named = games.iter().find(|game| game.name == "Blue Sky").unwrap();
        assert_eq!(named.executables, [declared.join("game.exe")]);
        let bare_game = games.iter().find(|game| game.name == "bare").unwrap();
        assert_eq!(bare_game.executables, [bare.join("Launch.EXE")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn xbox_packages_need_a_gdk_config_and_its_executable() {
        let games = xbox_games(
            r#"[{"Name":"Microsoft.ForzaHorizon5","InstallLocation":"C:\\Xbox\\Forza",
                 "IsFramework":false,"HasConfig":true},
                {"Name":"Microsoft.Framework","InstallLocation":"C:\\Xbox\\Fw",
                 "IsFramework":true,"HasConfig":true},
                {"Name":"Microsoft.StoreApp","InstallLocation":"C:\\Xbox\\App",
                 "IsFramework":false,"HasConfig":false}]"#,
            |dir| {
                (dir.ends_with("Forza")).then(|| {
                    "<!-- <Executable Name=\"old.exe\"/> --><ExecutableList><Executable Name=\"Forza.exe\"/></ExecutableList>".to_owned()
                })
            },
        );
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Microsoft.ForzaHorizon5");
        assert_eq!(
            games[0].executables,
            [PathBuf::from(r"C:\Xbox\Forza\Forza.exe")]
        );
    }

    #[test]
    fn a_named_executable_matches_even_outside_its_install_folder() {
        let game = Game {
            source: Source::Epic,
            name: "Fortnite".to_owned(),
            install_dir: PathBuf::from(r"D:\Epic\Fortnite"),
            executables: vec![PathBuf::from(r"D:\Elsewhere\FortniteLauncher.exe")],
            steam_app_id: None,
        };
        let games = vec![game];
        assert_eq!(
            app_for(&games, Path::new(r"d:\elsewhere\FORTNITELAUNCHER.EXE")),
            Some(&games[0])
        );
        assert!(app_for(&games, Path::new(r"D:\Elsewhere\Other.exe")).is_none());
        // A sibling of the install folder is not inside it.
        assert!(app_for(&games, Path::new(r"D:\Epic\Fortnite2\x.exe")).is_none());
        assert!(app_for(&games, Path::new(r"D:\Epic\Fortnite\x.exe")).is_some());
    }
}
