use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use crate::localization::{Locale, Message};
use crate::tab_ui::{UiElement, UiKind, UiRect};
use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewTab,
    Rename,
    Save,
    Restore,
    Copy,
    Paste,
    Close,
    Settings,
    Duplicate,
    ReopenClosed,
    Workspaces,
    SplitVertical,
    SplitHorizontal,
    PaneZoom,
    ClosePane,
}

pub struct MenuItem {
    pub rect: UiRect,
    pub text: String,
    pub shortcut: String,
    pub action: MenuAction,
    pub enabled: bool,
}

const MENU_ITEMS: [(MenuAction, Message, &str); 15] = [
    (MenuAction::NewTab, Message::NewTab, "Ctrl+Shift+T"),
    (MenuAction::Rename, Message::Rename, "Ctrl+Shift+R"),
    (MenuAction::Duplicate, Message::DuplicateTab, "Ctrl+Shift+D"),
    (MenuAction::ReopenClosed, Message::ReopenClosedTab, "Ctrl+Shift+Alt+T"),
    (MenuAction::SplitVertical, Message::SplitVertical, "Ctrl+Shift+E"),
    (MenuAction::SplitHorizontal, Message::SplitHorizontal, "Ctrl+Shift+J"),
    (MenuAction::PaneZoom, Message::TogglePaneZoom, "Ctrl+Shift+Z"),
    (MenuAction::Copy, Message::Copy, "Ctrl+Shift+C"),
    (MenuAction::Paste, Message::Paste, "Ctrl+Shift+V"),
    (MenuAction::Save, Message::SaveAll, "Ctrl+Shift+S"),
    (MenuAction::Restore, Message::Restore, "Ctrl+Shift+O"),
    (MenuAction::Workspaces, Message::Workspaces, "Ctrl+Shift+P"),
    (MenuAction::Settings, Message::Settings, "Ctrl+,"),
    (MenuAction::ClosePane, Message::ClosePane, "Ctrl+Alt+W"),
    (MenuAction::Close, Message::Close, "Ctrl+Shift+W"),
];

const MENU_ROWS: [Option<usize>; 19] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    None,
    Some(4),
    Some(5),
    Some(6),
    None,
    Some(7),
    Some(8),
    None,
    Some(9),
    Some(10),
    Some(11),
    Some(12),
    None,
    Some(13),
    Some(14),
];

struct MenuViewport {
    layout: UiRect,
    header: usize,
    body_top: usize,
    capacity: usize,
    offset: usize,
    up: Option<UiRect>,
    down: Option<UiRect>,
}

pub struct ContextMenu {
    pub tab_id: TabId,
    pub locale: Locale,
    pub selected: Option<MenuAction>,
    pub copy_enabled: bool,
    pub shortcuts: std::collections::BTreeMap<String, String>,
    column: usize,
    line: usize,
    scroll_offset: usize,
    hovered_scroll: Option<bool>,
}

impl ContextMenu {
    pub fn new(tab_id: TabId, column: usize, line: usize) -> Self {
        Self {
            tab_id,
            locale: Locale::En,
            selected: None,
            copy_enabled: false,
            shortcuts: Default::default(),
            column,
            line,
            scroll_offset: 0,
            hovered_scroll: None,
        }
    }

    fn shortcut(&self, action: MenuAction, default: &str) -> String {
        let name = match action {
            MenuAction::NewTab => "CreateNewTab",
            MenuAction::Rename => "RenameTab",
            MenuAction::Save => "SaveSession",
            MenuAction::Restore => "RestoreSession",
            MenuAction::Copy => "Copy",
            MenuAction::Paste => "Paste",
            MenuAction::Close => "CloseTab",
            MenuAction::Settings => "OpenSettings",
            MenuAction::Duplicate => "DuplicateTab",
            MenuAction::ReopenClosed => "ReopenClosedTab",
            MenuAction::Workspaces => "OpenWorkspaces",
            MenuAction::SplitVertical => "SplitVertical",
            MenuAction::SplitHorizontal => "SplitHorizontal",
            MenuAction::PaneZoom => "TogglePaneZoom",
            MenuAction::ClosePane => "ClosePane",
        };
        self.shortcuts
            .get(name)
            .filter(|value| {
                value.is_empty() || crate::settings::shortcut_binding(name, value).is_some()
            })
            .map_or_else(|| default.into(), Clone::clone)
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        let width = columns.min(50);
        let height = lines.min(23);
        UiRect {
            left: self.column.min(columns.saturating_sub(width)),
            top: self.line.min(lines.saturating_sub(height)),
            width,
            height,
        }
    }

