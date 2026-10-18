use std::cell::Cell;
use std::rc::Rc;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::config::UiConfig;
use crate::localization::{Language, Locale, Message};
use crate::settings::{
    SettingsValues, ThemeChoice, default_shortcuts, resolved_shortcuts, shortcut_binding,
    shortcut_conflict, shortcut_text,
};
use crate::tab_ui::{UiElement, UiKind, UiRect};

#[derive(Clone, Debug, PartialEq)]
pub enum ProductivityAction {
    SaveSettings(SettingsValues),
    SaveWorkspace(String),
    OpenWorkspace(String),
    RemoveWorkspace(String),
    StartupWorkspace(String),
    Cancel,
}

struct Control {
    id: usize,
    kind: UiKind,
    text: String,
    disabled: bool,
}

impl Control {
    fn button(id: usize, text: impl Into<String>) -> Self {
        Self { id, kind: UiKind::SecondaryButton, text: text.into(), disabled: false }
    }
}

enum Row {
    Label(String),
    One(Control),
    Two(Control, Control),
}

#[derive(Default)]
struct Panel {
    focus: usize,
    hovered: Option<usize>,
    offset: Cell<usize>,
    last_focus: Cell<Option<usize>>,
    viewport: Cell<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct PanelGeometry {
    rect: UiRect,
    body: UiRect,
    footer: UiRect,
    navigation_rows: usize,
    footer_rows: usize,
    control_height: usize,
    wide: bool,
}

impl Panel {
    const SCROLL_DOWN: usize = usize::MAX;
    const SCROLL_UP: usize = usize::MAX - 1;

    fn wide(columns: usize, lines: usize) -> bool {
        columns >= 90 && lines >= 22
    }

    fn width(columns: usize) -> usize {
        columns.saturating_sub(if columns >= 12 { 4 } else { 0 }).min(104)
    }

    fn content_width(columns: usize, lines: usize, settings: bool) -> usize {
        let width = Self::width(columns);
        width.saturating_sub(
            usize::from(width >= 6) * 4
                + 2
                + usize::from(settings && Self::wide(columns, lines)) * 23,
        )
    }

    fn labels(text: &str, width: usize) -> Vec<Row> {
        if width == 0 {
            return Vec::new();
        }
        let mut rows = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            if !line.is_empty() && line.width() + word.width() + 1 > width {
                rows.push(Row::Label(std::mem::take(&mut line)));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            rows.push(Row::Label(line));
        }
        rows
    }

    fn controls(rows: &[Row]) -> Vec<&Control> {
        rows.iter()
            .flat_map(|row| match row {
                Row::Label(_) => Vec::new(),
                Row::One(control) => vec![control],
                Row::Two(left, right) => vec![left, right],
            })
            .collect()
    }

    fn geometry(rows: &[Row], columns: usize, lines: usize) -> PanelGeometry {
        let settings = Self::controls(rows).first().is_some_and(|control| {
            matches!(control.kind, UiKind::Navigation | UiKind::SelectedNavigation)
        });
        let wide = settings && Self::wide(columns, lines);
        let control_height = if lines >= 26 { 2 } else { 1 };
        let width = Self::width(columns);
        let navigation_rows = if settings { 3 } else { 0 };
        let footer_rows = if settings { 1 } else { 3 }.min(rows.len());
        let desired = if settings {
            30
        } else {
            rows.iter().map(|row| Self::row_height(row, control_height)).sum::<usize>() + 8
        };
        let available = lines.saturating_sub(usize::from(lines >= 22) * 2);
        let height = desired.min(available);
        let rect = UiRect {
            left: columns.saturating_sub(width) / 2,
            top: lines.saturating_sub(height) / 2,
            width,
            height,
        };
        let inset = usize::from(width >= 6) * 2;
        let inner_width = width.saturating_sub(inset * 2);
        let footer_height = footer_rows * control_height;
        let footer_top = height.saturating_sub(footer_height + 2);
        let body_top = if settings && !wide { 7 } else { 4 };
        let body_left = inset + usize::from(wide) * 23;
        let body = UiRect {
            left: rect.left + body_left,
            top: rect.top + body_top,
            width: inner_width.saturating_sub(usize::from(wide) * 23 + 2),
            height: footer_top.saturating_sub(body_top + 1),
        };
        let footer = UiRect {
            left: rect.left + inset,
            top: rect.top + footer_top,
            width: inner_width,
            height: footer_height,
        };
        PanelGeometry { rect, body, footer, navigation_rows, footer_rows, control_height, wide }
    }

    fn layout(rows: &[Row], columns: usize, lines: usize) -> UiRect {
        Self::geometry(rows, columns, lines).rect
    }

    fn row_height(row: &Row, control_height: usize) -> usize {
        if matches!(row, Row::Label(_)) { 1 } else { control_height }
    }

    fn move_focus(&mut self, rows: &[Row], previous: bool) {
        let controls: Vec<_> = Self::controls(rows).into_iter().filter(|c| !c.disabled).collect();
        if controls.is_empty() {
            return;
        }
        let current = controls.iter().position(|c| c.id == self.focus);
        let index = match current {
            Some(index) if previous => (index + controls.len() - 1) % controls.len(),
            Some(index) => (index + 1) % controls.len(),
            None if previous => controls.len() - 1,
            None => 0,
        };
        self.focus = controls[index].id;
        self.last_focus.set(None);
    }

    fn row_parts(&self, row: &Row, rect: UiRect) -> Vec<(UiElement, Option<usize>)> {
        let element = |control: &Control, rect| {
            (
                UiElement {
                    rect,
                    kind: control.kind,
                    text: control.text.clone(),
                    focused: control.id == self.focus,
                    hovered: Some(control.id) == self.hovered,
                    disabled: control.disabled,
                },
                Some(control.id),
            )
        };
        match row {
            Row::Label(text) => vec![(
                UiElement {
                    rect,
                    kind: UiKind::Label,
                    text: text.clone(),
                    focused: false,
                    hovered: false,
                    disabled: false,
                },
                None,
            )],
            Row::One(control) => vec![element(control, rect)],
            Row::Two(left, right) if rect.width < 12 => {
                vec![element(if right.id == self.focus { right } else { left }, rect)]
            },
            Row::Two(left, right) => {
                let left_width = rect.width.saturating_sub(2) / 2;
                vec![
                    element(left, UiRect { width: left_width, ..rect }),
                    element(right, UiRect {
                        left: rect.left + left_width + 2,
                        width: rect.width.saturating_sub(left_width + 2),
                        ..rect
                    }),
                ]
            },
        }
    }

    fn offset(&self, body: &[Row], geometry: PanelGeometry) -> (usize, usize) {
        let mut total = 0;
        let mut focused = None;
        for row in body {
            let height = Self::row_height(row, geometry.control_height);
            if Self::controls(std::slice::from_ref(row)).iter().any(|c| c.id == self.focus) {
                focused = Some((total, height));
            }
            total += height;
        }
        let maximum = total.saturating_sub(geometry.body.height);
        let mut offset = self.offset.get().min(maximum);
        if self.last_focus.get() != Some(self.focus) {
            if let Some((top, height)) = focused {
                if top < offset {
                    offset = top;
                } else if top + height > offset + geometry.body.height {
                    offset = (top + height).saturating_sub(geometry.body.height);
                }
            }
        }
        self.offset.set(offset.min(maximum));
        self.last_focus.set(Some(self.focus));
        (self.offset.get(), maximum)
    }

