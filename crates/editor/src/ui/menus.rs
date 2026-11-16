//! What the shell can be asked to do, and the keys that ask for it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Export,
    Import,
    Undo,
    Redo,
    AddRectangle,
    AddText,
    DeleteSelected,
}

impl Command {
    pub const ALL: [Self; 11] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Export,
        Self::Import,
        Self::Undo,
        Self::Redo,
        Self::AddRectangle,
        Self::AddText,
        Self::DeleteSelected,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open…",
            Self::Save => "Save",
            Self::SaveAs => "Save As…",
            Self::Export => "Export…",
            Self::Import => "Import…",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::AddRectangle => "Rectangle",
            Self::AddText => "Text",
            Self::DeleteSelected => "Delete",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Self::New => "Ctrl+N",
            Self::Open => "Ctrl+O",
            Self::Save => "Ctrl+S",
            Self::SaveAs => "Ctrl+Shift+S",
            Self::Export => "Ctrl+E",
            Self::Import => "Ctrl+I",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Shift+Z",            Self::AddRectangle => "R",
            Self::AddText => "T",
            Self::DeleteSelected => "Del",
        }
    }

    /// The command a keystroke asks for, or none. `platform` is the command key
    /// on macOS and the Windows key elsewhere, so a shortcut that names Ctrl also
    /// answers the platform's own modifier.
    pub fn from_key(key: &str, control: bool, shift: bool, platform: bool) -> Option<Self> {
        let named = control || platform;
        let key = key.to_ascii_lowercase();
        match (named, shift, key.as_str()) {
            (true, false, "s") => Some(Self::Save),
            (true, true, "s") => Some(Self::SaveAs),
            (true, false, "o") => Some(Self::Open),
            (true, false, "n") => Some(Self::New),
            (true, false, "e") => Some(Self::Export),
            (true, false, "i") => Some(Self::Import),
            (true, false, "z") => Some(Self::Undo),
            (true, true, "z") => Some(Self::Redo),
            // Windows' own redo chord, alongside the one every editor shares.
            (true, false, "y") => Some(Self::Redo),
            (false, _, "delete" | "backspace") => Some(Self::DeleteSelected),
            (false, false, "r") => Some(Self::AddRectangle),
            (false, false, "t") => Some(Self::AddText),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcuts_ask_for_the_commands_they_name() {
        assert_eq!(Command::from_key("s", true, false, false), Some(Command::Save));
        assert_eq!(
            Command::from_key("s", true, true, false),
            Some(Command::SaveAs)
        );
        assert_eq!(Command::from_key("z", true, true, false), Some(Command::Redo));
        assert_eq!(Command::from_key("y", true, false, false), Some(Command::Redo));
        assert_eq!(
            Command::from_key("s", false, false, true),
            Some(Command::Save)
        );
        assert_eq!(
            Command::from_key("delete", false, false, false),
            Some(Command::DeleteSelected)
        );
        assert_eq!(Command::from_key("s", false, false, false), None);
        assert_eq!(Command::from_key("q", true, false, false), None);
    }
}
