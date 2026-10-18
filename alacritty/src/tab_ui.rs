use std::path::PathBuf;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::localization::{Locale, Message};
use crate::session::CommandRecord;
use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogAction {
    Apply,
    Save,
    Restore,
    Cancel,
}

pub struct TabDialog {
    pub tab_id: TabId,
    pub name: String,
    pub working_directory: Option<PathBuf>,
    pub commands: Vec<CommandRecord>,
    pub replay_last_command: bool,
    pub tracking_supported: bool,
    pub status: Option<Message>,
    pub locale: Locale,
    focus: usize,
    cursor: usize,
    selected_all: bool,
    hovered: Option<Control>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiRect {
    pub left: usize,
    pub top: usize,
    pub width: usize,
    pub height: usize,
}

impl UiRect {
    pub fn contains(self, column: usize, line: usize) -> bool {
        column >= self.left
            && column < self.left.saturating_add(self.width)
            && line >= self.top
            && line < self.top.saturating_add(self.height)
    }
}

pub type DialogLayout = UiRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKind {
    Title,
    Subtitle,
    Label,
    Input,
    Checkbox,
    Command,
    PrimaryButton,
    SecondaryButton,
    Link,
    Status,
    Divider,
    Navigation,
    SelectedNavigation,
    MenuItem,
    DangerButton,
    DangerMenu,
    Radio,
    SelectedItem,
    Scrollbar,
    Shortcut,
}

pub struct UiElement {
    pub rect: UiRect,
    pub kind: UiKind,
    pub text: String,
    pub focused: bool,
    pub hovered: bool,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Name,
    Replay,
    Apply,
    Save,
    Restore,
    Cancel,
    Close,
}

impl Control {
    fn focus(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Replay => 1,
            Self::Apply => 2,
            Self::Save => 3,
            Self::Restore => 4,
            Self::Cancel | Self::Close => 5,
        }
    }
}