    fn scroll(&self, rows: &[Row], delta: isize, columns: usize, lines: usize) -> bool {
        let geometry = Self::geometry(rows, columns, lines);
        let body = &rows[geometry.navigation_rows..rows.len().saturating_sub(geometry.footer_rows)];
        let (offset, maximum) = self.offset(body, geometry);
        let next = offset.saturating_add_signed(delta).min(maximum);
        self.offset.set(next);
        self.last_focus.set(Some(self.focus));
        next != offset
    }

    fn parts(
        &self,
        rows: &[Row],
        title: &str,
        subtitle: &str,
        footer_text: &str,
        columns: usize,
        lines: usize,
    ) -> Vec<(UiElement, Option<usize>)> {
        if self.viewport.replace((columns, lines)) != (columns, lines) {
            self.last_focus.set(None);
        }
        let geometry = Self::geometry(rows, columns, lines);
        let layout = geometry.rect;
        if layout.width == 0 || layout.height == 0 {
            return Vec::new();
        }
        if columns < 24 || lines < 12 {
            return Self::controls(rows)
                .into_iter()
                .find(|control| control.id == self.focus)
                .map(|control| {
                    vec![(
                        UiElement {
                            rect: layout,
                            kind: control.kind,
                            text: control.text.clone(),
                            focused: true,
                            hovered: false,
                            disabled: control.disabled,
                        },
                        Some(control.id),
                    )]
                })
                .unwrap_or_default();
        }
        let inset = usize::from(layout.width >= 6) * 2;
        let inner_width = layout.width.saturating_sub(inset * 2);
        let mut parts = Vec::new();
        for (top, kind, text) in [
            (layout.top + 1, UiKind::Title, title),
            (layout.top + 2, UiKind::Subtitle, subtitle),
            (layout.top + layout.height - 1, UiKind::Status, footer_text),
        ] {
            parts.push((
                UiElement {
                    rect: UiRect { left: layout.left + inset, top, width: inner_width, height: 1 },
                    kind,
                    text: text.into(),
                    focused: false,
                    hovered: false,
                    disabled: false,
                },
                None,
            ));
        }
        if geometry.navigation_rows > 0 {
            for (index, control) in
                Self::controls(&rows[..geometry.navigation_rows]).into_iter().enumerate()
            {
                let rect = if geometry.wide {
                    UiRect {
                        left: layout.left + inset,
                        top: geometry.body.top + index * geometry.control_height,
                        width: 20,
                        height: geometry.control_height,
                    }
                } else {
                    let available = inner_width.saturating_sub(2);
                    let column = index % 3;
                    let width = available / 3;
                    let extra = available % 3;
                    UiRect {
                        left: layout.left + inset + column * (width + 1) + column.min(extra),
                        top: layout.top + 4 + index / 3,
                        width: width + usize::from(column < extra),
                        height: 1,
                    }
                };
                parts.push((
                    UiElement {
                        rect,
                        kind: control.kind,
                        text: control.text.clone(),
                        focused: control.id == self.focus,
                        hovered: Some(control.id) == self.hovered,
                        disabled: control.disabled,
                    },
                    Some(control.id),
                ));
            }
            if geometry.wide {
                parts.push((
                    UiElement {
                        rect: UiRect {
                            left: geometry.body.left - 2,
                            top: geometry.body.top,
                            width: 1,
                            height: geometry.body.height,
                        },
                        kind: UiKind::Divider,
                        text: String::new(),
                        focused: false,
                        hovered: false,
                        disabled: false,
                    },
                    None,
                ));
            }
        }
        let body_end = rows.len().saturating_sub(geometry.footer_rows);
        let body = &rows[geometry.navigation_rows..body_end];
        let (offset, maximum) = self.offset(body, geometry);
        let mut position = 0;
        for row in body {
            let height = Self::row_height(row, geometry.control_height);
            let bottom = position + height;
            if bottom > offset && position < offset + geometry.body.height {
                let top = position.max(offset) - offset;
                let visible = bottom.min(offset + geometry.body.height) - position.max(offset);
                parts.extend(self.row_parts(row, UiRect {
                    top: geometry.body.top + top,
                    height: visible,
                    ..geometry.body
                }));
            }
            position = bottom;
        }
        if maximum > 0 && geometry.body.height > 1 {
            let left = geometry.body.left + geometry.body.width + 1;
            for (top, id, text, disabled) in [
                (geometry.body.top, Self::SCROLL_UP, "↑", offset == 0),
                (
                    geometry.body.top + geometry.body.height - 1,
                    Self::SCROLL_DOWN,
                    "↓",
                    offset == maximum,
                ),
            ] {
                parts.push((
                    UiElement {
                        rect: UiRect { left, top, width: 1, height: 1 },
                        kind: UiKind::Scrollbar,
                        text: text.into(),
                        focused: false,
                        hovered: self.hovered == Some(id),
                        disabled,
                    },
                    Some(id),
                ));
            }
            if geometry.body.height > 2 {
                let track = geometry.body.height - 2;
                let thumb = offset * track.saturating_sub(1) / maximum;
                parts.push((
                    UiElement {
                        rect: UiRect {
                            left,
                            top: geometry.body.top + 1 + thumb,
                            width: 1,
                            height: 1,
                        },
                        kind: UiKind::Scrollbar,
                        text: "┃".into(),
                        focused: false,
                        hovered: false,
                        disabled: false,
                    },
                    None,
                ));
            }
        }
        parts.push((
            UiElement {
                rect: UiRect {
                    left: geometry.footer.left,
                    top: geometry.footer.top.saturating_sub(1),
                    width: geometry.footer.width,
                    height: 1,
                },
                kind: UiKind::Divider,
                text: String::new(),
                focused: false,
                hovered: false,
                disabled: false,
            },
            None,
        ));
        for (index, row) in rows[body_end..].iter().enumerate() {
            parts.extend(self.row_parts(row, UiRect {
                top: geometry.footer.top + index * geometry.control_height,
                height: geometry.control_height,
                ..geometry.footer
            }));
        }
        parts.retain(|(element, _)| {
            element.rect.width > 0
                && element.rect.height > 0
                && element.rect.left + element.rect.width <= columns
                && element.rect.top + element.rect.height <= lines
        });
        parts
    }
}

#[derive(Default)]
struct TextInput {
    text: String,
    cursor: usize,
    selected_all: bool,
}

impl TextInput {
    fn new(text: String) -> Self {
        Self { cursor: text.chars().count(), text, selected_all: false }
    }

    fn byte(&self) -> usize {
        self.text.char_indices().nth(self.cursor).map_or(self.text.len(), |(index, _)| index)
    }

