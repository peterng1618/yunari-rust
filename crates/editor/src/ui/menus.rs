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
    AddEllipse,
    AddPath,
    AddLine,
    AddArc,
    AddWedge,
    AddText,
    AddVerticalText,
    AddGauge,
    AddLineChart,
    AddBarChart,
    AddPieChart,
    AddRadarChart,
    AddPolarBar,
    DeleteSelected,
    AlignLeft,
    AlignCenterX,
    AlignRight,
    AlignTop,
    AlignCenterY,
    AlignBottom,
    DistributeX,
    DistributeY,
    Group,
    Ungroup,
    Copy,
    Cut,
    Paste,
    Duplicate,
    OpenDisplay,
}

impl Command {
    pub const ALL: [Self; 38] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Export,
        Self::Import,
        Self::Undo,
        Self::Redo,
        Self::AddRectangle,
        Self::AddEllipse,
        Self::AddPath,
        Self::AddLine,
        Self::AddArc,
        Self::AddWedge,
        Self::AddText,
        Self::AddVerticalText,
        Self::AddGauge,
        Self::AddLineChart,
        Self::AddBarChart,
        Self::AddPieChart,
        Self::AddRadarChart,
        Self::AddPolarBar,
        Self::DeleteSelected,
        Self::AlignLeft,
        Self::AlignCenterX,
        Self::AlignRight,
        Self::AlignTop,
        Self::AlignCenterY,
        Self::AlignBottom,
        Self::DistributeX,
        Self::DistributeY,
        Self::Group,
        Self::Ungroup,
        Self::Copy,
        Self::Cut,
        Self::Paste,
        Self::Duplicate,
        Self::OpenDisplay,
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
            Self::AddEllipse => "Ellipse",
            Self::AddPath => "Path",
            Self::AddLine => "Horizontal line",
            Self::AddArc => "Arc",
            Self::AddWedge => "Wedge",
            Self::AddText => "Text",
            Self::AddVerticalText => "Vertical text",
            Self::AddGauge => "Gauge",
            Self::AddLineChart => "Line chart",
            Self::AddBarChart => "Bar chart",
            Self::AddPieChart => "Pie chart",
            Self::AddRadarChart => "Radar chart",
            Self::AddPolarBar => "Polar bar",
            Self::DeleteSelected => "Delete",
            Self::AlignLeft => "Align L",
            Self::AlignCenterX => "Centre X",
            Self::AlignRight => "Align R",
            Self::AlignTop => "Align T",
            Self::AlignCenterY => "Centre Y",
            Self::AlignBottom => "Align B",
            Self::DistributeX => "Space X",
            Self::DistributeY => "Space Y",
            Self::Group => "Group",
            Self::Ungroup => "Ungroup",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::Duplicate => "Duplicate",
            Self::OpenDisplay => "Open display",
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
            Self::Redo => "Ctrl+Shift+Z",
            Self::AddRectangle => "R",
            Self::AddEllipse => "E",
            Self::AddPath => "",
            Self::AddLine => "",
            Self::AddArc => "",
            Self::AddWedge => "",
            Self::AddText => "T",
            Self::AddVerticalText => "",
            Self::AddGauge => "",
            Self::AddLineChart => "",
            Self::AddBarChart => "",
            Self::AddPieChart => "",
            Self::AddRadarChart => "",
            Self::AddPolarBar => "",
            Self::DeleteSelected => "Del",
            Self::AlignLeft => "",
            Self::AlignCenterX => "",
            Self::AlignRight => "",
            Self::AlignTop => "",
            Self::AlignCenterY => "",
            Self::AlignBottom => "",
            Self::DistributeX => "",
            Self::DistributeY => "",
            Self::Group => "Ctrl+G",
            Self::Ungroup => "Ctrl+Shift+G",
            Self::Copy => "Ctrl+C",
            Self::Cut => "Ctrl+X",
            Self::Paste => "Ctrl+V",
            Self::Duplicate => "Ctrl+D",
            Self::OpenDisplay => "Ctrl+Shift+F",
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
            (true, false, "g") => Some(Self::Group),
            (true, true, "g") => Some(Self::Ungroup),
            (true, false, "c") => Some(Self::Copy),
            (true, false, "x") => Some(Self::Cut),
            (true, false, "v") => Some(Self::Paste),
            (true, false, "d") => Some(Self::Duplicate),
            (true, true, "f") => Some(Self::OpenDisplay),
            (false, _, "delete" | "backspace") => Some(Self::DeleteSelected),
            (false, false, "r") => Some(Self::AddRectangle),
            (false, false, "e") => Some(Self::AddEllipse),
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
        assert_eq!(
            Command::from_key("s", true, false, false),
            Some(Command::Save)
        );
        assert_eq!(
            Command::from_key("s", true, true, false),
            Some(Command::SaveAs)
        );
        assert_eq!(
            Command::from_key("z", true, true, false),
            Some(Command::Redo)
        );
        assert_eq!(
            Command::from_key("y", true, false, false),
            Some(Command::Redo)
        );
        assert_eq!(
            Command::from_key("s", false, false, true),
            Some(Command::Save)
        );
        assert_eq!(
            Command::from_key("delete", false, false, false),
            Some(Command::DeleteSelected)
        );
        assert_eq!(
            Command::from_key("g", true, false, false),
            Some(Command::Group)
        );
        assert_eq!(
            Command::from_key("g", true, true, false),
            Some(Command::Ungroup)
        );
        assert_eq!(
            Command::from_key("v", true, false, false),
            Some(Command::Paste)
        );
        assert_eq!(
            Command::from_key("f", true, true, false),
            Some(Command::OpenDisplay)
        );
        assert_eq!(Command::from_key("s", false, false, false), None);
        assert_eq!(Command::from_key("q", true, false, false), None);
    }
}
