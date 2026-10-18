use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::localization::{Locale, Message};
use crate::tab_ui::{UiElement, UiKind, UiRect};
use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Confirm,
    Cancel,
}

pub struct CloseConfirmation {
    pub tab_id: TabId,
    pub locale: Locale,
    pub pane: bool,
    title: String,
    hovered: Option<CloseAction>,
}

impl CloseConfirmation {
    pub fn new(tab_id: TabId, title: String, locale: Locale) -> Self {
        let title = title.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
        Self { tab_id, locale, pane: false, title, hovered: None }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        let width = if columns >= 12 { columns.saturating_sub(4) } else { columns }.min(64);
        let height = lines.min(12);
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

    pub fn key(&self, event: &KeyEvent, modifiers: ModifiersState) -> Option<CloseAction> {
        Self::logical_key(&event.logical_key, modifiers)
    }

    pub fn logical_key(key: &Key, modifiers: ModifiersState) -> Option<CloseAction> {
        if modifiers.control_key() || modifiers.alt_key() || modifiers.super_key() {
            return None;
        }
        match key {
            Key::Named(NamedKey::Enter) => Some(CloseAction::Confirm),
            Key::Named(NamedKey::Escape) => Some(CloseAction::Cancel),
            Key::Character(text) if text.eq_ignore_ascii_case("y") => Some(CloseAction::Confirm),
            Key::Character(text) if text.eq_ignore_ascii_case("n") => Some(CloseAction::Cancel),
            _ => None,
        }
    }

    pub fn click(
        &self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<CloseAction> {
        if !self.layout(columns, lines).contains(column, line) {
            return Some(CloseAction::Cancel);
        }
        self.control_at(column, line, columns, lines)
    }

    pub fn hover(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let hovered = self.control_at(column, line, columns, lines);
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }

    pub fn pointer_cursor(&self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        self.control_at(column, line, columns, lines).is_some()
    }

    fn control_at(
        &self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<CloseAction> {
        self.parts(columns, lines).into_iter().find_map(|(element, action)| {
            element.rect.contains(column, line).then_some(action).flatten()
        })
    }

    fn part(
        &self,
        layout: UiRect,
        position: (usize, usize, usize),
        kind: UiKind,
        text: String,
        action: Option<CloseAction>,
    ) -> (UiElement, Option<CloseAction>) {
        let (left, top, width) = position;
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
                focused: false,
                hovered: action.is_some() && self.hovered == action,
                disabled: false,
            },
            action,
        )
    }

    fn parts(&self, columns: usize, lines: usize) -> Vec<(UiElement, Option<CloseAction>)> {
        let layout = self.layout(columns, lines);
        if layout.width == 0 || layout.height == 0 {
            return Vec::new();
        }
        let inset = if layout.width >= 12 { 2 } else { 0 };
        let width = layout.width - inset * 2;
        let horizontal = width >= 28 || (layout.height == 1 && width >= 2);
        let button_rows = if horizontal { 1 } else { layout.height.min(2) };
        let help = layout.height >= 6;
        let spacious = layout.height >= 9;
        let button_row = if spacious {
            layout.height - 3 - button_rows
        } else {
            layout.height - button_rows - usize::from(help)
        };
        let content_end = button_row.saturating_sub(usize::from(spacious));
        let mut parts = Vec::new();
        let mut add = |row, kind, text: String| {
            if row < content_end {
                parts.push(self.part(layout, (inset, row, width), kind, text, None));
            }
        };
        if spacious && content_end >= 6 {
            add(
                1,
                UiKind::Title,
                self.locale
                    .text(if self.pane { Message::ClosePaneTitle } else { Message::CloseTabTitle })
                    .into(),
            );
            add(
                3,
                UiKind::Label,
                self.locale
                    .text(if self.pane {
                        Message::ClosePaneQuestion
                    } else {
                        Message::CloseTabQuestion
                    })
                    .into(),
            );
            add(4, UiKind::Command, self.title.clone());
            if content_end > 5 {
                add(
                    content_end - 1,
                    UiKind::Subtitle,
                    self.locale.text(Message::CloseTabWarning).into(),
                );
            }
        } else if content_end >= 4 {
            add(
                0,
                UiKind::Title,
                self.locale
                    .text(if self.pane { Message::ClosePaneTitle } else { Message::CloseTabTitle })
                    .into(),
            );
            add(
                1,
                UiKind::Label,
                self.locale
                    .text(if self.pane {
                        Message::ClosePaneQuestion
                    } else {
                        Message::CloseTabQuestion
                    })
                    .into(),
            );
            add(2, UiKind::Command, self.title.clone());
            add(3, UiKind::Subtitle, self.locale.text(Message::CloseTabWarning).into());
        } else {
            if content_end >= 2 {
                add(
                    0,
                    UiKind::Title,
                    self.locale
                        .text(if self.pane {
                            Message::ClosePaneTitle
                        } else {
                            Message::CloseTabTitle
                        })
                        .into(),
                );
            }
            if content_end > 0 {
                add(usize::from(content_end >= 2), UiKind::Command, self.title.clone());
            }
            if content_end >= 3 {
                add(2, UiKind::Subtitle, self.locale.text(Message::CloseTabWarning).into());
            }
        }
        if spacious {
            parts.push(self.part(
                layout,
                (inset, button_row - 1, width),
                UiKind::Divider,
                String::new(),
                None,
            ));
        }
        if horizontal {
            let gap = usize::from(width >= 28) * 2;
            let accept_width = if width >= 28 { 14 } else { width / 2 };
            let reject_width = (width - accept_width - gap).min(12);
            parts.push(
                self.part(
                    layout,
                    (inset, button_row, accept_width),
                    UiKind::DangerButton,
                    self.locale
                        .text(if self.pane { Message::ClosePane } else { Message::CloseTabAccept })
                        .into(),
                    Some(CloseAction::Confirm),
                ),
            );
            parts.push(self.part(
                layout,
                (inset + accept_width + gap, button_row, reject_width),
                UiKind::SecondaryButton,
                self.locale.text(Message::CloseTabReject).into(),
                Some(CloseAction::Cancel),
            ));
        } else {
            if button_rows == 2 {
                parts.push(
                    self.part(
                        layout,
                        (inset, button_row, width),
                        UiKind::DangerButton,
                        self.locale
                            .text(if self.pane {
                                Message::ClosePane
                            } else {
                                Message::CloseTabAccept
                            })
                            .into(),
                        Some(CloseAction::Confirm),
                    ),
                );
            }
            parts.push(self.part(
                layout,
                (inset, button_row + button_rows - 1, width),
                UiKind::SecondaryButton,
                self.locale.text(Message::CloseTabReject).into(),
                Some(CloseAction::Cancel),
            ));
        }
        if help {
            parts.push(self.part(
                layout,
                (inset, layout.height - 1 - usize::from(spacious), width),
                UiKind::Status,
                self.locale.text(Message::CloseTabKeys).into(),
                None,
            ));
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmation(locale: Locale) -> CloseConfirmation {
        CloseConfirmation::new(7, "🦀 produção 東京".into(), locale)
    }

    #[test]
    fn yes_no_and_named_keys_accept_both_letter_cases() {
        for key in
            [Key::Character("y".into()), Key::Character("Y".into()), Key::Named(NamedKey::Enter)]
        {
            assert_eq!(
                CloseConfirmation::logical_key(&key, ModifiersState::empty()),
                Some(CloseAction::Confirm)
            );
        }
        for key in
            [Key::Character("n".into()), Key::Character("N".into()), Key::Named(NamedKey::Escape)]
        {
            assert_eq!(
                CloseConfirmation::logical_key(&key, ModifiersState::empty()),
                Some(CloseAction::Cancel)
            );
        }
        assert_eq!(
            CloseConfirmation::logical_key(&Key::Character("yes".into()), ModifiersState::empty()),
            None
        );
        assert_eq!(
            CloseConfirmation::logical_key(&Key::Named(NamedKey::Space), ModifiersState::empty()),
            None
        );
    }

    #[test]
    fn command_modifiers_are_ignored_and_shift_allows_uppercase() {
        for modifiers in [
            ModifiersState::CONTROL,
            ModifiersState::ALT,
            ModifiersState::SUPER,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        ] {
            for key in [
                Key::Character("y".into()),
                Key::Character("n".into()),
                Key::Named(NamedKey::Enter),
                Key::Named(NamedKey::Escape),
            ] {
                assert_eq!(CloseConfirmation::logical_key(&key, modifiers), None);
            }
        }
        assert_eq!(
            CloseConfirmation::logical_key(&Key::Character("Y".into()), ModifiersState::SHIFT),
            Some(CloseAction::Confirm)
        );
    }

    #[test]
    fn mouse_actions_hover_and_cursor_share_visible_rectangles() {
        let mut dialog = confirmation(Locale::En);
        for (element, action) in dialog.parts(100, 40) {
            if let Some(action) = action {
                for column in element.rect.left..element.rect.left + element.rect.width {
                    assert_eq!(dialog.click(column, element.rect.top, 100, 40), Some(action));
                    assert!(dialog.pointer_cursor(column, element.rect.top, 100, 40));
                }
                assert!(dialog.hover(element.rect.left, element.rect.top, 100, 40));
                assert!(!dialog.hover(element.rect.left, element.rect.top, 100, 40));
                assert!(
                    dialog
                        .elements(100, 40)
                        .iter()
                        .any(|part| part.rect == element.rect && part.hovered)
                );
            }
        }
        let panel = dialog.layout(100, 40);
        assert_eq!(dialog.click(panel.left, panel.top, 100, 40), None);
        assert!(!dialog.pointer_cursor(panel.left, panel.top, 100, 40));
        assert_eq!(dialog.click(0, 0, 100, 40), Some(CloseAction::Cancel));
        assert_eq!(
            dialog.click(panel.left + panel.width, panel.top, 100, 40),
            Some(CloseAction::Cancel)
        );
        assert!(dialog.hover(0, 0, 100, 40));
    }

    #[test]
    fn all_responsive_rectangles_fit_and_do_not_overlap() {
        for locale in [Locale::En, Locale::PtBr] {
            let dialog = confirmation(locale);
            for columns in 0..100 {
                for lines in 0..40 {
                    let panel = dialog.layout(columns, lines);
                    assert!(panel.left + panel.width <= columns);
                    assert!(panel.top + panel.height <= lines);
                    assert!(panel.width <= 64 && panel.height <= 12);
                    let parts = dialog.parts(columns, lines);
                    for (index, (element, action)) in parts.iter().enumerate() {
                        assert!(element.rect.width > 0 && element.rect.height == 1);
                        assert!(element.rect.left >= panel.left && element.rect.top >= panel.top);
                        assert!(element.rect.left + element.rect.width <= panel.left + panel.width);
                        assert!(element.rect.top + element.rect.height <= panel.top + panel.height);
                        for (other, _) in &parts[index + 1..] {
                            assert!(
                                element.rect.top != other.rect.top
                                    || element.rect.left + element.rect.width <= other.rect.left
                                    || other.rect.left + other.rect.width <= element.rect.left
                            );
                        }
                        if let Some(action) = action {
                            assert_eq!(
                                dialog.click(element.rect.left, element.rect.top, columns, lines),
                                Some(*action)
                            );
                        }
                    }
                    if columns > 0 && lines >= 3 {
                        assert!(parts.iter().any(|(element, _)| {
                            element.kind == UiKind::Command && element.text == dialog.title
                        }));
                    }
                    if columns > 0 && lines > 0 {
                        assert!(
                            parts.iter().any(|(_, action)| *action == Some(CloseAction::Cancel))
                        );
                    }
                    if columns > 0 && lines >= 2 || columns >= 2 && lines == 1 {
                        assert!(
                            parts.iter().any(|(_, action)| *action == Some(CloseAction::Confirm))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unicode_title_is_preserved_and_controls_are_replaced() {
        let title = format!("🦀 produção\n東京\t\x1b{}", "界".repeat(100));
        let dialog = CloseConfirmation::new(42, title, Locale::PtBr);
        assert_eq!(dialog.tab_id, 42);
        assert!(!dialog.title.chars().any(char::is_control));
        assert_eq!(dialog.title, format!("🦀 produção 東京  {}", "界".repeat(100)));
        let elements = dialog.elements(100, 40);
        assert!(
            elements
                .iter()
                .any(|element| element.kind == UiKind::Command && element.text == dialog.title)
        );
        assert!(elements.iter().any(|element| element.text == "Fechar aba"));
        assert!(elements.iter().any(|element| element.text == "Cancelar"));
    }
}