    fn key(&mut self, key: &Key, modifiers: ModifiersState, limit: usize) -> bool {
        match key {
            Key::Named(NamedKey::Space) => {
                return self.key(&Key::Character(" ".into()), modifiers, limit);
            },
            Key::Character(text) if modifiers.control_key() && text.eq_ignore_ascii_case("a") => {
                self.selected_all = true;
            },
            Key::Named(NamedKey::ArrowLeft) => {
                self.cursor = self.cursor.saturating_sub(1);
                self.selected_all = false;
            },
            Key::Named(NamedKey::ArrowRight) => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                self.selected_all = false;
            },
            Key::Named(NamedKey::Home) => {
                self.cursor = 0;
                self.selected_all = false;
            },
            Key::Named(NamedKey::End) => {
                self.cursor = self.text.chars().count();
                self.selected_all = false;
            },
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => {
                if self.selected_all {
                    self.text.clear();
                    self.cursor = 0;
                    self.selected_all = false;
                } else if *key == Key::Named(NamedKey::Backspace) && self.cursor > 0 {
                    let end = self.byte();
                    self.cursor -= 1;
                    self.text.replace_range(self.byte()..end, "");
                } else if *key == Key::Named(NamedKey::Delete) {
                    let start = self.byte();
                    if let Some(ch) = self.text[start..].chars().next() {
                        self.text.replace_range(start..start + ch.len_utf8(), "");
                    }
                }
            },
            Key::Character(text) if !modifiers.control_key() && !modifiers.super_key() => {
                let count = text.chars().count();
                let current = if self.selected_all { 0 } else { self.text.chars().count() };
                if text.chars().any(char::is_control) || current + count > limit {
                    return true;
                }
                if self.selected_all {
                    self.text.clear();
                    self.cursor = 0;
                    self.selected_all = false;
                }
                self.text.insert_str(self.byte(), text);
                self.cursor += count;
            },
            _ => return false,
        }
        true
    }

    fn display(&self, width: usize, focused: bool) -> String {
        let mut text = self.text.clone();
        if focused {
            text.insert(self.byte(), '│');
        }
        let before = if focused { self.text[..self.byte()].width() + 1 } else { 0 };
        let mut skip = before.saturating_sub(width.saturating_sub(1));
        while skip > 0 && !text.is_empty() {
            let ch = text.remove(0);
            skip = skip.saturating_sub(ch.width().unwrap_or(0));
        }
        text
    }
}

pub struct PreferencesDialog {
    pub original: Rc<UiConfig>,
    pub values: SettingsValues,
    pub status: Option<Message>,
    pub locale: Locale,
    pub original_font_size: crossfont::Size,
    page: usize,
    shortcuts_page: usize,
    selected_shortcut: usize,
    capturing_shortcut: bool,
    panel: Panel,
    family: TextInput,
    workspaces: Vec<String>,
}

impl PreferencesDialog {
    pub fn new(config: Rc<UiConfig>, font_size: crossfont::Size, workspaces: Vec<String>) -> Self {
        let values = SettingsValues::from_config(&config);
        Self {
            locale: values.language.resolve(),
            family: TextInput::new(values.font_family.clone()),
            original: config,
            original_font_size: font_size,
            values,
            status: None,
            page: 0,
            shortcuts_page: 0,
            selected_shortcut: 0,
            capturing_shortcut: false,
            panel: Panel::default(),
            workspaces,
        }
    }

    fn shortcuts() -> [(&'static str, Message, &'static str); 16] {
        [
            ("CreateNewTab", Message::NewTab, ""),
            ("DuplicateTab", Message::DuplicateTab, ""),
            ("ReopenClosedTab", Message::ReopenClosedTab, ""),
            ("SplitVertical", Message::SplitVertical, ""),
            ("SplitHorizontal", Message::SplitHorizontal, ""),
            ("FocusPaneLeft", Message::FocusPane, " ←"),
            ("FocusPaneRight", Message::FocusPane, " →"),
            ("FocusPaneUp", Message::FocusPane, " ↑"),
            ("FocusPaneDown", Message::FocusPane, " ↓"),
            ("TogglePaneZoom", Message::TogglePaneZoom, ""),
            ("ClosePane", Message::ClosePane, ""),
            ("OpenWorkspaces", Message::Workspaces, ""),
            ("MoveTabLeft", Message::MoveTab, " ←"),
            ("MoveTabRight", Message::MoveTab, " →"),
            ("OpenSettings", Message::Settings, ""),
            ("SaveSession", Message::SaveAll, ""),
        ]
    }

    fn shortcut_label(&self, action: &str) -> String {
        let mut overrides = self.original.general.shortcuts.clone();
        overrides.extend(self.values.shortcut_overrides.clone());
        let action =
            toml::Value::String(action.into()).try_into::<crate::config::Action>().unwrap();
        let shortcuts: Vec<_> = resolved_shortcuts(&self.original, &overrides)
            .into_iter()
            .filter_map(|binding| {
                if binding.action != action {
                    return None;
                }
                let crate::config::BindingKey::Keycode { key, .. } = &binding.trigger else {
                    return Some(format!("{:?}", binding.trigger));
                };
                shortcut_text(key, binding.mods)
            })
            .collect();
        if shortcuts.is_empty() {
            self.locale.text(Message::ShortcutDisabled).into()
        } else {
            shortcuts.join(" / ")
        }
    }

