//! What the shell can be asked to do, and the keys that ask for it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Export,
    ExportWidget,
    Import,
    ImportAsset,
    Widgets,
    AssetFinder,
    Settings,
    /// The page that says how an agent reaches this host.
    Mcp,
    Undo,
    Redo,
    AddRectangle,
    AddEllipse,
    AddPath,
    AddPathPreset,
    TypographyPreset,
    AddLine,
    AddArc,
    AddWedge,
    AddText,
    AddVerticalText,
    AddImage,
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
    Store,
}

impl Command {
    pub const ALL: [Self; 48] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Export,
        Self::ExportWidget,
        Self::Import,
        Self::ImportAsset,
        Self::Widgets,
        Self::AssetFinder,
        Self::Settings,
        Self::Mcp,
        Self::Undo,
        Self::Redo,
        Self::AddRectangle,
        Self::AddEllipse,
        Self::AddPath,
        Self::AddPathPreset,
        Self::TypographyPreset,
        Self::AddLine,
        Self::AddArc,
        Self::AddWedge,
        Self::AddText,
        Self::AddVerticalText,
        Self::AddImage,
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
        Self::Store,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open…",
            Self::Save => "Save",
            Self::SaveAs => "Save As…",
            Self::Export => "Export…",
            Self::ExportWidget => "Export widget…",
            Self::Import => "Import…",
            Self::ImportAsset => "Import asset…",
            Self::Widgets => "Widgets…",
            Self::AssetFinder => "Asset finder…",
            Self::Settings => "Settings…",
            Self::Mcp => "Agent tools…",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::AddRectangle => "Rectangle",
            Self::AddEllipse => "Ellipse",
            Self::AddPath => "Path",
            Self::AddPathPreset => "Shape preset…",
            Self::TypographyPreset => "Typography preset…",
            Self::AddLine => "Horizontal line",
            Self::AddArc => "Arc",
            Self::AddWedge => "Wedge",
            Self::AddText => "Text",
            Self::AddVerticalText => "Vertical text",
            Self::AddImage => "Image",
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
            Self::Store => "Store…",
        }
    }

    /// The chords the product binds. `index.ts` in the source binds this same
    /// short list; every other command is reached from a pane, the canvas or
    /// the toolbar, so it prints no chord rather than an invented one.
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::New => "Ctrl+N",
            Self::Open => "Ctrl+O",
            Self::Save => "Ctrl+S",
            Self::AssetFinder => "Ctrl+Shift+F",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Shift+Z · Ctrl+Y",
            Self::Copy => "Ctrl+C",
            Self::Cut => "Ctrl+X",
            Self::Paste => "Ctrl+V",
            Self::Duplicate => "Ctrl+D",
            Self::Group => "Ctrl+G",
            Self::Ungroup => "Ctrl+Shift+G",
            Self::DeleteSelected => "Delete · Backspace",
            _ => "",
        }
    }

    /// The command a keystroke asks for, or none. `platform` is the command key
    /// on macOS and the Windows key elsewhere, so a shortcut that names Ctrl also
    /// answers the platform's own modifier.
    pub fn from_key(key: &str, control: bool, shift: bool, platform: bool) -> Option<Self> {
        let named = control || platform;
        let key = key.to_ascii_lowercase();
        match (named, shift, key.as_str()) {
            (true, false, "n") => Some(Self::New),
            (true, false, "o") => Some(Self::Open),
            (true, false, "s") => Some(Self::Save),
            // The source's comment: Chromium keeps Ctrl+Shift+A, so the finder
            // answers the chord that reaches the page.
            (true, true, "f") => Some(Self::AssetFinder),
            (true, true, "z") => Some(Self::Redo),
            // Windows' own redo chord, alongside the one every editor shares.
            (true, false, "y") => Some(Self::Redo),
            (true, false, "z") => Some(Self::Undo),
            (true, false, "c") => Some(Self::Copy),
            (true, false, "x") => Some(Self::Cut),
            (true, false, "v") => Some(Self::Paste),
            (true, false, "d") => Some(Self::Duplicate),
            (true, false, "g") => Some(Self::Group),
            (true, true, "g") => Some(Self::Ungroup),
            (false, _, "delete" | "backspace") => Some(Self::DeleteSelected),
            _ => None,
        }
    }
}

