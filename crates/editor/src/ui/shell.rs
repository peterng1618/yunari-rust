//! The editor's chrome: the source header's mode switch and toolbar, the
//! rail plus fixed pane/stage/inspector columns and the status bar.

use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::EditorWindow;
use crate::ui::menus::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    Themes,
    Editor,
    Settings,
}

impl ShellMode {
    pub const ALL: [Self; 3] = [Self::Themes, Self::Editor, Self::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Themes => "Themes",
            Self::Editor => "Editor",
            Self::Settings => "Settings",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Themes => 0,
            Self::Editor => 1,
            Self::Settings => 2,
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Themes => IconName::BookOpen,
            Self::Editor => IconName::Frame,
            Self::Settings => IconName::Settings,
        }
    }
}

/// The source rail's slots, in order. Each one has a pane body; `?` opens the
/// shortcut sheet listing the rest of `Command::ALL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftPane {
    Layers,
    Add,
    Widgets,
    Assets,
    Tokens,
    Document,
}

impl LeftPane {
    pub const ALL: [Self; 6] = [
        Self::Layers,
        Self::Add,
        Self::Widgets,
        Self::Assets,
        Self::Tokens,
        Self::Document,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layers",
            Self::Add => "Add",
            Self::Widgets => "Widgets",
            Self::Assets => "Assets",
            Self::Tokens => "Design tokens",
            Self::Document => "Theme settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Layers => IconName::Inspector,
            Self::Add => IconName::Plus,
            Self::Widgets => IconName::LayoutDashboard,
            Self::Assets => IconName::GalleryVerticalEnd,
            Self::Tokens => IconName::Palette,
            Self::Document => IconName::FileText,
        }
    }
}

/// The source header's toolbar, in source order: the document's lifecycle,
/// its own actions, then the editor's browsing tool. The rest of `Command::ALL`
/// is reachable from the `?` shortcut sheet and the panes.
pub const TOOLBAR_GROUPS: [&[Command]; 3] = [
    &[
        Command::New,
        Command::NewFromTemplate,
        Command::Open,
        Command::Import,
    ],
    &[
        Command::Save,
        Command::SaveAs,
        Command::OpenDisplay,
        Command::Export,
        Command::ExportWidget,
    ],
    &[Command::AssetFinder],
];

fn toolbar_icon(command: Command) -> IconName {
    match command {
        Command::New => IconName::Plus,
        Command::NewFromTemplate => IconName::LayoutDashboard,
        Command::Open => IconName::FolderOpen,
        Command::Import => IconName::File,
        Command::Save => IconName::HardDrive,
        Command::SaveAs => IconName::Copy,
        Command::Export => IconName::ExternalLink,
        Command::ExportWidget => IconName::Frame,
        Command::OpenDisplay => IconName::Play,
        Command::AssetFinder => IconName::Search,
        _ => IconName::Ellipsis,
    }
}

impl EditorWindow {
    pub(super) fn set_mode(&mut self, mode: ShellMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    /// Chooses a rail slot. Choosing the slot that is already showing closes
    /// the pane, which is what the source's rail press does.
    pub(super) fn choose_pane(&mut self, pane: LeftPane, cx: &mut Context<Self>) {
        if pane == self.left_pane && !self.left_collapsed {
            self.left_collapsed = true;
        } else {
            self.left_pane = pane;
            self.left_collapsed = false;
        }
        cx.notify();
    }

    pub(super) fn toggle_left_pane(&mut self, cx: &mut Context<Self>) {
        self.left_collapsed = !self.left_collapsed;
        cx.notify();
    }

    pub(super) fn toggle_right_pane(&mut self, cx: &mut Context<Self>) {
        self.right_collapsed = !self.right_collapsed;
        cx.notify();
    }

    pub(super) fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme, Sizable, button::{Button, ButtonVariants}, v_flex,
        };