    fn capture_shortcut(
        &mut self,
        key: &Key,
        modifiers: ModifiersState,
        physical: Option<PhysicalKey>,
    ) {
        if *key == Key::Named(NamedKey::Escape) {
            self.capturing_shortcut = false;
            self.status = None;
            return;
        }
        let Some(shortcut) = shortcut_text(key, modifiers) else { return };
        let (action, ..) = Self::shortcuts()[self.selected_shortcut];
        if shortcut_binding(action, &shortcut).is_none() {
            self.status = Some(Message::ShortcutInvalid);
            return;
        }
        let mut overrides = self.original.general.shortcuts.clone();
        overrides.extend(self.values.shortcut_overrides.clone());
        overrides.insert(action.into(), shortcut.clone());
        if shortcut_conflict(&self.original, &overrides, action, &shortcut, physical) {
            self.status = Some(Message::ShortcutConflict);
            return;
        }
        self.values.shortcut_overrides.insert(action.into(), shortcut);
        if let Some(physical) = physical {
            self.values.shortcut_physical.insert(action.into(), physical);
        } else {
            self.values.shortcut_physical.remove(action);
        }
        self.capturing_shortcut = false;
        self.status = Some(Message::ShortcutChanged);
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        let t = |key| self.locale.text(key);
        let navigation = |id, page, message| {
            let mut control = Control::button(id, t(message));
            control.kind =
                if self.page == page { UiKind::SelectedNavigation } else { UiKind::Navigation };
            control
        };
        let checkbox = |id, checked, message| {
            Row::One(Control {
                id,
                kind: UiKind::Checkbox,
                text: format!("{} {}", if checked { "[✓]" } else { "[ ]" }, t(message)),
                disabled: false,
            })
        };
        let mut rows = vec![
            Row::Two(navigation(0, 0, Message::Appearance), navigation(20, 3, Message::Terminal)),
            Row::Two(navigation(21, 4, Message::Tabs), navigation(1, 1, Message::Sessions)),
            Row::Two(
                navigation(13, 5, Message::LanguagePage),
                navigation(2, 2, Message::Shortcuts),
            ),
        ];
        match self.page {
            0 => {
                rows.push(Row::Label(t(Message::FontFamily).into()));
                rows.push(Row::One(Control {
                    id: 3,
                    kind: UiKind::Input,
                    text: self.family.display(
                        Panel::content_width(columns, lines, true).saturating_sub(2),
                        self.panel.focus == 3,
                    ),
                    disabled: false,
                }));
                rows.push(Row::Label(format!(
                    "{}: {:.1} pt",
                    t(Message::FontSize),
                    self.values.font_size
                )));
                rows.push(Row::Two(Control::button(4, "−"), Control::button(5, "+")));
                rows.push(Row::Label(format!(
                    "{}: {:.0}%",
                    t(Message::Opacity),
                    self.values.opacity * 100.
                )));
                rows.push(Row::Two(Control::button(6, "−"), Control::button(7, "+")));
                let theme = match self.values.theme {
                    ThemeChoice::Keep => Message::CurrentTheme,
                    ThemeChoice::Dark => Message::DarkTheme,
                    ThemeChoice::Light => Message::LightTheme,
                };
                rows.push(Row::One(Control::button(
                    8,
                    format!("{}: {}  ›", t(Message::Theme), t(theme)),
                )));
            },
            1 => {
                rows.push(checkbox(9, self.values.restore_session, Message::RestoreAtStartup));
                rows.push(checkbox(10, self.values.session_autosave, Message::AutoRecovery));
                rows.push(Row::One(Control::button(
                    11,
                    format!(
                        "{}: {}  ›",
                        t(Message::StartupWorkspace),
                        self.values
                            .startup_session
                            .as_deref()
                            .unwrap_or(t(Message::LastSavedSession))
                    ),
                )));
                rows.extend(Panel::labels(
                    t(Message::RecoveryHint),
                    Panel::content_width(columns, lines, true),
                ));
            },
            3 => {
                rows.push(Row::Label(format!(
                    "{}: {}",
                    t(Message::ScrollbackLines),
                    self.values.scrollback_lines
                )));
                rows.push(Row::Two(Control::button(22, "− 1000"), Control::button(23, "+ 1000")));
                rows.push(Row::Label(t(Message::ScrollbackSaveHint).into()));
                rows.push(checkbox(
                    24,
                    self.values.hide_mouse_when_typing,
                    Message::HideMouseTyping,
                ));
                rows.push(checkbox(25, self.values.copy_on_select, Message::CopyOnSelect));
            },
            4 => {
                rows.push(checkbox(16, self.values.confirm_tab_close, Message::ConfirmTabClose));
                rows.extend(Panel::labels(
                    t(Message::TabSettingsHint),
                    Panel::content_width(columns, lines, true),
                ));
                rows.push(Row::Label(format!("{}   Ctrl+Shift+D", t(Message::DuplicateTab))));
                rows.push(Row::Label(format!(
                    "{}   Ctrl+Shift+Alt+T",
                    t(Message::ReopenClosedTab)
                )));
            },
            5 => {
                for (index, language) in Language::ALL.into_iter().enumerate() {
                    rows.push(Row::One(Control {
                        id: 30 + index,
                        kind: UiKind::Radio,
                        text: format!(
                            "{} {}",
                            if self.values.language == language { "●" } else { "○" },
                            language.label(self.locale),
                        ),
                        disabled: false,
                    }));
                }
                rows.extend(Panel::labels(
                    t(Message::SettingsLanguageHint),
                    Panel::content_width(columns, lines, true),
                ));
            },
            _ => {
                for (index, (action, label, suffix)) in
                    Self::shortcuts().into_iter().enumerate().skip(self.shortcuts_page * 2).take(2)
                {
                    let mut button = Control::button(100 + index, format!("{}{suffix}", t(label)));
                    if index == self.selected_shortcut {
                        button.kind = UiKind::SelectedItem;
                    }
                    rows.push(Row::One(button));
                    rows.push(Row::Label(self.shortcut_label(action)));
                }
                rows.push(Row::Label(
                    t(if self.capturing_shortcut {
                        Message::ShortcutCapture
                    } else {
                        Message::ShortcutEditHint
                    })
                    .into(),
                ));
                let page = format!(
                    "{} / {}",
                    self.shortcuts_page + 1,
                    Self::shortcuts().len().div_ceil(2)
                );
                rows.push(Row::Two(
                    Control::button(26, format!("‹  {page}")),
                    Control::button(27, format!("{page}  ›")),
                ));
                rows.push(Row::One(Control::button(
                    28,
                    t(if self.capturing_shortcut {
                        Message::ShortcutCancelCapture
                    } else {
                        Message::ShortcutClear
                    }),
                )));
            },
        }
        if self.page != 2 {
            rows.push(Row::One(Control::button(12, t(Message::Defaults))));
        }
        let mut save = Control::button(14, t(Message::SaveSettings));
        save.kind = UiKind::PrimaryButton;
        rows.push(Row::Two(save, Control::button(15, t(Message::Cancel))));
        rows
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        self.status = None;
        match id {
            0..=2 | 13 | 20 | 21 => {
                self.page = match id {
                    20 => 3,
                    21 => 4,
                    13 => 5,
                    _ => id,
                };
                self.panel.focus = id;
                self.panel.hovered = None;
                self.panel.offset.set(0);
                self.panel.last_focus.set(None);
                self.capturing_shortcut = false;
            },
            4 => self.values.font_size = (self.values.font_size - 0.5).clamp(1., 128.),
            5 => self.values.font_size = (self.values.font_size + 0.5).clamp(1., 128.),
            6 => self.values.opacity = (self.values.opacity - 0.05).clamp(0., 1.),
            7 => self.values.opacity = (self.values.opacity + 0.05).clamp(0., 1.),
            8 => {
                self.values.theme = match self.values.theme {
                    ThemeChoice::Keep => ThemeChoice::Dark,
                    ThemeChoice::Dark => ThemeChoice::Light,
                    ThemeChoice::Light => ThemeChoice::Keep,
                }
            },
            9 => self.values.restore_session = !self.values.restore_session,
            10 => self.values.session_autosave = !self.values.session_autosave,
            11 => {
                let next = self
                    .values
                    .startup_session
                    .as_ref()
                    .and_then(|name| self.workspaces.iter().position(|candidate| candidate == name))
                    .map_or(0, |i| i + 1);
                self.values.startup_session = self.workspaces.get(next).cloned();
            },
            12 => {
                self.values = SettingsValues::from_config(&UiConfig::default());
                self.values.theme = ThemeChoice::Dark;
                self.values.shortcut_overrides = default_shortcuts();
                self.family = TextInput::new(self.values.font_family.clone());
                self.locale = self.values.language.resolve();
            },
            14 => {
                self.values.font_family = self.family.text.trim().into();
                if self.values.validate().is_err() {
                    self.status = Some(Message::SettingsInvalid);
                } else if !self.values.shortcut_changes_valid(&self.original) {
                    self.status = Some(Message::ShortcutConflict);
                } else {
                    return Some(ProductivityAction::SaveSettings(self.values.clone()));
                }
            },
            15 => return Some(ProductivityAction::Cancel),
            16 => self.values.confirm_tab_close = !self.values.confirm_tab_close,
            22 => self.values.scrollback_lines = self.values.scrollback_lines.saturating_sub(1000),
            23 => {
                self.values.scrollback_lines =
                    self.values.scrollback_lines.saturating_add(1000).min(100_000)
            },
            24 => self.values.hide_mouse_when_typing = !self.values.hide_mouse_when_typing,
            25 => self.values.copy_on_select = !self.values.copy_on_select,
            26 | 27 => {
                let pages = Self::shortcuts().len().div_ceil(2);
                self.capturing_shortcut = false;
                self.shortcuts_page =
                    (self.shortcuts_page + if id == 26 { pages - 1 } else { 1 }) % pages;
                self.selected_shortcut = self.shortcuts_page * 2;
            },
            28 => {
                if self.capturing_shortcut {
                    self.capturing_shortcut = false;
                } else {
                    let (action, ..) = Self::shortcuts()[self.selected_shortcut];
                    self.values.shortcut_overrides.insert(action.into(), String::new());
                    self.values.shortcut_physical.remove(action);
                    self.status = Some(Message::ShortcutChanged);
                }
            },
            100..=115 => {
                self.selected_shortcut = id - 100;
                self.capturing_shortcut = true;
            },
            30..=32 => {
                self.values.language = Language::ALL[id - 30];
                self.locale = self.values.language.resolve();
            },
            _ => (),
        }
        None
    }
}