/// One row of the `?` sheet. The source builds it from `PRODUCT_SHORTCUTS`
/// grouped by the action id's prefix; the port keeps the same projection as a
/// table so a chord cannot be right in the sheet and wrong in the handler.
pub struct ShortcutRow {
    pub group: &'static str,
    pub label: &'static str,
    pub chord: &'static str,
}

/// The source's rows in the source's order: file, edit, canvas, help. The
/// publish row is the one absent — its surface lands in P4, and a sheet that
/// printed a chord for nothing is the lie the table exists to prevent.
pub const SHORTCUT_REFERENCE: [ShortcutRow; 21] = [
    ShortcutRow { group: "File", label: "New theme", chord: "Ctrl+N" },
    ShortcutRow { group: "File", label: "Import theme", chord: "Ctrl+O" },
    ShortcutRow { group: "File", label: "Save", chord: "Ctrl+S" },
    ShortcutRow { group: "File", label: "Find assets…", chord: "Ctrl+Shift+F" },
    ShortcutRow { group: "Edit", label: "Redo", chord: "Ctrl+Shift+Z · Ctrl+Y" },
    ShortcutRow { group: "Edit", label: "Undo", chord: "Ctrl+Z" },
    ShortcutRow { group: "Edit", label: "Copy", chord: "Ctrl+C" },
    ShortcutRow { group: "Edit", label: "Cut", chord: "Ctrl+X" },
    ShortcutRow { group: "Edit", label: "Paste", chord: "Ctrl+V" },
    ShortcutRow { group: "Edit", label: "Duplicate", chord: "Ctrl+D" },
    ShortcutRow { group: "Edit", label: "Ungroup", chord: "Ctrl+Shift+G" },
    ShortcutRow { group: "Edit", label: "Group", chord: "Ctrl+G" },
    ShortcutRow { group: "Edit", label: "Delete", chord: "Delete · Backspace" },
    ShortcutRow { group: "Canvas", label: "Select all", chord: "Ctrl+A" },
    ShortcutRow { group: "Canvas", label: "Bring to front", chord: "Ctrl+} · Ctrl+]" },
    ShortcutRow { group: "Canvas", label: "Send to back", chord: "Ctrl+{ · Ctrl+[" },
    ShortcutRow { group: "Canvas", label: "Nudge left", chord: "←" },
    ShortcutRow { group: "Canvas", label: "Nudge right", chord: "→" },
    ShortcutRow { group: "Canvas", label: "Nudge up", chord: "↑" },
    ShortcutRow { group: "Canvas", label: "Nudge down", chord: "↓" },
    ShortcutRow { group: "Help", label: "Keyboard shortcuts", chord: "?" },
];

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
            Command::from_key("backspace", false, false, false),
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
            Some(Command::AssetFinder)
        );
        // The source binds eight chords and no more; the rest are reached from
        // a pane or the toolbar.
        assert_eq!(Command::from_key("e", true, true, false), None);
        assert_eq!(Command::from_key("m", true, true, false), None);
        assert_eq!(Command::from_key("r", false, false, false), None);
        assert_eq!(Command::from_key("s", false, false, false), None);
        assert_eq!(Command::from_key("q", true, false, false), None);
    }

    #[test]
    fn only_the_commands_the_source_binds_print_a_chord() {
        let bound: Vec<_> = Command::ALL
            .into_iter()
            .filter(|command| !command.shortcut().is_empty())
            .collect();
        assert_eq!(
            bound,
            [
                Command::New,
                Command::Open,
                Command::Save,
                Command::AssetFinder,
                Command::Undo,
                Command::Redo,
                Command::DeleteSelected,
                Command::Group,
                Command::Ungroup,
                Command::Copy,
                Command::Cut,
                Command::Paste,
                Command::Duplicate,
            ]
        );
    }

    #[test]
    fn the_reference_lists_the_source_rows_in_the_source_group_order() {
        let mut groups: Vec<_> = Vec::new();
        for row in &SHORTCUT_REFERENCE {
            if groups.last() != Some(&row.group) {
                groups.push(row.group);
            }
        }
        assert_eq!(groups, ["File", "Edit", "Canvas", "Help"]);
        assert_eq!(
            SHORTCUT_REFERENCE
                .iter()
                .filter(|row| row.group == "File")
                .map(|row| row.label)
                .collect::<Vec<_>>(),
            ["New theme", "Import theme", "Save", "Find assets…"]
        );
        assert!(SHORTCUT_REFERENCE
            .iter()
            .any(|row| row.label == "Keyboard shortcuts" && row.chord == "?"));
    }
}