    fn row_rect(layout: UiRect, row: usize) -> UiRect {
        let inset = usize::from(layout.width >= 4);
        UiRect {
            left: layout.left + inset,
            top: layout.top + row,
            width: layout.width.saturating_sub(inset * 2),
            height: 1,
        }
    }

    fn viewport(&self, columns: usize, lines: usize) -> MenuViewport {
        let layout = self.layout(columns, lines);
        let header = if layout.height == 23 { 3 } else { usize::from(layout.height >= 8) };
        let bottom = usize::from(layout.height == 23);
        let available = layout.height.saturating_sub(header + bottom);
        let overflow = MENU_ROWS.len() > available;
        let arrows = overflow && layout.height >= 5 && layout.width != 0;
        let capacity = if layout.width == 0 {
            0
        } else {
            available.saturating_sub(usize::from(arrows) * 2).min(MENU_ROWS.len())
        };
        let body_top = header + usize::from(arrows);
        let offset = self.scroll_offset.min(MENU_ROWS.len().saturating_sub(capacity));
        MenuViewport {
            layout,
            header,
            body_top,
            capacity,
            offset,
            up: arrows.then(|| Self::row_rect(layout, header)),
            down: arrows.then(|| Self::row_rect(layout, body_top + capacity)),
        }
    }

    pub fn ensure_visible(&mut self, columns: usize, lines: usize) {
        let viewport = self.viewport(columns, lines);
        self.scroll_offset = viewport.offset;
        if viewport.capacity == 0 {
            return;
        }
        let selected = self.selected.and_then(|action| {
            MENU_ROWS.iter().position(|row| row.is_some_and(|index| MENU_ITEMS[index].0 == action))
        });
        if let Some(row) = selected {
            if row < self.scroll_offset {
                self.scroll_offset = row;
            } else if row >= self.scroll_offset + viewport.capacity {
                self.scroll_offset = row + 1 - viewport.capacity;
            }
        }
    }

    pub fn decorations(&self, columns: usize, lines: usize) -> Vec<UiElement> {
        let viewport = self.viewport(columns, lines);
        if viewport.layout.width == 0 || viewport.layout.height == 0 {
            return Vec::new();
        }
        let mut elements = Vec::new();
        let mut add = |rect, kind, text, hovered, disabled| {
            elements.push(UiElement { rect, kind, text, focused: false, hovered, disabled })
        };
        if viewport.header != 0 {
            let mut rect = Self::row_rect(viewport.layout, usize::from(viewport.header == 3));
            if rect.width >= 4 {
                rect.left += 1;
                rect.width -= 2;
            }
            add(rect, UiKind::Label, self.locale.text(Message::MenuHeading).into(), false, false);
            if viewport.header == 3 {
                add(
                    Self::row_rect(viewport.layout, 2),
                    UiKind::Divider,
                    String::new(),
                    false,
                    false,
                );
            }
        }
        for (row, item) in
            MENU_ROWS.iter().skip(viewport.offset).take(viewport.capacity).enumerate()
        {
            if item.is_none() {
                add(
                    Self::row_rect(viewport.layout, viewport.body_top + row),
                    UiKind::Divider,
                    String::new(),
                    false,
                    false,
                );
            }
        }
        if let Some(rect) = viewport.up {
            add(
                rect,
                UiKind::MenuItem,
                format!("{:^width$}", "↑", width = rect.width),
                self.hovered_scroll == Some(false),
                viewport.offset == 0,
            );
        }
        if let Some(rect) = viewport.down {
            add(
                rect,
                UiKind::MenuItem,
                format!("{:^width$}", "↓", width = rect.width),
                self.hovered_scroll == Some(true),
                viewport.offset + viewport.capacity >= MENU_ROWS.len(),
            );
        }
        elements
    }

    pub fn scroll(&mut self, delta: isize, columns: usize, lines: usize) -> bool {
        let viewport = self.viewport(columns, lines);
        if viewport.capacity == 0 || delta == 0 {
            return false;
        }
        let maximum = MENU_ROWS.len().saturating_sub(viewport.capacity);
        let mut next = viewport.offset.saturating_add_signed(delta).min(maximum);
        if viewport.capacity == 1 && MENU_ROWS[next].is_none() {
            next = if delta > 0 { (next + 1).min(maximum) } else { next.saturating_sub(1) };
        }
        let changed = next != viewport.offset;
        self.scroll_offset = next;
        if changed {
            self.selected = None;
        }
        changed
    }