pub struct WorkspaceDialog {
    pub locale: Locale,
    pub status: Option<Message>,
    pub names: Vec<String>,
    pub selected: Option<String>,
    pub startup: Option<String>,
    panel: Panel,
    name: TextInput,
    confirm_delete: bool,
}

impl WorkspaceDialog {
    pub fn new(locale: Locale, names: Vec<String>, startup: Option<String>) -> Self {
        Self {
            locale,
            status: None,
            names,
            selected: None,
            startup,
            panel: Panel::default(),
            name: TextInput::default(),
            confirm_delete: false,
        }
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        let t = |key| self.locale.text(key);
        let mut rows = vec![
            Row::Label(t(Message::WorkspaceName).into()),
            Row::One(Control {
                id: 0,
                kind: UiKind::Input,
                text: self.name.display(
                    Panel::content_width(columns, lines, false).saturating_sub(2),
                    self.panel.focus == 0,
                ),
                disabled: false,
            }),
        ];
        if self.names.is_empty() {
            rows.push(Row::Label(t(Message::NoWorkspaces).into()));
        }
        for (i, name) in self.names.iter().enumerate() {
            rows.push(Row::One(Control {
                id: 100 + i,
                kind: UiKind::Radio,
                text: format!(
                    "{} {}{}",
                    if self.selected.as_ref() == Some(name) { "●" } else { "○" },
                    name,
                    if self.startup.as_ref() == Some(name) { " ★" } else { "" }
                ),
                disabled: false,
            }));
        }
        let mut save = Control::button(1, t(Message::SaveWorkspace));
        save.kind = UiKind::PrimaryButton;
        let mut open = Control::button(2, t(Message::OpenWorkspace));
        open.disabled = self.selected.is_none();
        rows.push(Row::Two(save, open));
        let mut startup = Control::button(3, t(Message::UseAtStartup));
        startup.disabled = self.selected.is_none();
        let mut delete = Control::button(
            4,
            t(if self.confirm_delete { Message::ConfirmDelete } else { Message::DeleteWorkspace }),
        );
        delete.disabled = self.selected.is_none();
        delete.kind = UiKind::DangerButton;
        rows.push(Row::Two(startup, delete));
        rows.push(Row::One(Control::button(5, t(Message::Cancel))));
        rows
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        if id != 4 {
            self.confirm_delete = false;
        }
        self.status = None;
        match id {
            1 => {
                let name = self.name.text.trim();
                if name.is_empty()
                    || name.chars().count() > 80
                    || name.chars().any(char::is_control)
                {
                    self.status = Some(Message::WorkspaceInvalid);
                    None
                } else {
                    Some(ProductivityAction::SaveWorkspace(name.into()))
                }
            },
            2 => self.selected.clone().map(ProductivityAction::OpenWorkspace),
            3 => self.selected.clone().map(ProductivityAction::StartupWorkspace),
            4 if self.confirm_delete => {
                self.selected.clone().map(ProductivityAction::RemoveWorkspace)
            },
            4 => {
                self.confirm_delete = true;
                None
            },
            5 => Some(ProductivityAction::Cancel),
            100.. => {
                if let Some(name) = self.names.get(id - 100) {
                    self.selected = Some(name.clone());
                    self.name = TextInput::new(name.clone());
                }
                None
            },
            _ => None,
        }
    }
}

pub enum ProductivityDialog {
    Preferences(Box<PreferencesDialog>),
    Workspaces(WorkspaceDialog),
}

impl ProductivityDialog {
    pub fn locale_mut(&mut self) -> &mut Locale {
        match self {
            Self::Preferences(dialog) => &mut dialog.locale,
            Self::Workspaces(dialog) => &mut dialog.locale,
        }
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        match self {
            Self::Preferences(dialog) => dialog.rows(columns, lines),
            Self::Workspaces(dialog) => dialog.rows(columns, lines),
        }
    }

    fn panel(&self) -> &Panel {
        match self {
            Self::Preferences(dialog) => &dialog.panel,
            Self::Workspaces(dialog) => &dialog.panel,
        }
    }

