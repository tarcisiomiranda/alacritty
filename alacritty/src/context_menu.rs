use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use crate::config::{Action, BindingKey, BindingMode, UiConfig};
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
    Groups,
    Agents,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    OpenEditor,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    OpenFileManager,
}

impl MenuAction {
    fn name(self) -> &'static str {
        match self {
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
            MenuAction::Groups => "TabGroups",
            MenuAction::Agents => "Agents",
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            MenuAction::OpenEditor => "OpenEditor",
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            MenuAction::OpenFileManager => "OpenFileManager",
        }
    }
}

pub struct MenuItem {
    pub rect: UiRect,
    pub text: String,
    pub shortcut: String,
    pub action: MenuAction,
    pub enabled: bool,
}

const fn platform(other: &'static str, macos: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { macos } else { other }
}

const MENU_ITEMS: &[(MenuAction, Message, &str)] = &[
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    (MenuAction::OpenEditor, Message::OpenEditor, ""),
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    (MenuAction::OpenFileManager, Message::OpenFolder, ""),
    (MenuAction::NewTab, Message::NewTab, platform("Ctrl+Shift+T", "Cmd+T")),
    (MenuAction::Rename, Message::Rename, "F2"),
    (MenuAction::Duplicate, Message::DuplicateTab, platform("Ctrl+Shift+D", "Shift+Cmd+D")),
    (
        MenuAction::ReopenClosed,
        Message::ReopenClosedTab,
        platform("Ctrl+Shift+Alt+T", "Shift+Cmd+T"),
    ),
    (MenuAction::SplitVertical, Message::SplitVertical, platform("Ctrl+Shift+E", "Shift+Cmd+E")),
    (
        MenuAction::SplitHorizontal,
        Message::SplitHorizontal,
        platform("Ctrl+Shift+J", "Shift+Cmd+J"),
    ),
    (MenuAction::PaneZoom, Message::TogglePaneZoom, platform("Ctrl+Shift+Z", "Shift+Cmd+Z")),
    (MenuAction::Copy, Message::Copy, platform("Ctrl+Shift+C", "Cmd+C")),
    (MenuAction::Paste, Message::Paste, platform("Ctrl+Shift+V", "Cmd+V")),
    (MenuAction::Save, Message::SaveAll, platform("Ctrl+Shift+S", "Shift+Cmd+S")),
    (MenuAction::Restore, Message::Restore, platform("Ctrl+Shift+O", "Shift+Cmd+O")),
    (MenuAction::Workspaces, Message::Workspaces, platform("Ctrl+Shift+P", "Shift+Cmd+P")),
    (MenuAction::Settings, Message::Settings, platform("Ctrl+,", "Cmd+,")),
    (MenuAction::Agents, Message::Agents, ""),
    (MenuAction::Groups, Message::TabGroups, ""),
    (MenuAction::ClosePane, Message::ClosePane, platform("Ctrl+Alt+W", "Option+Cmd+W")),
    (MenuAction::Close, Message::Close, platform("Ctrl+Shift+W", "Cmd+W")),
];

const fn menu_item_index(index: usize) -> usize {
    if cfg!(any(target_os = "linux", target_os = "macos")) { index + 2 } else { index }
}