    pub fn hit_scroll(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let viewport = self.viewport(columns, lines);
        if viewport.up.is_some_and(|rect| rect.contains(column, line)) {
            self.scroll(-1, columns, lines);
            true
        } else if viewport.down.is_some_and(|rect| rect.contains(column, line)) {
            self.scroll(1, columns, lines);
            true
        } else {
            false
        }
    }

    pub fn items(&self, columns: usize, lines: usize) -> Vec<MenuItem> {
        let viewport = self.viewport(columns, lines);
        MENU_ROWS
            .iter()
            .skip(viewport.offset)
            .take(viewport.capacity)
            .enumerate()
            .filter_map(|(row, index)| {
                let (action, message, shortcut) = MENU_ITEMS[(*index)?];
                Some(MenuItem {
                    rect: Self::row_rect(viewport.layout, viewport.body_top + row),
                    text: self.locale.text(message).into(),
                    shortcut: self.shortcut(action, shortcut),
                    action,
                    enabled: action != MenuAction::Copy || self.copy_enabled,
                })
            })
            .collect()
    }

    pub fn hover(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let selected = self
            .items(columns, lines)
            .into_iter()
            .find(|item| item.enabled && item.rect.contains(column, line))
            .map(|item| item.action);
        let viewport = self.viewport(columns, lines);
        let hovered_scroll = if viewport.up.is_some_and(|rect| rect.contains(column, line)) {
            Some(false)
        } else if viewport.down.is_some_and(|rect| rect.contains(column, line)) {
            Some(true)
        } else {
            None
        };
        let changed = self.selected != selected || self.hovered_scroll != hovered_scroll;
        self.selected = selected;
        self.hovered_scroll = hovered_scroll;
        changed
    }

    pub fn click(
        &self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<MenuAction> {
        self.items(columns, lines)
            .into_iter()
            .find(|item| item.enabled && item.rect.contains(column, line))
            .map(|item| item.action)
    }

    pub fn key(&mut self, event: &KeyEvent, columns: usize, lines: usize) -> Option<MenuAction> {
        self.hovered_scroll = None;
        match event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.move_selection(false, columns, lines),
            Key::Named(NamedKey::ArrowUp) => self.move_selection(true, columns, lines),
            Key::Named(NamedKey::Home) => self.selected = Some(MenuAction::NewTab),
            Key::Named(NamedKey::End) => self.selected = Some(MenuAction::Close),
            Key::Named(NamedKey::PageDown | NamedKey::PageUp) => {
                let previous = event.logical_key == Key::Named(NamedKey::PageUp);
                let selected = self
                    .selected
                    .and_then(|action| MENU_ITEMS.iter().position(|item| item.0 == action));
                let index = selected.unwrap_or(if previous { MENU_ITEMS.len() - 1 } else { 0 });
                let count = self.viewport(columns, lines).capacity.max(1);
                let mut index = if previous {
                    index.saturating_sub(count)
                } else {
                    (index + count).min(MENU_ITEMS.len() - 1)
                };
                if MENU_ITEMS[index].0 == MenuAction::Copy && !self.copy_enabled {
                    index = if previous { index - 1 } else { index + 1 };
                }
                self.selected = Some(MENU_ITEMS[index].0);
            },
            Key::Named(NamedKey::Enter) => {
                return self
                    .selected
                    .filter(|action| *action != MenuAction::Copy || self.copy_enabled);
            },
            _ => (),
        }
        self.ensure_visible(columns, lines);
        None
    }