    fn panel_mut(&mut self) -> &mut Panel {
        match self {
            Self::Preferences(dialog) => &mut dialog.panel,
            Self::Workspaces(dialog) => &mut dialog.panel,
        }
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        match self {
            Self::Preferences(dialog) => dialog.activate(id),
            Self::Workspaces(dialog) => dialog.activate(id),
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        Panel::layout(&self.rows(columns, lines), columns, lines)
    }

    fn parts(&self, columns: usize, lines: usize) -> Vec<(UiElement, Option<usize>)> {
        let (locale, title, subtitle, status) = match self {
            Self::Preferences(d) => {
                let subtitle = if d.page == 2 {
                    Message::ShortcutsSubtitle
                } else {
                    Message::PreferencesSubtitle
                };
                (d.locale, Message::SettingsTitle, subtitle, d.status)
            },
            Self::Workspaces(d) => {
                (d.locale, Message::Workspaces, Message::WorkspacesHint, d.status)
            },
        };
        self.panel().parts(
            &self.rows(columns, lines),
            locale.text(title),
            locale.text(subtitle),
            locale.text(status.unwrap_or(if columns < 64 {
                Message::KeyboardHelpCompact
            } else {
                Message::KeyboardHelp
            })),
            columns,
            lines,
        )
    }

    pub fn elements(&self, columns: usize, lines: usize) -> Vec<UiElement> {
        self.parts(columns, lines).into_iter().map(|(element, _)| element).collect()
    }

    pub fn key(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Option<ProductivityAction> {
        if let Self::Preferences(dialog) = self {
            if dialog.capturing_shortcut {
                if event.repeat {
                    return None;
                }
                let key = if (cfg!(target_os = "macos")
                    || (cfg!(windows) && modifiers.control_key()))
                    && modifiers.alt_key()
                {
                    event.key_without_modifiers()
                } else {
                    event.logical_key.clone()
                };
                dialog.capture_shortcut(&key, modifiers, Some(event.physical_key));
                return None;
            }
        }
        self.logical_key(&event.logical_key, modifiers)
    }

    pub fn captures_shortcut(&self) -> bool {
        matches!(self, Self::Preferences(dialog) if dialog.capturing_shortcut)
    }

    fn logical_key(&mut self, key: &Key, modifiers: ModifiersState) -> Option<ProductivityAction> {
        if let Self::Preferences(dialog) = self {
            if dialog.capturing_shortcut {
                dialog.capture_shortcut(key, modifiers, None);
                return None;
            }
        }
        if *key == Key::Named(NamedKey::Escape) {
            return Some(ProductivityAction::Cancel);
        }
        if matches!(key, Key::Named(NamedKey::PageUp | NamedKey::PageDown)) {
            let (columns, lines) = self.panel().viewport.get();
            let delta = (lines / 2).max(1) as isize;
            self.scroll(
                if *key == Key::Named(NamedKey::PageUp) { -delta } else { delta },
                columns,
                lines,
            );
            return None;
        }
        self.panel().last_focus.set(None);
        let focus = self.panel().focus;
        let input = match self {
            Self::Preferences(dialog) if focus == 3 => Some((&mut dialog.family, 128)),
            Self::Workspaces(dialog) if focus == 0 => Some((&mut dialog.name, 80)),
            _ => None,
        };
        if let Some((input, limit)) = input {
            if input.key(key, modifiers, limit) {
                if let Self::Preferences(dialog) = self {
                    dialog.values.font_family = dialog.family.text.clone();
                }
                return None;
            }
        }
        match key {
            Key::Named(NamedKey::Tab | NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let (columns, lines) = self.panel().viewport.get();
                let rows = self.rows(columns.max(80), lines.max(24));
                self.panel_mut().move_focus(
                    &rows,
                    modifiers.shift_key() || *key == Key::Named(NamedKey::ArrowUp),
                );
                None
            },
            Key::Named(NamedKey::Enter | NamedKey::Space) => self.activate(focus),
            Key::Character(text) if text == " " => self.activate(focus),
            _ => None,
        }
    }

    pub fn click(
        &mut self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<ProductivityAction> {
        if !self.layout(columns, lines).contains(column, line) {
            return Some(ProductivityAction::Cancel);
        }
        let id = self.parts(columns, lines).into_iter().find_map(|(element, id)| {
            (!element.disabled && element.rect.contains(column, line)).then_some(id).flatten()
        })?;
        if id == Panel::SCROLL_UP || id == Panel::SCROLL_DOWN {
            self.scroll(if id == Panel::SCROLL_UP { -3 } else { 3 }, columns, lines);
            return None;
        }
        self.panel().last_focus.set(None);
        self.panel_mut().focus = id;
        self.activate(id)
    }

    pub fn scroll(&mut self, delta: isize, columns: usize, lines: usize) -> bool {
        if self.captures_shortcut() {
            return false;
        }
        self.panel().scroll(&self.rows(columns, lines), delta, columns, lines)
    }

    pub fn hover(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let hovered = self.parts(columns, lines).into_iter().find_map(|(element, id)| {
            (!element.disabled && element.rect.contains(column, line)).then_some(id).flatten()
        });
        let changed = hovered != self.panel().hovered;
        self.panel_mut().hovered = hovered;
        changed
    }

    pub fn paste(&mut self, text: &str) {
        match self {
            Self::Preferences(dialog) if dialog.panel.focus == 3 => {
                dialog.family.key(&Key::Character(text.into()), ModifiersState::empty(), 128);
                dialog.values.font_family = dialog.family.text.clone();
            },
            Self::Workspaces(dialog) if dialog.panel.focus == 0 => {
                dialog.name.key(&Key::Character(text.into()), ModifiersState::empty(), 80);
            },
            _ => (),
        }
    }

    pub fn pointer_cursor(&self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        self.parts(columns, lines).into_iter().any(|(element, id)| {
            id.is_some() && !element.disabled && element.rect.contains(column, line)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_capture_rejects_conflicts_and_escape_cancels_only_capture() {
        let config = toml::from_str(
            "[keyboard]\nbindings=[{key='d',mods='Control|Shift',action='DuplicateTab'}]\n",
        )
        .unwrap();
        let preferences =
            PreferencesDialog::new(Rc::new(config), crossfont::Size::new(11.), Vec::new());
        let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
        dialog.activate(2);
        dialog.activate(100);
        assert!(dialog.captures_shortcut());
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Control), ModifiersState::CONTROL),
            None
        );
        assert!(dialog.captures_shortcut());
        dialog.logical_key(&Key::Character("x".into()), ModifiersState::empty());
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(preferences.status, Some(Message::ShortcutInvalid));
        dialog.logical_key(
            &Key::Character("d".into()),
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        );
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(preferences.status, Some(Message::ShortcutConflict));
        assert!(preferences.values.shortcut_overrides.is_empty());
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
            None
        );
        assert!(!dialog.captures_shortcut());
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
            Some(ProductivityAction::Cancel)
        );
    }

    #[test]
    fn shortcut_changes_stay_draft_until_save_and_can_be_disabled() {
        let config = Rc::new(UiConfig::default());
        let preferences =
            PreferencesDialog::new(config.clone(), crossfont::Size::new(11.), Vec::new());
        let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
        dialog.activate(2);
        dialog.activate(100);
        dialog
            .logical_key(&Key::Named(NamedKey::F9), ModifiersState::CONTROL | ModifiersState::ALT);
        assert!(!dialog.captures_shortcut());
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(preferences.shortcut_label("CreateNewTab"), "Ctrl+Alt+F9");
        assert!(config.general.shortcuts.is_empty());
        let mut preview = (*config).clone();
        preferences.values.preview(&mut preview).unwrap();
        assert!(preview.general.shortcuts.is_empty());
        let Some(ProductivityAction::SaveSettings(values)) = dialog.activate(14) else { panic!() };
        assert_eq!(values.shortcut_overrides["CreateNewTab"], "Ctrl+Alt+F9");
        dialog.activate(28);
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(
            preferences.shortcut_label("CreateNewTab"),
            preferences.locale.text(Message::ShortcutDisabled)
        );
    }

    #[test]
    fn preferences_switch_between_sidebar_and_compact_navigation() {
        let dialog = ProductivityDialog::Preferences(Box::new(PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        )));
        let wide = dialog.parts(100, 32);
        let navigation: Vec<_> = wide
            .iter()
            .filter(|(element, _)| {
                matches!(element.kind, UiKind::Navigation | UiKind::SelectedNavigation)
            })
            .collect();
        assert_eq!(navigation.len(), 6);
        assert!(
            navigation.iter().all(|(element, _)| element.rect.left == navigation[0].0.rect.left)
        );
        assert!(navigation.windows(2).all(|pair| pair[0].0.rect.top + 2 == pair[1].0.rect.top));
        let input = wide.iter().find(|(_, id)| *id == Some(3)).unwrap();
        assert!(input.0.rect.left > navigation[0].0.rect.left + navigation[0].0.rect.width);
        assert_eq!(input.0.rect.height, 2);
        let compact = dialog.parts(45, 18);
        let navigation: Vec<_> = compact
            .iter()
            .filter(|(element, _)| {
                matches!(element.kind, UiKind::Navigation | UiKind::SelectedNavigation)
            })
            .collect();
        assert!(
            navigation
                .iter()
                .all(|(element, _)| element.text.width() <= element.rect.width.saturating_sub(2))
        );
        assert_eq!(navigation[0].0.rect.top, navigation[2].0.rect.top);
        assert_eq!(navigation[3].0.rect.top, navigation[5].0.rect.top);
        assert_eq!(navigation[3].0.rect.top, navigation[0].0.rect.top + 1);
        assert_eq!(navigation.iter().map(|(_, id)| *id).collect::<Vec<_>>(), vec![
            Some(0),
            Some(20),
            Some(21),
            Some(1),
            Some(13),
            Some(2)
        ]);
    }