impl TabDialog {
    pub fn new(
        tab_id: TabId,
        name: String,
        working_directory: Option<PathBuf>,
        commands: Vec<CommandRecord>,
        replay_last_command: bool,
        tracking_supported: bool,
    ) -> Self {
        let cursor = name.len();
        Self {
            tab_id,
            name,
            working_directory,
            commands,
            replay_last_command,
            tracking_supported,
            status: None,
            locale: Locale::En,
            focus: 0,
            cursor,
            selected_all: true,
            hovered: None,
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> DialogLayout {
        let width = if columns >= 12 { columns.saturating_sub(4) } else { columns }.min(76);
        let height = lines.min(24);
        UiRect {
            left: columns.saturating_sub(width) / 2,
            top: lines.saturating_sub(height) / 2,
            width,
            height,
        }
    }

    pub fn elements(&self, columns: usize, lines: usize) -> Vec<UiElement> {
        self.parts(columns, lines).into_iter().map(|(element, _)| element).collect()
    }

    fn part(
        &self,
        layout: UiRect,
        position: (usize, usize, usize),
        kind: UiKind,
        text: String,
        control: Option<Control>,
    ) -> (UiElement, Option<Control>) {
        let (left, top, width) = position;
        let disabled = control == Some(Control::Replay) && !self.can_replay();
        (
            UiElement {
                rect: UiRect {
                    left: layout.left + left,
                    top: layout.top + top,
                    width: width.min(layout.width.saturating_sub(left)),
                    height: usize::from(top < layout.height),
                },
                kind,
                text,
                focused: control.is_some_and(|control| {
                    control != Control::Close && self.focus == control.focus()
                }),
                hovered: control.is_some() && self.hovered == control,
                disabled,
            },
            control,
        )
    }

    fn parts(&self, columns: usize, lines: usize) -> Vec<(UiElement, Option<Control>)> {
        let layout = self.layout(columns, lines);
        if layout.width == 0 || layout.height == 0 {
            return Vec::new();
        }
        let inset = if layout.width >= 12 { 2 } else { 0 };
        let width = layout.width.saturating_sub(inset * 2);
        let mut parts = Vec::new();
        let mut add = |row, kind, text: String, control| {
            parts.push(self.part(layout, (inset, row, width), kind, text, control));
        };
        if layout.height < 12 {
            add(0, UiKind::Title, self.locale.text(Message::DialogTitle).into(), None);
            add(1, UiKind::Input, self.visible_name(width).1, Some(Control::Name));
            add(2, UiKind::Checkbox, self.replay_label(), Some(Control::Replay));
            if layout.height >= 7 {
                for (row, control) in [
                    (3, Control::Apply),
                    (4, Control::Save),
                    (5, Control::Cancel),
                    (6, Control::Restore),
                ] {
                    let (kind, text) = self.control_label(control, width);
                    add(row, kind, text.into(), Some(control));
                }
            } else if layout.height >= 4 {
                let control = match self.focus {
                    3 => Control::Save,
                    4 => Control::Restore,
                    5 => Control::Cancel,
                    _ => Control::Apply,
                };
                let (kind, text) = self.control_label(control, width);
                add(layout.height - 1, kind, text.into(), Some(control));
            }
            if layout.height >= 9 {
                add(
                    8,
                    UiKind::Status,
                    self.status.map_or_else(
                        || self.locale.text(Message::KeyboardHelpCompact).into(),
                        |message| self.locale.text(message).into(),
                    ),
                    None,
                );
            }
        } else {
            let wide = width >= 54;
            let button_row = layout.height - if wide { 5 } else { 7 };
            let body_end = button_row.saturating_sub(1);
            add(1, UiKind::Title, self.locale.text(Message::DialogTitle).into(), None);
            let mut row = 2;
            if layout.height >= 20 {
                add(row, UiKind::Subtitle, self.locale.text(Message::DialogSubtitle).into(), None);
                row += 1;
            }
            if body_end >= 17 {
                row += 1;
            }
            if body_end.saturating_sub(row) >= 4 {
                add(row, UiKind::Label, self.locale.text(Message::Name).into(), None);
                row += 1;
            }
            add(row, UiKind::Input, self.visible_name(width).1, Some(Control::Name));
            row += 1;
            if body_end.saturating_sub(row) >= 9 {
                add(row, UiKind::Subtitle, self.locale.text(Message::NameHelp).into(), None);
                row += 1;
            }
            if body_end.saturating_sub(row) >= 8 {
                if body_end >= 17 {
                    row += 1;
                }
                add(row, UiKind::Label, self.locale.text(Message::WorkingDirectory).into(), None);
                row += 1;
                add(row, UiKind::Subtitle, self.directory_text(), None);
                row += 1;
            } else if body_end.saturating_sub(row) >= 4 {
                add(
                    row,
                    UiKind::Subtitle,
                    format!(
                        "{} {}",
                        self.locale.text(Message::DirectoryPrefix),
                        self.directory_text()
                    ),
                    None,
                );
                row += 1;
            }
            if body_end.saturating_sub(row) >= 5 {
                if body_end >= 17 {
                    row += 1;
                }
                add(row, UiKind::Label, self.locale.text(Message::RecentCommands).into(), None);
                row += 1;
                let count = body_end.saturating_sub(row + 2).min(3);
                for index in 0..count {
                    let text = self.commands.iter().rev().nth(index).map_or_else(
                        || {
                            if index == 0 {
                                self.locale.text(Message::NoCommands).into()
                            } else {
                                String::new()
                            }
                        },
                        |record| {
                            record
                                .command
                                .chars()
                                .map(|ch| if ch.is_control() { ' ' } else { ch })
                                .collect()
                        },
                    );
                    add(row, UiKind::Command, text, None);
                    row += 1;
                }
            }
            if body_end.saturating_sub(row) >= 3 {
                row += 1;
            }
            add(row, UiKind::Checkbox, self.replay_label(), Some(Control::Replay));
            row += 1;
            if row < body_end {
                let text = if !self.tracking_supported {
                    self.locale.text(Message::ReplayUnavailable)
                } else if self.commands.is_empty() {
                    self.locale.text(Message::ReplayNoCommand)
                } else {
                    self.locale.text(Message::ReplayHelp)
                };
                add(row, UiKind::Subtitle, text.into(), None);
            }
            add(body_end, UiKind::Divider, String::new(), None);
            if wide {
                for (left, button_width, control) in [
                    (inset, 12, Control::Apply),
                    (inset + 14, 26, Control::Save),
                    (inset + 42, 12, Control::Cancel),
                ] {
                    let (kind, text) = self.control_label(control, button_width);
                    parts.push(self.part(
                        layout,
                        (left, button_row, button_width),
                        kind,
                        text.into(),
                        Some(control),
                    ));
                }
            } else {
                for (offset, control) in
                    [Control::Apply, Control::Save, Control::Cancel].into_iter().enumerate()
                {
                    let (kind, text) = self.control_label(control, width);
                    parts.push(self.part(
                        layout,
                        (inset, button_row + offset, width),
                        kind,
                        text.into(),
                        Some(control),
                    ));
                }
            }
            let (kind, text) = self.control_label(Control::Restore, width);
            parts.push(self.part(
                layout,
                (inset, layout.height - 3, width),
                kind,
                text.into(),
                Some(Control::Restore),
            ));
            parts.push(self.part(
                layout,
                (inset, layout.height - 2, width),
                UiKind::Status,
                self.status.map_or_else(
                    || self.locale.text(Message::KeyboardHelp).into(),
                    |message| self.locale.text(message).into(),
                ),
                None,
            ));
        }
        let close_row = usize::from(layout.height >= 12);
        parts.push(self.part(
            layout,
            (layout.width.saturating_sub(4), close_row, 2.min(layout.width)),
            UiKind::SecondaryButton,
            "×".into(),
            Some(Control::Close),
        ));
        parts.retain(|(element, _)| element.rect.width > 0 && element.rect.height > 0);
        parts
    }

    fn control_label(&self, control: Control, width: usize) -> (UiKind, &'static str) {
        match control {
            Control::Save => (
                UiKind::PrimaryButton,
                if width < 22 {
                    self.locale.text(Message::SaveTabs)
                } else {
                    self.locale.text(Message::SaveAll)
                },
            ),
            Control::Apply => (UiKind::SecondaryButton, self.locale.text(Message::Apply)),
            Control::Cancel | Control::Close => {
                (UiKind::SecondaryButton, self.locale.text(Message::Cancel))
            },
            Control::Restore => (
                UiKind::Link,
                if width < 22 {
                    self.locale.text(Message::RestoreShort)
                } else {
                    self.locale.text(Message::Restore)
                },
            ),
            Control::Name | Control::Replay => unreachable!(),
        }
    }

    fn directory_text(&self) -> String {
        self.working_directory.as_ref().map_or_else(
            || self.locale.text(Message::Unavailable).into(),
            |path| {
                path.to_string_lossy()
                    .chars()
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect()
            },
        )
    }

    fn replay_label(&self) -> String {
        format!(
            "{} {}",
            if self.replay_last_command { "[✓]" } else { "[ ]" },
            self.locale.text(Message::Replay)
        )
    }

    fn can_replay(&self) -> bool {
        self.tracking_supported && !self.commands.is_empty()
    }

    fn visible_name(&self, width: usize) -> (usize, String) {
        let available = width.saturating_sub(3).max(1);
        let mut start = 0;
        while start < self.cursor
            && UnicodeWidthStr::width(&self.name[start..self.cursor]) >= available
        {
            start += self.name[start..].chars().next().unwrap().len_utf8();
        }
        let mut text = self.name[start..].to_owned();
        if self.focus == 0 {
            text.insert(self.cursor - start, '│');
        } else if text.is_empty() {
            text = self.locale.text(Message::Name).into();
        }
        (start, text)
    }

    pub fn insert_text(&mut self, text: &str) {
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        let current_length = if self.selected_all { 0 } else { self.name.len() };
        if current_length + text.len() > 256 {
            return;
        };
        if self.selected_all {
            self.name.clear();
            self.cursor = 0;
            self.selected_all = false;
        }
        self.name.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    pub fn key(&mut self, event: &KeyEvent, modifiers: ModifiersState) -> Option<DialogAction> {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => return Some(DialogAction::Cancel),
            Key::Named(NamedKey::Tab) => {
                self.focus = (self.focus + if modifiers.shift_key() { 5 } else { 1 }) % 6;
            },
            Key::Named(NamedKey::Enter) => return self.activate(),
            Key::Named(NamedKey::Space) if self.focus == 1 => self.toggle_replay(),
            Key::Character(text) if text.as_str() == " " && self.focus == 1 => self.toggle_replay(),
            Key::Character(text)
                if text.eq_ignore_ascii_case("a") && modifiers.control_key() && self.focus == 0 =>
            {
                self.selected_all = true;
            },
            Key::Named(NamedKey::Backspace | NamedKey::Delete) if self.focus == 0 => {
                if self.selected_all {
                    self.name.clear();
                    self.cursor = 0;
                    self.selected_all = false;
                } else if event.logical_key == Key::Named(NamedKey::Backspace) && self.cursor > 0 {
                    let previous = self.name[..self.cursor].char_indices().next_back().unwrap().0;
                    self.name.drain(previous..self.cursor);
                    self.cursor = previous;
                } else if event.logical_key == Key::Named(NamedKey::Delete)
                    && self.cursor < self.name.len()
                {
                    let end =
                        self.cursor + self.name[self.cursor..].chars().next().unwrap().len_utf8();
                    self.name.drain(self.cursor..end);
                }
            },
            Key::Named(NamedKey::ArrowLeft) if self.focus == 0 => {
                self.cursor =
                    self.name[..self.cursor].char_indices().next_back().map_or(0, |(i, _)| i);
                self.selected_all = false;
            },
            Key::Named(NamedKey::ArrowRight) if self.focus == 0 => {
                self.cursor += self.name[self.cursor..].chars().next().map_or(0, char::len_utf8);
                self.selected_all = false;
            },
            Key::Named(NamedKey::Home) if self.focus == 0 => {
                self.cursor = 0;
                self.selected_all = false;
            },
            Key::Named(NamedKey::End) if self.focus == 0 => {
                self.cursor = self.name.len();
                self.selected_all = false;
            },
            _ if self.focus == 0
                && !modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key() =>
            {
                if let Some(text) = &event.text {
                    self.insert_text(text);
                }
            },
            _ => (),
        }
        None
    }

    pub fn click(
        &mut self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<DialogAction> {
        if !self.layout(columns, lines).contains(column, line) {
            return Some(DialogAction::Cancel);
        }
        let control =
            self.parts(columns, lines).into_iter().rev().find_map(|(element, control)| {
                (element.rect.contains(column, line) && !element.disabled)
                    .then_some((element.rect, control?))
            });
        let (rect, control) = control?;
        let visible_caret = self.focus == 0;
        let previous_cursor = self.cursor;
        self.focus = control.focus();
        if control == Control::Name {
            let start = self.visible_name(rect.width).0;
            let target = column.saturating_sub(rect.left + 1);
            let mut cell = 0;
            self.cursor = self.name.len();
            for (offset, ch) in self.name[start..].char_indices() {
                if visible_caret && start + offset == previous_cursor {
                    if target <= cell {
                        self.cursor = previous_cursor;
                        break;
                    }
                    cell += 1;
                }
                let width = UnicodeWidthChar::width(ch).unwrap_or(0);
                if cell + width > target {
                    self.cursor = start + offset;
                    break;
                }
                cell += width;
            }
            self.selected_all = false;
            None
        } else if control == Control::Close {
            Some(DialogAction::Cancel)
        } else {
            self.activate()
        }
    }

    pub fn hover(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let hovered =
            self.parts(columns, lines).into_iter().rev().find_map(|(element, control)| {
                (element.rect.contains(column, line) && !element.disabled)
                    .then_some(control)
                    .flatten()
            });
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }

    pub fn pointer_cursor(&self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        self.parts(columns, lines).into_iter().any(|(element, control)| {
            control.is_some() && !element.disabled && element.rect.contains(column, line)
        })
    }

    pub fn wants_paste(&self) -> bool {
        self.focus == 0
    }

    fn toggle_replay(&mut self) {
        if self.can_replay() {
            self.replay_last_command = !self.replay_last_command;
        }
    }

    fn activate(&mut self) -> Option<DialogAction> {
        match self.focus {
            0 | 2 => Some(DialogAction::Apply),
            1 => {
                self.toggle_replay();
                None
            },
            3 => Some(DialogAction::Save),
            4 => Some(DialogAction::Restore),
            _ => Some(DialogAction::Cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> TabDialog {
        TabDialog::new(
            1,
            "Produção".into(),
            Some("/srv/projeto".into()),
            vec![CommandRecord { command: "echo ready".into(), working_directory: "/tmp".into() }],
            false,
            true,
        )
    }

    #[test]
    fn oversized_paste_preserves_the_selected_name() {
        let mut dialog = TabDialog::new(1, "Projeto".into(), None, Vec::new(), false, true);
        dialog.insert_text(&"x".repeat(257));
        assert_eq!(dialog.name, "Projeto");
        dialog.insert_text("Outro projeto");
        assert_eq!(dialog.name, "Outro projeto");
    }

    #[test]
    fn renaming_replaces_selected_unicode_text_and_filters_control_input() {
        let mut dialog = TabDialog::new(1, "日本語".into(), None, vec![], false, true);
        dialog.insert_text("API\r\n");
        assert_eq!(dialog.name, "API");
        dialog.insert_text(" produção");
        assert_eq!(dialog.name, "API produção");
    }

    #[test]
    fn replay_checkbox_requires_a_command_and_supported_shell() {
        let mut dialog = dialog();
        let rect = dialog
            .elements(80, 24)
            .into_iter()
            .find(|element| element.kind == UiKind::Checkbox)
            .unwrap()
            .rect;
        dialog.click(rect.left, rect.top, 80, 24);
        assert!(dialog.replay_last_command);
        dialog.tracking_supported = false;
        dialog.replay_last_command = false;
        dialog.click(rect.left, rect.top, 80, 24);
        assert!(!dialog.replay_last_command);
        assert!(!dialog.pointer_cursor(rect.left, rect.top, 80, 24));
        dialog.tracking_supported = true;
        dialog.commands.clear();
        dialog.click(rect.left, rect.top, 80, 24);
        assert!(!dialog.replay_last_command);
    }

    #[test]
    fn locales_translate_controls_and_notices_while_preserving_user_content() {
        let mut dialog = dialog();
        dialog.name = "Nome personalizado".into();
        dialog.cursor = dialog.name.len();
        dialog.working_directory = Some(PathBuf::from("/srv/produção"));
        dialog.status = Some(Message::SessionSaved);
        let command = dialog.commands[0].command.clone();
        for locale in [Locale::En, Locale::PtBr] {
            dialog.locale = locale;
            let elements = dialog.elements(80, 24);
            assert!(
                elements.iter().any(|element| element.text == locale.text(Message::DialogTitle))
            );
            assert!(
                elements.iter().any(|element| element.text == locale.text(Message::SessionSaved))
            );
            assert!(elements.iter().any(|element| element.text == "/srv/produção"));
            assert!(elements.iter().any(|element| element.text.contains("Nome personalizado")));
            assert_eq!(dialog.commands[0].command, command);
            dialog.working_directory = None;
            assert_eq!(dialog.directory_text(), locale.text(Message::Unavailable));
            dialog.working_directory = Some(PathBuf::from("/srv/produção"));
        }
    }

    #[test]
    fn dialog_buttons_use_actual_rectangles_and_blank_space_does_nothing() {
        let mut dialog = dialog();
        for (label, action) in [
            (Locale::En.text(Message::Apply), DialogAction::Apply),
            (Locale::En.text(Message::SaveAll), DialogAction::Save),
            (Locale::En.text(Message::Restore), DialogAction::Restore),
            (Locale::En.text(Message::Cancel), DialogAction::Cancel),
            ("×", DialogAction::Cancel),
        ] {
            let rect = dialog
                .elements(80, 24)
                .into_iter()
                .find(|element| element.text == label)
                .unwrap()
                .rect;
            assert_eq!(dialog.click(rect.left, rect.top, 80, 24), Some(action));
        }
        let layout = dialog.layout(80, 24);
        assert_eq!(dialog.click(layout.left, layout.top + 8, 80, 24), None);
        assert_eq!(dialog.click(0, 0, 80, 24), Some(DialogAction::Cancel));
    }

    #[test]
    fn hover_tracks_the_same_rectangles_as_click() {
        let mut dialog = dialog();
        let rect = dialog
            .elements(80, 24)
            .into_iter()
            .find(|element| element.kind == UiKind::PrimaryButton)
            .unwrap()
            .rect;
        assert!(dialog.hover(rect.left, rect.top, 80, 24));
        assert!(!dialog.hover(rect.left + 1, rect.top, 80, 24));
        assert!(
            dialog
                .elements(80, 24)
                .into_iter()
                .any(|element| element.kind == UiKind::PrimaryButton && element.hovered)
        );
        assert!(dialog.hover(0, 0, 80, 24));
    }

    #[test]
    fn compact_dialog_keeps_controls_inside_the_viewport() {
        for (columns, lines) in [(80, 24), (40, 20), (30, 12), (16, 9), (8, 7), (3, 3), (0, 0)] {
            let dialog = dialog();
            for element in dialog.elements(columns, lines) {
                assert!(element.rect.left + element.rect.width <= columns);
                assert!(element.rect.top + element.rect.height <= lines);
            }
            if columns > 0 && lines >= 7 {
                let controls = dialog
                    .parts(columns, lines)
                    .into_iter()
                    .filter_map(|(_, control)| control)
                    .collect::<Vec<_>>();
                for control in [
                    Control::Name,
                    Control::Replay,
                    Control::Apply,
                    Control::Save,
                    Control::Restore,
                    Control::Cancel,
                    Control::Close,
                ] {
                    assert!(controls.contains(&control), "{columns} × {lines}: {control:?}");
                }
            }
        }
    }

    #[test]
    fn clicking_name_places_cursor_at_unicode_cell_boundaries() {
        let mut dialog = dialog();
        dialog.name = "日本abc".into();
        dialog.cursor = dialog.name.len();
        let rect = dialog
            .elements(80, 24)
            .into_iter()
            .find(|element| element.kind == UiKind::Input)
            .unwrap()
            .rect;
        dialog.click(rect.left + 3, rect.top, 80, 24);
        dialog.insert_text("語");
        assert_eq!(dialog.name, "日語本abc");
    }

    #[test]
    fn clicking_after_an_interior_caret_uses_the_displayed_cell() {
        for (column, expected) in [(3, "aXbc"), (4, "abXc")] {
            let mut dialog = dialog();
            dialog.name = "abc".into();
            dialog.cursor = 1;
            dialog.selected_all = false;
            let rect = dialog
                .elements(80, 24)
                .into_iter()
                .find(|element| element.kind == UiKind::Input)
                .unwrap()
                .rect;
            dialog.click(rect.left + column, rect.top, 80, 24);
            dialog.insert_text("X");
            assert_eq!(dialog.name, expected);
        }
    }

    #[test]
    fn long_names_scroll_to_show_the_editing_cursor() {
        let mut dialog = dialog();
        dialog.name = "日本語日本語日本語日本語日本語".into();
        dialog.cursor = dialog.name.len();
        let input = dialog
            .elements(20, 14)
            .into_iter()
            .find(|element| element.kind == UiKind::Input)
            .unwrap();
        assert!(input.text.ends_with('│'));
        assert!(UnicodeWidthStr::width(input.text.as_str()) < input.rect.width);
    }
}