const MENU_ROWS: &[Option<usize>] = &[
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Some(0),
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Some(1),
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    None,
    Some(menu_item_index(0)),
    Some(menu_item_index(1)),
    Some(menu_item_index(2)),
    Some(menu_item_index(3)),
    None,
    Some(menu_item_index(4)),
    Some(menu_item_index(5)),
    Some(menu_item_index(6)),
    None,
    Some(menu_item_index(7)),
    Some(menu_item_index(8)),
    None,
    Some(menu_item_index(9)),
    Some(menu_item_index(10)),
    Some(menu_item_index(11)),
    Some(menu_item_index(12)),
    Some(menu_item_index(13)),
    Some(menu_item_index(14)),
    None,
    Some(MENU_ITEMS.len() - 2),
    Some(MENU_ITEMS.len() - 1),
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    editor_name: Option<String>,
    #[cfg(target_os = "linux")]
    editor_icon: Option<std::sync::Arc<crate::app_icon::AppIcon>>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    file_manager_name: Option<String>,
    #[cfg(target_os = "linux")]
    file_manager_icon: Option<std::sync::Arc<crate::app_icon::AppIcon>>,
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
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            editor_name: None,
            #[cfg(target_os = "linux")]
            editor_icon: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            file_manager_name: None,
            #[cfg(target_os = "linux")]
            file_manager_icon: None,
            column,
            line,
            scroll_offset: 0,
            hovered_scroll: None,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn update_applications(&mut self, config: &UiConfig) {
        use crate::desktop_apps::{self, AppKind, InstalledApps};
        let apps = InstalledApps::detect();
        let label = |kind, preference: &str| {
            apps.resolve(kind, preference)
                .map(|app| app.name)
                .filter(|name| !name.is_empty())
                .or_else(|| desktop_apps::name(kind, preference))
                .map(str::to_owned)
        };
        self.editor_name = label(AppKind::Editor, &config.general.editor);
        self.file_manager_name = label(AppKind::FileManager, &config.general.file_manager);
        #[cfg(target_os = "linux")]
        {
            self.editor_icon = Some(crate::app_icon::AppIcon::load(
                AppKind::Editor,
                apps.resolve(AppKind::Editor, &config.general.editor)
                    .map_or(config.general.editor.as_str(), |app| app.id),
            ));
            self.file_manager_icon = Some(crate::app_icon::AppIcon::load(
                AppKind::FileManager,
                apps.resolve(AppKind::FileManager, &config.general.file_manager)
                    .map_or(config.general.file_manager.as_str(), |app| app.id),
            ));
        }
    }

    #[cfg(target_os = "linux")]
    pub fn app_icon(&self, action: MenuAction) -> Option<&crate::app_icon::AppIcon> {
        match action {
            MenuAction::OpenEditor => self.editor_icon.as_deref(),
            MenuAction::OpenFileManager => self.file_manager_icon.as_deref(),
            _ => None,
        }
    }

    fn shortcut(&self, action: MenuAction, default: &str) -> String {
        self.shortcuts.get(action.name()).map_or_else(|| default.into(), Clone::clone)
    }

    pub fn update_shortcuts(&mut self, config: &UiConfig, mode: BindingMode) {
        let bindings = crate::settings::resolved_shortcuts(config, &config.general.shortcuts);
        self.shortcuts.clear();
        for &(item, _, default) in MENU_ITEMS {
            let name = item.name();
            let Ok(action) = toml::Value::String(name.into()).try_into::<Action>() else {
                continue;
            };
            let labels: Vec<_> = bindings
                .iter()
                .filter_map(|binding| {
                    if binding.action != action
                        || !mode.contains(binding.mode)
                        || mode.intersects(binding.notmode)
                    {
                        return None;
                    }
                    match &binding.trigger {
                        BindingKey::Keycode { key, .. } => {
                            crate::settings::shortcut_text(key, binding.mods)
                        },
                        BindingKey::Scancode(key) => Some(format!("{key:?}")),
                    }
                })
                .collect();
            let label = labels
                .iter()
                .find(|label| label.as_str() == default)
                .or_else(|| labels.first())
                .cloned()
                .unwrap_or_default();
            self.shortcuts.insert(name.into(), label);
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        let width = columns.min(50);
        let height = lines.min(MENU_ROWS.len() + 4);
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
        let header =
            if layout.height == MENU_ROWS.len() + 4 { 3 } else { usize::from(layout.height >= 8) };
        let bottom = usize::from(layout.height == MENU_ROWS.len() + 4);
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
                    text: {
                        #[cfg(any(target_os = "linux", target_os = "macos"))]
                        let name = match action {
                            MenuAction::OpenEditor => self.editor_name.as_deref(),
                            MenuAction::OpenFileManager => self.file_manager_name.as_deref(),
                            _ => None,
                        };
                        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                        let name: Option<&str> = None;
                        name.map_or_else(
                            || self.locale.text(message).into(),
                            |name| format!("{} {name}", self.locale.text(Message::OpenIn)),
                        )
                    },
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
            Key::Named(NamedKey::Home) => self.selected = Some(MENU_ITEMS[0].0),
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn desktop_actions_display_preferred_names_in_both_languages() {
        let mut config = UiConfig::default();
        config.general.editor = "zed".into();
        config.general.file_manager = "dolphin".into();
        let mut menu = ContextMenu::new(1, 0, 0);
        menu.update_applications(&config);
        for (locale, prefix) in [(Locale::En, "Open in"), (Locale::PtBr, "Abrir no")] {
            menu.locale = locale;
            let items = menu.items(100, 40);
            for (action, name) in
                [(MenuAction::OpenEditor, "Zed"), (MenuAction::OpenFileManager, "Dolphin")]
            {
                let item = items.iter().find(|item| item.action == action).unwrap();
                assert_eq!(item.text, format!("{prefix} {name}"));
                assert!(item.shortcut.is_empty());
                assert!(item.enabled);
            }
        }
    }

    #[test]
    fn menu_shortcuts_follow_bindings_modes_and_disabled_actions() {
        let mut config: UiConfig = toml::from_str(
            r#"
[keyboard]
bindings = [
    { key = "F2", mode = "~Alt", action = "RenameTab" },
    { key = "r", mods = "Control|Shift", action = "RenameTab" },
    { key = "F8", mods = "Control", action = "RestoreSession" },
    { key = "c", mods = "Control|Shift", action = "Copy" },
]
"#,
        )
        .unwrap();
        let mut menu = ContextMenu::new(0, 0, 0);
        menu.update_shortcuts(&config, BindingMode::empty());
        assert_eq!(menu.shortcut(MenuAction::Rename, ""), "F2");
        assert_eq!(menu.shortcut(MenuAction::Restore, ""), "Ctrl+F8");
        assert_eq!(menu.shortcut(MenuAction::Copy, ""), "Ctrl+Shift+C");
        menu.update_shortcuts(&config, BindingMode::ALT_SCREEN);
        assert_eq!(menu.shortcut(MenuAction::Rename, ""), "Ctrl+Shift+R");
        config.general.shortcuts.insert("RenameTab".into(), "Ctrl+F7".into());
        menu.update_shortcuts(&config, BindingMode::empty());
        assert_eq!(menu.shortcut(MenuAction::Rename, ""), "Ctrl+F7");
        config.general.shortcuts.insert("RenameTab".into(), String::new());
        menu.update_shortcuts(&config, BindingMode::empty());
        assert_eq!(menu.shortcut(MenuAction::Rename, "F2"), "");
    }

    #[test]
    fn menu_languages_keep_settings_actions_and_shortcuts_consistent() {
        let mut menu = ContextMenu::new(1, 0, 0);
        for locale in [Locale::En, Locale::PtBr] {
            menu.locale = locale;
            let items = menu.items(80, 25);
            let settings = items.iter().find(|item| item.action == MenuAction::Settings).unwrap();
            assert_eq!(settings.text, locale.text(Message::Settings));
            assert_eq!(
                menu.click(settings.rect.left, settings.rect.top, 80, 25),
                Some(MenuAction::Settings)
            );
            let copy = items.iter().find(|item| item.action == MenuAction::Copy).unwrap();
            assert_eq!(copy.shortcut, platform("Ctrl+Shift+C", "Cmd+C"));
        }
        menu.shortcuts.insert("SplitVertical".into(), "Ctrl+Alt+E".into());
        let items = menu.items(80, 25);
        assert_eq!(
            items.iter().find(|item| item.action == MenuAction::SplitVertical).unwrap().shortcut,
            "Ctrl+Alt+E"
        );
    }

    #[test]
    fn menu_groups_follow_the_same_order_for_mouse_and_keyboard() {
        let mut menu = ContextMenu::new(1, 0, 0);
        menu.copy_enabled = true;
        let items = menu.items(80, 32);
        let expected = vec![
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            MenuAction::OpenEditor,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            MenuAction::OpenFileManager,
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
            MenuAction::Agents,
            MenuAction::Groups,
            MenuAction::ClosePane,
            MenuAction::Close,
        ];
        assert_eq!(items.iter().map(|item| item.action).collect::<Vec<_>>(), expected);
        for action in expected {
            menu.move_selection(false, 80, 32);
            assert_eq!(menu.selected, Some(action));
        }
        let separators: Vec<_> = menu
            .decorations(80, 32)
            .into_iter()
            .filter(|element| element.kind == UiKind::Divider)
            .map(|element| element.rect.top)
            .collect();
        let expected_separators = if cfg!(any(target_os = "linux", target_os = "macos")) {
            vec![2, 5, 10, 14, 17, 24]
        } else {
            vec![2, 7, 11, 14, 21]
        };
        assert_eq!(separators, expected_separators);
        assert_eq!(menu.layout(80, 25).width, 50);
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
            .items(80, 25)
            .into_iter()
            .find(|item| item.action == MenuAction::Copy)
            .unwrap()
            .rect;
        assert_eq!(menu.click(copy.left, copy.top, 80, 25), None);
        assert!(!menu.hover(copy.left, copy.top, 80, 25));
        menu.copy_enabled = true;
        assert!(menu.hover(copy.left, copy.top, 80, 25));
        assert_eq!(menu.selected, Some(MenuAction::Copy));
        assert_eq!(
            menu.click(copy.left + copy.width - 1, copy.top, 80, 25),
            Some(MenuAction::Copy)
        );
    }

    #[test]
    fn hovering_header_or_separator_clears_selection_without_moving_viewport() {
        let mut menu = ContextMenu::new(1, 0, 0);
        assert!(menu.hover(2, 3, 80, 32));
        assert!(menu.hover(2, 1, 80, 32));
        assert_eq!(menu.selected, None);
        assert_eq!(menu.click(2, 1, 80, 32), None);
        let separator = if cfg!(any(target_os = "linux", target_os = "macos")) { 5 } else { 7 };
        assert_eq!(menu.click(2, separator, 80, 32), None);
        assert!(!menu.hit_scroll(2, separator, 80, 32));
        assert_eq!(menu.scroll_offset, 0);
    }

    #[test]
    fn keyboard_skips_disabled_actions_and_wraps() {
        let mut menu = ContextMenu::new(1, 0, 0);
        menu.move_selection(true, 80, 25);
        assert_eq!(menu.selected, Some(MenuAction::Close));
        menu.move_selection(false, 80, 25);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(menu.selected, Some(MenuAction::OpenEditor));
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        assert_eq!(menu.selected, Some(MenuAction::NewTab));
        menu.selected = Some(MenuAction::PaneZoom);
        menu.move_selection(false, 80, 25);
        assert_eq!(menu.selected, Some(MenuAction::Paste));
        menu.move_selection(true, 80, 25);
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
        assert_eq!(menu.items(40, 10).first().unwrap().action, MENU_ITEMS[0].0);
        assert!(menu.viewport(40, 4).up.is_none());
        assert!(menu.viewport(40, 25).up.is_none());
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