        let active = self.left_pane;
        v_flex()
            .w_full()
            .h_full()
            .items_center()
            .gap_1()
            .py_2()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .children(LeftPane::ALL.map(|pane| {
                Button::new(format!("rail-{pane:?}"))
                    .ghost()
                    .small()
                    .icon(pane.icon())
                    .tooltip(pane.label())
                    .toggled(pane == active && !self.left_collapsed)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.choose_pane(pane, cx)
                    }))
            }))
            .child(div().flex_1())
            .child(
                Button::new("rail-settings")
                    .ghost()
                    .small()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.set_mode(ShellMode::Settings, cx)
                    })),
            )
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme, Sizable, TitleBar, button::{Button, ButtonVariants, Toggle, ToggleGroup},
            h_flex, toolbar::Toolbar,
        };

        let mode = self.mode;
        let left = self.left_collapsed;
        let right = self.right_collapsed;

        let modes = ShellMode::ALL
            .into_iter()
            .map(|candidate| {
                Toggle::new(format!("shell-mode-{}", candidate.index()))
                    .icon(candidate.icon())
                    .label(candidate.label())
                    .checked(candidate == mode)
            })
            .collect::<Vec<_>>();

        let mut toolbar = Toolbar::new("theme-toolbar").flex_none();
        for (group_index, group) in TOOLBAR_GROUPS.iter().enumerate() {
            if group_index > 0 {
                toolbar = toolbar.content(div().w(px(1.0)).h(px(16.0)).bg(cx.theme().border));
            }
            for command in group.iter().copied() {
                toolbar = toolbar.child(
                    Button::new(format!("toolbar-{:?}", command))
                        .ghost()
                        .icon(toolbar_icon(command))
                        .tooltip(command.label())
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.apply(command, window, cx)
                        })),
                );
            }
        }

        TitleBar::new().child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child("Yunari"),
                )
                .child(
                    ToggleGroup::new("shell-modes")
                        .small()
                        .single()
                        .segmented()
                        .children(modes)
                        .on_click(cx.listener(|this, states: &Vec<bool>, _, cx| {
                            if let Some(index) = states.iter().position(|checked| *checked) {
                                this.set_mode(ShellMode::ALL[index], cx);
                            }
                        })),
                )
                .when(mode == ShellMode::Editor, |this| this.child(toolbar))
                .child(div().flex_1())
                .child(
                    Button::new("toggle-left-pane")
                        .ghost()
                        .small()
                        .icon(IconName::PanelLeft)
                        .tooltip(if left {
                            "Show left pane"
                        } else {
                            "Hide left pane"
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_left_pane(cx)
                        })),
                )
                .child(
                    Button::new("toggle-right-pane")
                        .ghost()
                        .small()
                        .icon(IconName::PanelRight)
                        .tooltip(if right {
                            "Show inspector"
                        } else {
                            "Hide inspector"
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_right_pane(cx)
                        })),
                ),
        )
    }

    pub(super) fn body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme,
            empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
            h_flex,
        };

        let mut editor = h_flex().size_full().min_h(px(0.0));
        editor = editor.child(
            div()
                .w(px(46.0))
                .flex_shrink_0()
                .h_full()
                .child(self.rail(cx)),
        );
        if !self.left_collapsed {
            let pane = match self.left_pane {
                LeftPane::Layers => self.layers_view(cx).into_any_element(),
                LeftPane::Document => self.document_view(cx).into_any_element(),
                LeftPane::Tokens => self.tokens_view(cx).into_any_element(),
                LeftPane::Assets => self.assets_view(cx).into_any_element(),
                LeftPane::Add => self.add_view(cx).into_any_element(),
                LeftPane::Widgets => self.widgets_view(cx).into_any_element(),
            };
            editor = editor.child(div().w(px(246.0)).flex_shrink_0().h_full().child(pane));
        }
        editor = editor.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(self.canvas_view(cx)),
        );
        if !self.right_collapsed {
            editor = editor.child(
                div()
                    .w(px(276.0))
                    .flex_shrink_0()
                    .h_full()
                    .child(self.inspector_view(cx)),
            );
        }

        div()
            .flex_1()
            .w_full()
            .min_h(px(0.0))
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .size_full()
                    .when(self.mode != ShellMode::Editor, |this| this.invisible())
                    .child(editor),
            )
            .when(self.mode != ShellMode::Editor, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().background)
                        .child(
                            Empty::new().header(
                                EmptyHeader::new()
                                    .title(EmptyTitle::new().child(self.mode.label()))
                                    .description(EmptyDescription::new().child(
                                        "This surface lands in P4; the open theme is kept.",
                                    )),
                            ),
                        ),
                )
            })
    }

    pub(super) fn status_bar(&self, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::{ActiveTheme, h_flex, status_bar::StatusBar};

        let document = self.editor.document();
        let name = document
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| document.id.clone());
        let objects = crate::layers::count(document);

        // The source's order: how much is there, what is chosen, what is open.
        let mut left = h_flex()
            .gap_2()
            .child(format!("{objects} objects"))
            .child(format!("{} selected", self.selection.len()))
            .child(format!("Opened {name}"));
        if self.editor.is_dirty() {
            left = left.child("unsaved changes");
        }

        let mut right = h_flex().gap_2();
        if let Some(status) = &self.status {
            right = right.child(
                div()
                    .text_color(if status.error {
                        cx.theme().danger
                    } else {
                        cx.theme().success
                    })
                    .child(status.text.clone()),
            );
        }
        // `live` is the host being reachable, as the source's indicator reads.
        let live = self.host.is_some();
        right = right.child(
            h_flex()
                .gap_1()
                .child(div().text_color(if live {
                    cx.theme().success
                } else {
                    cx.theme().muted_foreground
                }).child(if live { IconName::Check } else { IconName::CircleX }))
                .child(if live { "live" } else { "offline" }),
        );

        StatusBar::new().left(left).right(right)
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, LeftPane, ShellMode, TOOLBAR_GROUPS};

    #[test]
    fn shell_modes_are_themes_editor_settings_in_order() {
        assert_eq!(
            ShellMode::ALL.map(ShellMode::label),
            ["Themes", "Editor", "Settings"]
        );
        assert_eq!(ShellMode::ALL.map(ShellMode::index), [0, 1, 2]);
    }

    #[test]
    fn rail_slots_match_the_source_order() {
        assert_eq!(
            LeftPane::ALL.map(LeftPane::label),
            [
                "Layers",
                "Add",
                "Widgets",
                "Assets",
                "Design tokens",
                "Theme settings",
            ]
        );
    }

    #[test]
    fn toolbar_groups_match_the_source_header() {
        assert_eq!(
            TOOLBAR_GROUPS,
            [
                &[
                    Command::New,
                    Command::NewFromTemplate,
                    Command::Open,
                    Command::Import,
                ][..],
                &[
                    Command::Save,
                    Command::SaveAs,
                    Command::OpenDisplay,
                    Command::Export,
                    Command::ExportWidget,
                ][..],
                &[Command::AssetFinder][..],
            ]
        );
    }

    #[test]
    fn toolbar_groups_do_not_repeat_a_command() {
        let mut seen = Vec::new();
        for command in TOOLBAR_GROUPS.into_iter().flatten() {
            assert!(!seen.contains(command), "{command:?} appears twice");
            seen.push(*command);
        }
    }
}