    #[test]
    fn footer_stays_anchored_while_keyboard_reveals_scrolled_controls() {
        let mut dialog = ProductivityDialog::Preferences(Box::new(PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        )));
        let before = dialog.parts(45, 18);
        let save = before.iter().find(|(_, id)| *id == Some(14)).unwrap().0.rect;
        assert!(!before.iter().any(|(_, id)| *id == Some(12)));
        for _ in 0..12 {
            dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
            dialog.parts(45, 18);
        }
        assert_eq!(dialog.panel().focus, 12);
        let after = dialog.parts(45, 18);
        assert!(after.iter().any(|(element, id)| *id == Some(12) && element.focused));
        assert_eq!(after.iter().find(|(_, id)| *id == Some(14)).unwrap().0.rect, save);
        assert!(dialog.panel().offset.get() > 0);
        dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
        assert_eq!(dialog.panel().focus, 14);
        assert_eq!(
            dialog.parts(45, 18).iter().find(|(_, id)| *id == Some(14)).unwrap().0.rect,
            save
        );
    }

    #[test]
    fn workspace_scrolling_is_stable_and_keeps_actions_reachable() {
        let names = (0..30).map(|index| format!("Workspace {index:02}")).collect();
        let mut dialog =
            ProductivityDialog::Workspaces(WorkspaceDialog::new(Locale::En, names, None));
        let initial = dialog.parts(45, 18);
        let footer: Vec<_> = initial
            .iter()
            .filter(|(_, id)| matches!(id, Some(1..=5)))
            .map(|(element, id)| (*id, element.rect))
            .collect();
        assert_eq!(footer.len(), 5);
        assert!(dialog.scroll(3, 45, 18));
        assert_eq!(dialog.panel().focus, 0);
        let scrolled = dialog.parts(45, 18);
        let offset = dialog.panel().offset.get();
        let targets: Vec<_> = scrolled.iter().map(|(element, id)| (*id, element.rect)).collect();
        assert_eq!(
            targets,
            dialog
                .parts(45, 18)
                .iter()
                .map(|(element, id)| (*id, element.rect))
                .collect::<Vec<_>>()
        );
        dialog.hover(10, 10, 45, 18);
        assert_eq!(dialog.panel().offset.get(), offset);
        dialog.logical_key(&Key::Named(NamedKey::PageDown), ModifiersState::empty());
        assert!(dialog.panel().offset.get() > offset);
        dialog.logical_key(&Key::Named(NamedKey::PageUp), ModifiersState::empty());
        assert_eq!(dialog.panel().offset.get(), offset);
        let parts = dialog.parts(45, 18);
        let down = parts.iter().find(|(_, id)| *id == Some(Panel::SCROLL_DOWN)).unwrap().0.rect;
        dialog.click(down.left, down.top, 45, 18);
        assert!(dialog.panel().offset.get() > offset);
        assert_eq!(dialog.panel().focus, 0);
        dialog.panel_mut().focus = 129;
        let last = dialog.parts(45, 18);
        assert!(last.iter().any(|(element, id)| *id == Some(129) && element.focused));
        assert_eq!(
            footer,
            last.iter()
                .filter(|(_, id)| matches!(id, Some(1..=5)))
                .map(|(element, id)| (*id, element.rect))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn resizing_reveals_the_focused_body_control() {
        let mut dialog = ProductivityDialog::Preferences(Box::new(PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        )));
        dialog.panel_mut().focus = 12;
        assert!(dialog.parts(100, 32).iter().any(|(element, _)| element.focused));
        assert!(
            dialog.parts(45, 18).iter().any(|(element, id)| *id == Some(12) && element.focused)
        );
    }

    #[test]
    fn every_preferences_page_keeps_keyboard_and_mouse_targets_visible() {
        for locale in [Locale::En, Locale::PtBr] {
            for page in [0, 20, 21, 1, 13, 2] {
                let mut preferences = PreferencesDialog::new(
                    Rc::new(UiConfig::default()),
                    crossfont::Size::new(11.),
                    Vec::new(),
                );
                preferences.locale = locale;
                preferences.activate(page);
                let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
                for (columns, lines) in [(100, 32), (45, 18), (12, 6), (2, 1)] {
                    for _ in 0..20 {
                        let parts = dialog.parts(columns, lines);
                        assert!(parts.iter().all(|(element, _)| {
                            element.rect.left + element.rect.width <= columns
                                && element.rect.top + element.rect.height <= lines
                        }));
                        let (focused, id) =
                            parts.iter().find(|(element, _)| element.focused).unwrap();
                        assert_eq!(*id, Some(dialog.panel().focus));
                        assert!(dialog.pointer_cursor(
                            focused.rect.left,
                            focused.rect.top,
                            columns,
                            lines
                        ));
                        dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
                    }
                }
            }
        }
    }

    #[test]
    fn language_choice_is_a_local_draft_until_explicit_save() {
        let config = Rc::new(UiConfig::default());
        let before = config.general.language;
        let mut preferences =
            PreferencesDialog::new(config.clone(), crossfont::Size::new(11.), Vec::new());
        preferences.activate(13);
        preferences.activate(32);
        assert_eq!(preferences.values.language, Language::PtBr);
        assert_eq!(preferences.locale, Locale::PtBr);
        assert_eq!(config.general.language, before);
        preferences.activate(20);
        preferences.activate(13);
        assert_eq!(preferences.values.language, Language::PtBr);
        let Some(ProductivityAction::SaveSettings(saved)) = preferences.activate(14) else {
            panic!()
        };
        assert_eq!(saved.language, Language::PtBr);
        assert_eq!(config.general.language, before);
        preferences.activate(31);
        assert_eq!(preferences.locale, Locale::En);
        assert_eq!(preferences.activate(15), Some(ProductivityAction::Cancel));
    }

    #[test]
    fn terminal_controls_preserve_drafts_across_pages_and_bound_scrollback() {
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        preferences.activate(20);
        preferences.values.scrollback_lines = 500;
        preferences.activate(22);
        assert_eq!(preferences.values.scrollback_lines, 0);
        preferences.activate(22);
        assert_eq!(preferences.values.scrollback_lines, 0);
        preferences.values.scrollback_lines = 99_999;
        preferences.activate(23);
        assert_eq!(preferences.values.scrollback_lines, 100_000);
        preferences.activate(23);
        assert_eq!(preferences.values.scrollback_lines, 100_000);
        preferences.activate(24);
        preferences.activate(25);
        let before = preferences.values.clone();
        preferences.activate(0);
        preferences.activate(20);
        assert_eq!(preferences.values, before);
        preferences.activate(12);
        assert_eq!(preferences.values.scrollback_lines, UiConfig::default().scrolling.history());
    }

    #[test]
    fn paginated_shortcuts_show_every_binding_at_compact_size() {
        let shortcuts = PreferencesDialog::shortcuts();
        for locale in [Locale::En, Locale::PtBr] {
            let mut preferences = PreferencesDialog::new(
                Rc::new(UiConfig::default()),
                crossfont::Size::new(11.),
                Vec::new(),
            );
            preferences.locale = locale;
            preferences.activate(2);
            let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
            for (index, entries) in shortcuts.chunks(2).enumerate() {
                let parts = dialog.parts(45, 18);
                let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
                for (action, ..) in entries {
                    assert!(
                        parts
                            .iter()
                            .any(|(element, _)| element.text == preferences.shortcut_label(action))
                    );
                }
                let (button, _) = parts.iter().find(|(_, id)| *id == Some(27)).unwrap();
                assert_eq!(dialog.click(button.rect.left, button.rect.top, 45, 18), None);
                let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
                assert_eq!(preferences.shortcuts_page, (index + 1) % shortcuts.len().div_ceil(2));
            }
        }
    }

    #[test]
    fn named_space_is_typed_into_font_and_workspace_inputs() {
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        preferences.panel.focus = 3;
        for mut dialog in [
            ProductivityDialog::Preferences(Box::new(preferences)),
            ProductivityDialog::Workspaces(WorkspaceDialog::new(Locale::En, Vec::new(), None)),
        ] {
            dialog.logical_key(&Key::Character("a".into()), ModifiersState::CONTROL);
            dialog.logical_key(&Key::Character("Daily".into()), ModifiersState::empty());
            dialog.logical_key(&Key::Named(NamedKey::Space), ModifiersState::empty());
            dialog.logical_key(&Key::Character("panes".into()), ModifiersState::empty());
            match dialog {
                ProductivityDialog::Preferences(preferences) => {
                    assert_eq!(preferences.values.font_family, "Daily panes")
                },
                ProductivityDialog::Workspaces(workspace) => {
                    assert_eq!(workspace.name.text, "Daily panes")
                },
            }
        }
    }

    #[test]
    fn unicode_editing_is_bounded_and_keeps_the_cursor_visible() {
        let mut input = TextInput::new("produção日本語".into());
        input.key(&Key::Named(NamedKey::ArrowLeft), ModifiersState::empty(), 80);
        input.key(&Key::Named(NamedKey::Backspace), ModifiersState::empty(), 80);
        assert_eq!(input.text, "produção日語");
        assert!(input.display(6, true).contains('│'));
        input.key(&Key::Character("a".into()), ModifiersState::CONTROL, 80);
        input.key(&Key::Character("backend".into()), ModifiersState::empty(), 80);
        assert_eq!(input.text, "backend");
        input.key(&Key::Character("bad\ntext".into()), ModifiersState::empty(), 80);
        assert_eq!(input.text, "backend");
    }

    #[test]
    fn narrow_panels_keep_focused_actions_visible_and_inside_the_viewport() {
        let mut dialog = ProductivityDialog::Preferences(Box::new(PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        )));
        for (columns, lines) in [(100, 32), (45, 18), (12, 6), (2, 1)] {
            for _ in 0..25 {
                let elements = dialog.elements(columns, lines);
                assert!(
                    elements
                        .iter()
                        .all(|element| element.rect.left + element.rect.width <= columns
                            && element.rect.top + element.rect.height <= lines)
                );
                assert!(elements.iter().any(|element| element.focused));
                dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
            }
        }
    }

    #[test]
    fn size_and_opacity_buttons_preserve_small_values_and_clamp_to_supported_bounds() {
        let mut dialog = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        dialog.values.font_size = 1.5;
        dialog.activate(4);
        assert_eq!(dialog.values.font_size, 1.0);
        dialog.activate(4);
        assert_eq!(dialog.values.font_size, 1.0);
        dialog.activate(5);
        assert_eq!(dialog.values.font_size, 1.5);
        dialog.values.font_size = 128.0;
        dialog.activate(5);
        assert_eq!(dialog.values.font_size, 128.0);
        dialog.values.opacity = 0.1;
        dialog.activate(6);
        assert!((dialog.values.opacity - 0.05).abs() < f32::EPSILON);
        dialog.activate(6);
        assert!(dialog.values.opacity.abs() < f32::EPSILON);
        dialog.activate(6);
        assert_eq!(dialog.values.opacity, 0.0);
        dialog.values.opacity = 1.0;
        dialog.activate(7);
        assert_eq!(dialog.values.opacity, 1.0);
    }

    #[test]
    fn long_unicode_inputs_keep_the_final_cursor_inside_the_rendered_field() {
        let text = "界".repeat(80);
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        preferences.family = TextInput::new(text.clone());
        preferences.panel.focus = 3;
        let mut workspace = WorkspaceDialog::new(Locale::En, Vec::new(), None);
        workspace.name = TextInput::new(text);
        for dialog in [
            ProductivityDialog::Preferences(Box::new(preferences)),
            ProductivityDialog::Workspaces(workspace),
        ] {
            for columns in [45, 100, 200] {
                let input = dialog
                    .elements(columns, 32)
                    .into_iter()
                    .find(|element| element.kind == UiKind::Input)
                    .unwrap();
                let cursor = input.text.find('│').unwrap();
                assert!(input.text[..cursor].width() < input.rect.width);
                assert!(input.text.width() <= input.rect.width.saturating_sub(2));
                assert!(input.text.ends_with('│'));
            }
        }
    }

    #[test]
    fn pasting_on_buttons_preserves_preferences_and_workspace_confirmation() {
        let preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        let before = preferences.values.clone();
        let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
        for focus in [4, 5, 6, 7, 8, 12] {
            dialog.panel_mut().focus = focus;
            dialog.paste(" ");
            dialog.paste("different text");
            let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
            assert_eq!(preferences.values, before);
            assert_eq!(preferences.family.text, before.font_family);
        }
        let mut workspace = WorkspaceDialog::new(Locale::En, vec!["Backend".into()], None);
        workspace.activate(100);
        workspace.panel.focus = 4;
        let mut dialog = ProductivityDialog::Workspaces(workspace);
        dialog.paste(" ");
        let ProductivityDialog::Workspaces(workspace) = dialog else { unreachable!() };
        assert!(!workspace.confirm_delete);
        assert_eq!(workspace.selected.as_deref(), Some("Backend"));
        assert_eq!(workspace.name.text, "Backend");
    }

    #[test]
    fn paste_updates_only_the_focused_input_with_normal_limits() {
        let mut dialog = ProductivityDialog::Preferences(Box::new(PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        )));
        dialog.panel_mut().focus = 3;
        dialog.logical_key(&Key::Character("a".into()), ModifiersState::CONTROL);
        dialog.paste("日本語 font");
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(preferences.values.font_family, "日本語 font");
        dialog.paste("invalid\ntext");
        let ProductivityDialog::Preferences(preferences) = dialog else { unreachable!() };
        assert_eq!(preferences.values.font_family, "日本語 font");
        let mut dialog =
            ProductivityDialog::Workspaces(WorkspaceDialog::new(Locale::En, Vec::new(), None));
        dialog.paste("Backend");
        dialog.paste(&"x".repeat(80));
        let ProductivityDialog::Workspaces(workspace) = dialog else { unreachable!() };
        assert_eq!(workspace.name.text, "Backend");
    }

    #[test]
    fn workspace_selection_and_deletion_require_explicit_confirmation() {
        let mut dialog = WorkspaceDialog::new(Locale::En, vec!["Backend".into()], None);
        assert_eq!(dialog.activate(100), None);
        assert_eq!(dialog.selected.as_deref(), Some("Backend"));
        assert_eq!(dialog.activate(4), None);
        assert_eq!(dialog.activate(4), Some(ProductivityAction::RemoveWorkspace("Backend".into())));
        dialog.activate(100);
        assert_eq!(dialog.activate(4), None);
        dialog.activate(0);
        assert_eq!(dialog.activate(4), None);
    }
}