    fn move_selection(&mut self, previous: bool, columns: usize, lines: usize) {
        let selected = self
            .selected
            .and_then(|selected| MENU_ITEMS.iter().position(|item| item.0 == selected));
        let mut index = match selected {
            Some(index) if previous => (index + MENU_ITEMS.len() - 1) % MENU_ITEMS.len(),
            Some(index) => (index + 1) % MENU_ITEMS.len(),
            None if previous => MENU_ITEMS.len() - 1,
            None => 0,
        };
        if MENU_ITEMS[index].0 == MenuAction::Copy && !self.copy_enabled {
            index = if previous { index - 1 } else { index + 1 };
        }
        self.selected = Some(MENU_ITEMS[index].0);
        self.ensure_visible(columns, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_languages_keep_settings_actions_and_shortcuts_consistent() {
        let mut menu = ContextMenu::new(1, 0, 0);
        for locale in [Locale::En, Locale::PtBr] {
            menu.locale = locale;
            let items = menu.items(80, 24);
            let settings = items.iter().find(|item| item.action == MenuAction::Settings).unwrap();
            assert_eq!(settings.text, locale.text(Message::Settings));
            assert_eq!(
                menu.click(settings.rect.left, settings.rect.top, 80, 24),
                Some(MenuAction::Settings)
            );
            let copy = items.iter().find(|item| item.action == MenuAction::Copy).unwrap();
            assert_eq!(copy.shortcut, "Ctrl+Shift+C");
        }
        menu.shortcuts.insert("SplitVertical".into(), "Ctrl+Alt+E".into());
        let items = menu.items(80, 24);
        assert_eq!(
            items.iter().find(|item| item.action == MenuAction::SplitVertical).unwrap().shortcut,
            "Ctrl+Alt+E"
        );
    }

    #[test]
    fn menu_groups_follow_the_same_order_for_mouse_and_keyboard() {
        let mut menu = ContextMenu::new(1, 0, 0);
        menu.copy_enabled = true;
        let items = menu.items(80, 24);
        let expected = vec![
            MenuAction::NewTab,
            MenuAction::Rename,
            MenuAction::Duplicate,
            MenuAction::ReopenClosed,
            MenuAction::SplitVertical,
            MenuAction::SplitHorizontal,
            MenuAction::PaneZoom,
            MenuAction::Copy,
            MenuAction::Paste,
            MenuAction::Save,
            MenuAction::Restore,
            MenuAction::Workspaces,
            MenuAction::Settings,
            MenuAction::ClosePane,
            MenuAction::Close,
        ];
        assert_eq!(items.iter().map(|item| item.action).collect::<Vec<_>>(), expected);
        for action in expected {
            menu.move_selection(false, 80, 24);
            assert_eq!(menu.selected, Some(action));
        }
        let separators: Vec<_> = menu
            .decorations(80, 24)
            .into_iter()
            .filter(|element| element.kind == UiKind::Divider)
            .map(|element| element.rect.top)
            .collect();
        assert_eq!(separators, vec![2, 7, 11, 14, 19]);
        assert_eq!(menu.layout(80, 24).width, 50);
    }

    #[test]
    fn all_viewports_keep_items_and_decorations_bounded_and_nonoverlapping() {
        for columns in 0..60 {
            for lines in 0..30 {
                let mut menu = ContextMenu::new(1, usize::MAX, usize::MAX);
                for delta in [0, 3, isize::MAX, isize::MIN] {
                    menu.scroll(delta, columns, lines);
                    let rect = menu.layout(columns, lines);
                    assert!(rect.left + rect.width <= columns);
                    assert!(rect.top + rect.height <= lines);
                    let mut occupied = vec![false; columns * lines];
                    let items = menu.items(columns, lines);
                    let decorations = menu.decorations(columns, lines);
                    for item in items
                        .iter()
                        .map(|item| item.rect)
                        .chain(decorations.iter().map(|element| element.rect))
                    {
                        assert!(rect.contains(item.left, item.top));
                        assert!(item.left + item.width <= columns);
                        assert!(item.top + item.height <= lines);
                        for y in item.top..item.top + item.height {
                            for x in item.left..item.left + item.width {
                                assert!(!occupied[y * columns + x]);
                                occupied[y * columns + x] = true;
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn copy_is_disabled_until_a_selection_exists() {
        let mut menu = ContextMenu::new(1, 0, 0);
        let copy = menu
            .items(80, 24)
            .into_iter()
            .find(|item| item.action == MenuAction::Copy)
            .unwrap()
            .rect;
        assert_eq!(menu.click(copy.left, copy.top, 80, 24), None);
        assert!(!menu.hover(copy.left, copy.top, 80, 24));
        menu.copy_enabled = true;
        assert!(menu.hover(copy.left, copy.top, 80, 24));
        assert_eq!(menu.selected, Some(MenuAction::Copy));
        assert_eq!(
            menu.click(copy.left + copy.width - 1, copy.top, 80, 24),
            Some(MenuAction::Copy)
        );
    }

    #[test]
    fn hovering_header_or_separator_clears_selection_without_moving_viewport() {
        let mut menu = ContextMenu::new(1, 0, 0);
        assert!(menu.hover(2, 3, 80, 24));
        assert!(menu.hover(2, 1, 80, 24));
        assert_eq!(menu.selected, None);
        assert_eq!(menu.click(2, 1, 80, 24), None);
        assert_eq!(menu.click(2, 7, 80, 24), None);
        assert!(!menu.hit_scroll(2, 7, 80, 24));
        assert_eq!(menu.scroll_offset, 0);
    }

    #[test]
    fn keyboard_skips_disabled_actions_and_wraps() {
        let mut menu = ContextMenu::new(1, 0, 0);
        menu.move_selection(true, 80, 24);
        assert_eq!(menu.selected, Some(MenuAction::Close));
        menu.move_selection(false, 80, 24);
        assert_eq!(menu.selected, Some(MenuAction::NewTab));
        menu.selected = Some(MenuAction::PaneZoom);
        menu.move_selection(false, 80, 24);
        assert_eq!(menu.selected, Some(MenuAction::Paste));
        menu.move_selection(true, 80, 24);
        assert_eq!(menu.selected, Some(MenuAction::PaneZoom));
    }

    #[test]
    fn keyboard_selection_stays_visible_at_all_menu_heights() {
        for height in 1..=24 {
            for previous in [false, true] {
                let mut menu = ContextMenu::new(1, 0, 0);
                for _ in 0..30 {
                    menu.move_selection(previous, 40, height);
                    assert!(
                        menu.items(40, height)
                            .iter()
                            .any(|item| Some(item.action) == menu.selected)
                    );
                }
            }
        }
    }

    #[test]
    fn scrolling_and_hovering_preserve_a_contiguous_stable_viewport() {
        let mut menu = ContextMenu::new(1, 0, 0);
        assert!(menu.scroll(5, 40, 10));
        let before: Vec<_> =
            menu.items(40, 10).into_iter().map(|item| (item.action, item.rect)).collect();
        for (action, rect) in &before {
            if *action == MenuAction::Copy {
                continue;
            }
            menu.hover(rect.left, rect.top, 40, 10);
            let after: Vec<_> =
                menu.items(40, 10).into_iter().map(|item| (item.action, item.rect)).collect();
            assert_eq!(before, after);
        }
        let expected: Vec<_> = MENU_ROWS
            .iter()
            .skip(5)
            .take(menu.viewport(40, 10).capacity)
            .filter_map(|index| index.map(|index| MENU_ITEMS[index].0))
            .collect();
        assert_eq!(before.iter().map(|(action, _)| *action).collect::<Vec<_>>(), expected);
        assert!(menu.scroll(1, 40, 10));
        assert_eq!(menu.selected, None);
    }

    #[test]
    fn overflow_arrows_scroll_and_are_consumed_even_at_limits() {
        let mut menu = ContextMenu::new(1, 0, 0);
        let viewport = menu.viewport(40, 10);
        let up = viewport.up.unwrap();
        let down = viewport.down.unwrap();
        assert!(menu.hit_scroll(up.left, up.top, 40, 10));
        assert_eq!(menu.scroll_offset, 0);
        assert!(menu.hit_scroll(down.left, down.top, 40, 10));
        assert_eq!(menu.scroll_offset, 1);
        assert!(menu.scroll(isize::MAX, 40, 10));
        assert_eq!(menu.items(40, 10).last().unwrap().action, MenuAction::Close);
        let offset = menu.scroll_offset;
        assert!(menu.hit_scroll(down.left, down.top, 40, 10));
        assert_eq!(menu.scroll_offset, offset);
        assert!(!menu.scroll(1, 40, 10));
        assert!(menu.scroll(isize::MIN, 40, 10));
        assert_eq!(menu.items(40, 10).first().unwrap().action, MenuAction::NewTab);
        assert!(menu.viewport(40, 4).up.is_none());
        assert!(menu.viewport(40, 24).up.is_none());
    }

    #[test]
    fn one_line_scroll_skips_separators_and_keyboard_selection_survives_resize() {
        let mut menu = ContextMenu::new(1, 0, 0);
        for _ in 0..30 {
            menu.scroll(1, 20, 1);
            assert_eq!(menu.items(20, 1).len(), 1);
        }
        for _ in 0..30 {
            menu.scroll(-1, 20, 1);
            assert_eq!(menu.items(20, 1).len(), 1);
        }
        menu.selected = Some(MenuAction::Close);
        for height in [24, 3, 18, 1, 0, 5, 24] {
            menu.ensure_visible(20, height);
            if height != 0 {
                assert!(menu.items(20, height).iter().any(|item| item.action == MenuAction::Close));
            }
        }
    }
}
