use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};

use crate::config::{KeyBinding, KeyLocation, UiConfig};
use crate::localization::{Language, Locale, Message};
use crate::personalization::IconPreset;
use crate::settings::{
    SettingsValues, ThemeChoice, default_shortcuts, resolved_shortcuts, shortcut_binding,
    shortcut_conflict, shortcut_text,
};
use crate::tab_ui::{UiElement, UiKind, UiRect};
#[cfg(termust_updater)]
use crate::updates::Status as UpdateStatus;

const FIRST_ICON_CONTROL: usize = 600;

#[derive(Clone, Debug, PartialEq)]
pub enum ProductivityAction {
    SaveSettings(SettingsValues),
    #[cfg(termust_updater)]
    CheckUpdates,
    #[cfg(termust_updater)]
    InstallUpdate(String),
    SaveWorkspace(String),
    OpenWorkspace(String),
    RemoveWorkspace(String),
    StartupWorkspace(String),
    GroupCreate(String),
    GroupRename { old: String, new: String },
    GroupMove(Option<String>),
    GroupShow(Option<String>),
    FocusAgent { tab_id: crate::tabs::TabId, pane_id: crate::tabs::TabId },
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
    IconChoices(Control, Option<Control>),
    Toolbar(Control, Control),
    ShortcutHeader(String, String),
    AgentHeader(String, String),
    Shortcut { control: Control, shortcut: String },
    Adjustment { label: String, value: String, decrease: Control, increase: Control },
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
            if word.width() <= width {
                line.push_str(word);
                continue;
            }
            let mut used = 0;
            for ch in word.chars() {
                let size = ch.width().unwrap_or(0);
                if used > 0 && used + size > width {
                    rows.push(Row::Label(std::mem::take(&mut line)));
                    used = 0;
                }
                line.push(ch);
                used += size;
            }
        }
        if !line.is_empty() {
            rows.push(Row::Label(line));
        }
        rows
    }

    fn controls(rows: &[Row]) -> Vec<&Control> {
        rows.iter()
            .flat_map(|row| match row {
                Row::Label(_) | Row::ShortcutHeader(..) | Row::AgentHeader(..) => Vec::new(),
                Row::One(control) | Row::Shortcut { control, .. } => vec![control],
                Row::Two(left, right) | Row::Toolbar(left, right) => vec![left, right],
                Row::IconChoices(left, right) => {
                    std::iter::once(left).chain(right.as_ref()).collect()
                },
                Row::Adjustment { decrease, increase, .. } => vec![decrease, increase],
            })
            .collect()
    }

    fn geometry(rows: &[Row], columns: usize, lines: usize) -> PanelGeometry {
        let settings = Self::controls(rows).first().is_some_and(|control| {
            matches!(control.kind, UiKind::Navigation | UiKind::SelectedNavigation)
        });
        let compact = matches!(rows.first(), Some(Row::AgentHeader(..)));
        let wide = settings && Self::wide(columns, lines);
        let control_height = if lines >= 26 && !compact { 2 } else { 1 };
        let width = if compact {
            columns.saturating_sub(usize::from(columns >= 8) * 2).min(56)
        } else {
            Self::width(columns)
        };
        let navigation_rows = rows
            .iter()
            .take_while(|row| {
                Self::controls(std::slice::from_ref(*row)).iter().any(|control| {
                    matches!(control.kind, UiKind::Navigation | UiKind::SelectedNavigation)
                })
            })
            .count();
        let navigation_controls = Self::controls(&rows[..navigation_rows]).len();
        let shortcuts =
            rows.iter().any(|row| matches!(row, Row::ShortcutHeader(..) | Row::AgentHeader(..)));
        let footer_rows = if compact {
            1
        } else if shortcuts {
            2
        } else if settings {
            1
        } else {
            3
        }
        .min(rows.len());
        let desired = if settings {
            30
        } else {
            rows.iter().map(|row| Self::row_height(row, control_height)).sum::<usize>()
                + if compact { 7 } else { 8 }
        };
        let available = lines.saturating_sub(usize::from(lines >= 22) * 2);
        let height = desired.min(available).min(if compact { 16 } else { usize::MAX });
        let rect = UiRect {
            left: if compact {
                columns.saturating_sub(width + usize::from(columns > width))
            } else {
                columns.saturating_sub(width) / 2
            },
            top: if compact { 0 } else { lines.saturating_sub(height) / 2 },
            width,
            height,
        };
        let inset = usize::from(width >= 6) * 2;
        let inner_width = width.saturating_sub(inset * 2);
        let footer_height = rows[rows.len().saturating_sub(footer_rows)..]
            .iter()
            .map(|row| Self::row_height(row, control_height))
            .sum::<usize>();
        let footer_top = height.saturating_sub(footer_height + 3);
        let body_top = if compact {
            3
        } else if settings && !wide {
            5 + navigation_controls.div_ceil(3)
        } else {
            4
        };
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
        match row {
            Row::IconChoices(..) => 4,
            Row::One(Control { kind: UiKind::AppearanceChoice { icon: Some(_), .. }, .. }) => {
                control_height + 1
            },
            Row::Label(_)
            | Row::ShortcutHeader(..)
            | Row::AgentHeader(..)
            | Row::Shortcut { .. } => 1,
            _ => control_height,
        }
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

    fn shortcut_row_text(label: &str, shortcut: &str, width: usize) -> String {
        let key_width = width.saturating_sub(14).min(29);
        let label_width = width.saturating_sub(key_width + 2);
        let shorten = |text: &str, width| {
            crate::string::StrShortener::new(
                text,
                width,
                crate::string::ShortenDirection::Right,
                Some('…'),
            )
            .collect::<String>()
        };
        let label = shorten(label, label_width);
        format!(
            "{label}{}{}",
            " ".repeat(label_width.saturating_sub(label.width()) + 2),
            shorten(shortcut, key_width)
        )
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
            Row::IconChoices(left, right) => {
                if rect.height < 4 {
                    return std::iter::once(left)
                        .chain(right.as_ref())
                        .filter(|control| control.id == self.focus)
                        .map(|control| {
                            let mut part = element(control, rect);
                            let selected = matches!(control.kind, UiKind::IconCard(_, true));
                            part.0.kind = UiKind::Radio;
                            part.0.text =
                                format!("{} {}", if selected { "●" } else { "○" }, control.text);
                            part
                        })
                        .collect();
                }
                let width = rect.width.saturating_sub(2) / 2;
                let mut parts = vec![element(left, UiRect { width, ..rect })];
                if let Some(right) = right {
                    parts.push(element(
                        right,
                        UiRect {
                            left: rect.left + width + 2,
                            width: rect.width.saturating_sub(width + 2),
                            ..rect
                        },
                    ));
                }
                parts
            },
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
            Row::ShortcutHeader(label, shortcut) | Row::AgentHeader(label, shortcut) => self
                .row_parts(
                    &Row::Label(format!(
                        " {}",
                        Self::shortcut_row_text(label, shortcut, rect.width.saturating_sub(2))
                    )),
                    rect,
                ),
            Row::Shortcut { control, shortcut } => {
                let mut part = element(control, rect);
                part.0.text =
                    Self::shortcut_row_text(&control.text, shortcut, rect.width.saturating_sub(2));
                vec![part]
            },
            Row::Toolbar(left, right) => {
                let available = rect.width.saturating_sub(2);
                let left_width = (left.text.width() + 4).max(10).min(available / 2);
                let right_width = (right.text.width() + 4).max(10).min(available - left_width);
                vec![
                    element(left, UiRect { width: left_width, ..rect }),
                    element(
                        right,
                        UiRect { left: rect.left + left_width + 2, width: right_width, ..rect },
                    ),
                ]
            },
            Row::One(control) => {
                let width = if control.kind == UiKind::SecondaryButton {
                    (control.text.width() + 4).max(12).min(rect.width)
                } else {
                    rect.width
                };
                vec![element(control, UiRect { width, ..rect })]
            },
            Row::Adjustment { label, value, decrease, increase } => {
                if rect.width < 24 {
                    return self.row_parts(
                        &Row::Two(
                            Control::button(decrease.id, format!("− {value}")),
                            Control::button(increase.id, format!("{value} +")),
                        ),
                        rect,
                    );
                }
                let value_width = value.width().max(7);
                let group_width = value_width + 8;
                let left = rect.left + rect.width.saturating_sub(group_width);
                let label_width = left.saturating_sub(rect.left + 2);
                let label: String = crate::string::StrShortener::new(
                    label,
                    label_width,
                    crate::string::ShortenDirection::Right,
                    Some('…'),
                )
                .collect();
                let mut parts =
                    self.row_parts(&Row::Label(label), UiRect { width: label_width, ..rect });
                parts.push(element(decrease, UiRect { left, width: 3, ..rect }));
                parts.extend(self.row_parts(
                    &Row::Label(format!("{value:^value_width$}")),
                    UiRect { left: left + 4, width: value_width, ..rect },
                ));
                parts.push(element(
                    increase,
                    UiRect { left: left + value_width + 5, width: 3, ..rect },
                ));
                parts
            },
            Row::Two(left, right) if rect.width < 12 => {
                vec![element(if right.id == self.focus { right } else { left }, rect)]
            },
            Row::Two(left, right) if left.kind == UiKind::PrimaryButton => {
                let available = rect.width.saturating_sub(2);
                let left_width = (left.text.width() + 4).max(10).min(available / 2);
                let right_width = (right.text.width() + 4).max(10).min(available - left_width);
                let start = rect.left + rect.width - left_width - right_width - 2;
                vec![
                    element(left, UiRect { left: start, width: left_width, ..rect }),
                    element(
                        right,
                        UiRect { left: start + left_width + 2, width: right_width, ..rect },
                    ),
                ]
            },
            Row::Two(left, right) => {
                let left_width = rect.width.saturating_sub(2) / 2;
                vec![
                    element(left, UiRect { width: left_width, ..rect }),
                    element(
                        right,
                        UiRect {
                            left: rect.left + left_width + 2,
                            width: rect.width.saturating_sub(left_width + 2),
                            ..rect
                        },
                    ),
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

    fn fixed_body_rows(body: &[Row], geometry: PanelGeometry) -> usize {
        let count = body
            .iter()
            .position(|row| matches!(row, Row::ShortcutHeader(..) | Row::AgentHeader(..)))
            .map_or(0, |index| index + 1);
        let height: usize =
            body[..count].iter().map(|row| Self::row_height(row, geometry.control_height)).sum();
        if height < geometry.body.height { count } else { 0 }
    }

    fn scrolling_geometry(
        body: &[Row],
        count: usize,
        mut geometry: PanelGeometry,
    ) -> PanelGeometry {
        let height: usize =
            body[..count].iter().map(|row| Self::row_height(row, geometry.control_height)).sum();
        geometry.body.top += height;
        geometry.body.height = geometry.body.height.saturating_sub(height);
        geometry
    }

    fn scroll(&self, rows: &[Row], delta: isize, columns: usize, lines: usize) -> bool {
        let geometry = Self::geometry(rows, columns, lines);
        let body = &rows[geometry.navigation_rows..rows.len().saturating_sub(geometry.footer_rows)];
        let fixed = Self::fixed_body_rows(body, geometry);
        let geometry = Self::scrolling_geometry(body, fixed, geometry);
        let (offset, maximum) = self.offset(&body[fixed..], geometry);
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
        if columns < 24 || lines < 12 || geometry.body.height == 0 {
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
            (layout.top + layout.height - 2, UiKind::Status, footer_text),
        ] {
            if matches!(rows.first(), Some(Row::AgentHeader(..))) && kind == UiKind::Subtitle {
                continue;
            }
            parts.push((
                UiElement {
                    rect: UiRect { left: layout.left + inset, top, width: inner_width, height: 1 },
                    kind,
                    text: crate::string::StrShortener::new(
                        text,
                        inner_width,
                        crate::string::ShortenDirection::Right,
                        Some('…'),
                    )
                    .collect(),
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
                        text: crate::string::StrShortener::new(
                            &control.text,
                            rect.width.saturating_sub(2),
                            crate::string::ShortenDirection::Right,
                            Some('…'),
                        )
                        .collect(),
                        focused: control.id == self.focus,
                        hovered: Some(control.id) == self.hovered,
                        disabled: control.disabled,
                    },
                    Some(control.id),
                ));
            }
            if !geometry.wide {
                parts.push((
                    UiElement {
                        rect: UiRect {
                            left: layout.left + inset,
                            top: geometry.body.top - 1,
                            width: inner_width,
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
        let fixed = Self::fixed_body_rows(body, geometry);
        let mut position = 0;
        for row in &body[..fixed] {
            let height = Self::row_height(row, geometry.control_height);
            parts.extend(self.row_parts(
                row,
                UiRect { top: geometry.body.top + position, height, ..geometry.body },
            ));
            position += height;
        }
        let geometry = Self::scrolling_geometry(body, fixed, geometry);
        let body = &body[fixed..];
        let (offset, maximum) = self.offset(body, geometry);
        let mut position = 0;
        for row in body {
            let height = Self::row_height(row, geometry.control_height);
            let bottom = position + height;
            if bottom > offset && position < offset + geometry.body.height {
                let top = position.max(offset) - offset;
                let visible = bottom.min(offset + geometry.body.height) - position.max(offset);
                parts.extend(self.row_parts(
                    row,
                    UiRect { top: geometry.body.top + top, height: visible, ..geometry.body },
                ));
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
            parts.extend(self.row_parts(
                row,
                UiRect {
                    top: geometry.footer.top + index * geometry.control_height,
                    height: geometry.control_height,
                    ..geometry.footer
                },
            ));
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
            Key::Character(text)
                if crate::ui::shortcut_pressed(modifiers) && text.eq_ignore_ascii_case("a") =>
            {
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

struct ShortcutSwap {
    shortcut: String,
    physical: Option<PhysicalKey>,
    other: usize,
}

pub struct PreferencesDialog {
    pub original: Rc<UiConfig>,
    pub values: SettingsValues,
    pub status: Option<Message>,
    #[cfg(termust_updater)]
    pub update: UpdateStatus,
    pub locale: Locale,
    pub original_font_size: crossfont::Size,
    page: usize,
    shortcut_filter: TextInput,
    selected_shortcut: usize,
    capturing_shortcut: bool,
    pending_swap: Option<ShortcutSwap>,
    save_pressed: Option<Instant>,
    panel: Panel,
    family: TextInput,
    workspaces: Vec<String>,
    applications: crate::desktop_apps::InstalledApps,
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
            #[cfg(termust_updater)]
            update: UpdateStatus::Idle,
            page: 0,
            shortcut_filter: TextInput::new(String::new()),
            selected_shortcut: 0,
            capturing_shortcut: false,
            pending_swap: None,
            save_pressed: None,
            panel: Panel::default(),
            workspaces,
            applications: crate::desktop_apps::InstalledApps::detect(),
        }
    }

    pub fn rebase(&mut self, config: Rc<UiConfig>, scale_factor: f32) {
        let family = self.values.font_family.clone();
        self.values.rebase(&self.original, &config);
        if self.values.font_family != family {
            self.family = TextInput::new(self.values.font_family.clone());
        }
        if self.original.font != config.font {
            self.original_font_size = config.font.size().scale(scale_factor);
        }
        self.original = config;
        self.locale = self.values.language.resolve();
    }

    fn shortcuts() -> [(&'static str, Message, &'static str); 20] {
        [
            ("CreateNewTab", Message::NewTab, ""),
            ("RenameTab", Message::Rename, ""),
            ("CloseTab", Message::Close, ""),
            ("SelectPreviousTab", Message::PreviousTab, ""),
            ("SelectNextTab", Message::NextTab, ""),
            ("DuplicateTab", Message::DuplicateTab, ""),
            ("ReopenClosedTab", Message::ReopenClosedTab, ""),
            ("MoveTabLeft", Message::MoveTab, " ←"),
            ("MoveTabRight", Message::MoveTab, " →"),
            ("SplitVertical", Message::SplitVertical, " (vertical)"),
            ("SplitHorizontal", Message::SplitHorizontal, " (horizontal)"),
            ("FocusPaneLeft", Message::FocusPane, " ←"),
            ("FocusPaneRight", Message::FocusPane, " →"),
            ("FocusPaneUp", Message::FocusPane, " ↑"),
            ("FocusPaneDown", Message::FocusPane, " ↓"),
            ("TogglePaneZoom", Message::TogglePaneZoom, ""),
            ("ClosePane", Message::ClosePane, ""),
            ("OpenWorkspaces", Message::Workspaces, ""),
            ("SaveSession", Message::SaveAll, ""),
            ("OpenSettings", Message::Settings, ""),
        ]
    }

    #[cfg(termust_updater)]
    fn can_check_updates(&self) -> bool {
        matches!(
            self.update,
            UpdateStatus::Idle
                | UpdateStatus::Current
                | UpdateStatus::Available { .. }
                | UpdateStatus::Failed
        )
    }

    fn shortcut_bindings(&self) -> Vec<KeyBinding> {
        let mut overrides = self.original.general.shortcuts.clone();
        overrides.extend(self.values.shortcut_overrides.clone());
        resolved_shortcuts(&self.original, &overrides)
    }

    fn binding_label(&self, action: &str, bindings: &[KeyBinding]) -> String {
        let action =
            toml::Value::String(action.into()).try_into::<crate::config::Action>().unwrap();
        let shortcuts: Vec<_> = bindings
            .iter()
            .filter(|binding| binding.action == action)
            .filter_map(|binding| match &binding.trigger {
                crate::config::BindingKey::Keycode { key, location } => {
                    shortcut_text(key, binding.mods, *location)
                },
                trigger => Some(format!("{trigger:?}")),
            })
            .collect();
        if shortcuts.is_empty() {
            self.locale.text(Message::ShortcutDisabled).into()
        } else {
            shortcuts.join(" / ")
        }
    }

    fn shortcut_label(&self, action: &str) -> String {
        self.binding_label(action, &self.shortcut_bindings())
    }

    fn filtered_shortcuts(&self) -> Vec<(usize, String)> {
        let query = self.shortcut_filter.text.trim().to_lowercase();
        let bindings = self.shortcut_bindings();
        Self::shortcuts()
            .into_iter()
            .enumerate()
            .filter_map(|(index, (action, label, suffix))| {
                let shortcut = self.binding_label(action, &bindings);
                if !query.is_empty() {
                    let text = format!("{}{suffix} {action} {shortcut}", self.locale.text(label))
                        .to_lowercase();
                    if !query.split_whitespace().all(|word| text.contains(word)) {
                        return None;
                    }
                }
                Some((index, shortcut))
            })
            .collect()
    }

    fn filter_changed(&mut self) {
        if let Some((index, _)) = self.filtered_shortcuts().first() {
            self.selected_shortcut = *index;
        }
        self.panel.offset.set(0);
        self.panel.last_focus.set(None);
        self.status = None;
    }

    fn swapped_shortcuts(
        &self,
        shortcut: &str,
        physical: Option<PhysicalKey>,
    ) -> Option<(usize, SettingsValues)> {
        let (action, ..) = Self::shortcuts()[self.selected_shortcut];
        let bindings = self.shortcut_bindings();
        let previous = self.binding_label(action, &bindings);
        shortcut_binding(action, &previous)?;
        let (index, (other, ..)) =
            Self::shortcuts().into_iter().enumerate().find(|(_, (name, ..))| {
                *name != action && self.binding_label(name, &bindings) == shortcut
            })?;
        let mut values = self.values.clone();
        values.set_shortcut(other, previous, self.values.shortcut_physical.get(action).copied());
        values.set_shortcut(action, shortcut.into(), physical);
        values.shortcut_changes_valid(&self.original).then_some((index, values))
    }

    fn confirm_swap(&mut self) {
        if let Some(swap) = self.pending_swap.take() {
            if let Some((_, values)) = self
                .swapped_shortcuts(&swap.shortcut, swap.physical)
                .filter(|(other, _)| *other == swap.other)
            {
                self.values = values;
                self.finish_shortcut_capture();
            } else {
                self.status = Some(Message::ShortcutConflict);
            }
        }
    }

    fn capture_shortcut(
        &mut self,
        key: &Key,
        modifiers: ModifiersState,
        event_key: Option<(PhysicalKey, winit::keyboard::KeyLocation)>,
    ) {
        if *key == Key::Named(NamedKey::Enter)
            && modifiers.is_empty()
            && self.pending_swap.is_some()
        {
            self.confirm_swap();
            return;
        }
        if *key == Key::Named(NamedKey::Escape) {
            self.pending_swap = None;
            self.capturing_shortcut = false;
            self.status = None;
            return;
        }
        let location = event_key.map_or(KeyLocation::Any, |(_, location)| location.into());
        let Some(shortcut) = shortcut_text(key, modifiers, location) else {
            if !matches!(key, Key::Named(_)) {
                self.pending_swap = None;
                self.status = Some(Message::ShortcutInvalid);
            }
            return;
        };
        let physical = event_key.map(|(physical, _)| physical);
        self.pending_swap = None;
        let (action, ..) = Self::shortcuts()[self.selected_shortcut];
        if shortcut_binding(action, &shortcut).is_none() {
            self.status = Some(Message::ShortcutInvalid);
            return;
        }
        let mut overrides = self.original.general.shortcuts.clone();
        overrides.extend(self.values.shortcut_overrides.clone());
        overrides.insert(action.into(), shortcut.clone());
        if shortcut_conflict(&self.original, &overrides, action, &shortcut, physical) {
            if let Some((other, _)) = self.swapped_shortcuts(&shortcut, physical) {
                self.pending_swap = Some(ShortcutSwap { shortcut, physical, other });
            }
            self.status = Some(Message::ShortcutConflict);
            return;
        }
        self.values.set_shortcut(action, shortcut, physical);
        self.finish_shortcut_capture();
    }

    fn finish_shortcut_capture(&mut self) {
        self.capturing_shortcut = false;
        self.panel.focus = 100 + self.selected_shortcut;
        if !self.shortcut_filter.text.is_empty()
            && !self.filtered_shortcuts().iter().any(|(index, _)| *index == self.selected_shortcut)
        {
            self.shortcut_filter = TextInput::new(String::new());
            self.panel.focus = 26;
        }
        self.panel.last_focus.set(None);
        self.status = Some(Message::ShortcutChanged);
    }

    fn application_options(
        &self,
        kind: crate::desktop_apps::AppKind,
    ) -> Vec<(String, String, bool)> {
        use crate::desktop_apps::{self, AppKind};
        let preference = match kind {
            AppKind::Editor => &self.values.editor,
            AppKind::FileManager => &self.values.file_manager,
        };
        let automatic = match kind {
            AppKind::Editor => self.locale.text(Message::AppAutomatic),
            AppKind::FileManager => self.locale.text(Message::SystemDefault),
        };
        let mut options = vec![("auto".into(), automatic.into(), false)];
        options.extend(
            self.applications
                .choices(kind)
                .iter()
                .map(|app| (app.id.into(), app.name.into(), false)),
        );
        if preference != "auto" && !options.iter().any(|(id, ..)| id == preference) {
            if let Some(name) = desktop_apps::name(kind, preference) {
                options.push((
                    preference.clone(),
                    format!("{name} ({})", self.locale.text(Message::AppNotInstalled)),
                    true,
                ));
            }
        }
        options
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        let t = |key| self.locale.text(key);
        let navigation = |id, page, message| {
            let mut control = Control::button(id, t(message));
            control.kind = if self.page == page || (page == 0 && self.page == 8) {
                UiKind::SelectedNavigation
            } else {
                UiKind::Navigation
            };
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
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Row::Two(navigation(33, 6, Message::Agents), navigation(36, 7, Message::Applications)),
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            Row::One(navigation(33, 6, Message::Agents)),
        ];
        match self.page {
            0 => {
                rows.push(Row::One(Control {
                    id: 38,
                    kind: UiKind::AppearanceChoice {
                        label: t(Message::AppIcon),
                        icon: Some(self.values.icon),
                    },
                    text: self.values.icon.label(self.locale).into(),
                    disabled: false,
                }));
                let theme = match self.values.theme {
                    ThemeChoice::Keep => Message::CurrentTheme,
                    ThemeChoice::Dark => Message::DarkTheme,
                    ThemeChoice::Light => Message::LightTheme,
                };
                rows.push(Row::One(Control {
                    id: 8,
                    kind: UiKind::AppearanceChoice { label: t(Message::Theme), icon: None },
                    text: t(theme).into(),
                    disabled: false,
                }));
                rows.push(Row::Label(String::new()));
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
                rows.push(Row::Adjustment {
                    label: t(Message::FontSize).into(),
                    value: format!("{:.1} pt", self.values.font_size),
                    decrease: Control::button(4, "−"),
                    increase: Control::button(5, "+"),
                });
                rows.push(Row::Adjustment {
                    label: t(Message::Opacity).into(),
                    value: format!("{:.0}%", self.values.opacity * 100.),
                    decrease: Control::button(6, "−"),
                    increase: Control::button(7, "+"),
                });
            },
            8 => {
                rows.push(Row::Label(t(Message::AppIcon).into()));
                let control = |index: usize| {
                    let preset = IconPreset::ALL[index];
                    Control {
                        id: FIRST_ICON_CONTROL + index,
                        kind: UiKind::IconCard(preset, self.values.icon == preset),
                        text: preset.label(self.locale).into(),
                        disabled: false,
                    }
                };
                for index in (0..IconPreset::ALL.len()).step_by(2) {
                    rows.push(Row::IconChoices(
                        control(index),
                        (index + 1 < IconPreset::ALL.len()).then(|| control(index + 1)),
                    ));
                }
                rows.extend(Panel::labels(
                    t(Message::IconHint),
                    Panel::content_width(columns, lines, true),
                ));
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
                rows.push(Row::Adjustment {
                    label: t(Message::ScrollbackLines).into(),
                    value: self.values.scrollback_lines.to_string(),
                    decrease: Control::button(22, "−"),
                    increase: Control::button(23, "+"),
                });
                rows.push(Row::Label(t(Message::ScrollbackSaveHint).into()));
                rows.push(checkbox(
                    24,
                    self.values.hide_mouse_when_typing,
                    Message::HideMouseTyping,
                ));
                rows.push(checkbox(25, self.values.copy_on_select, Message::CopyOnSelect));
                #[cfg(termust_updater)]
                {
                    rows.push(Row::Label(String::new()));
                    rows.push(Row::Label(t(Message::Updates).into()));
                    let managed = !matches!(self.update, UpdateStatus::Unmanaged);
                    if managed {
                        rows.push(checkbox(
                            40,
                            self.values.check_updates,
                            Message::AutoCheckUpdates,
                        ));
                    }
                    let status = match &self.update {
                        UpdateStatus::Idle => String::new(),
                        UpdateStatus::Checking => t(Message::UpdateChecking).into(),
                        UpdateStatus::Current => t(Message::UpdateCurrent).into(),
                        UpdateStatus::Available { version } => {
                            t(Message::UpdateAvailable).replace("{version}", version)
                        },
                        UpdateStatus::Installing => t(Message::UpdateInstalling).into(),
                        UpdateStatus::Installed => t(Message::UpdateInstalled).into(),
                        UpdateStatus::Failed => t(Message::UpdateFailed).into(),
                        UpdateStatus::Unmanaged => t(Message::UpdateUnmanaged).into(),
                    };
                    rows.extend(Panel::labels(&status, Panel::content_width(columns, lines, true)));
                    if managed {
                        let mut check = Control::button(41, t(Message::CheckUpdatesNow));
                        check.disabled = !self.can_check_updates();
                        rows.push(Row::One(check));
                    }
                    if matches!(self.update, UpdateStatus::Available { .. }) {
                        rows.push(Row::One(Control::button(42, t(Message::InstallUpdate))));
                    }
                }
            },
            4 => {
                rows.push(checkbox(16, self.values.confirm_tab_close, Message::ConfirmTabClose));
                rows.extend(Panel::labels(
                    t(Message::TabSettingsHint),
                    Panel::content_width(columns, lines, true),
                ));
                rows.push(Row::Label(format!(
                    "{}   {}",
                    t(Message::DuplicateTab),
                    self.shortcut_label("DuplicateTab"),
                )));
                rows.push(Row::Label(format!(
                    "{}   {}",
                    t(Message::ReopenClosedTab),
                    self.shortcut_label("ReopenClosedTab"),
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
            6 => {
                rows.push(checkbox(34, self.values.agent_integration, Message::AgentIntegration));
                rows.push(checkbox(
                    35,
                    self.values.agent_notifications,
                    Message::AgentNotifications,
                ));
                rows.push(Row::Label(t(Message::AgentStatusIndicator).into()));
                let indicator = |id, animated, label| Control {
                    id,
                    kind: UiKind::Radio,
                    text: format!(
                        "{} {}",
                        if self.values.agent_status_animation == animated { "●" } else { "○" },
                        t(label),
                    ),
                    disabled: false,
                };
                rows.push(Row::Two(
                    indicator(43, true, Message::AgentAnimatedIndicator),
                    indicator(44, false, Message::AgentStaticIndicator),
                ));
                for message in
                    [Message::AgentSettingsHint, Message::AgentNewTabsHint, Message::AgentCodexHint]
                {
                    rows.push(Row::Label(String::new()));
                    rows.extend(Panel::labels(
                        t(message),
                        Panel::content_width(columns, lines, true),
                    ));
                }
            },
            7 => {
                use crate::desktop_apps::AppKind;
                for (kind, label, first, preference) in [
                    (AppKind::Editor, Message::PreferredEditor, 400, &self.values.editor),
                    (
                        AppKind::FileManager,
                        Message::PreferredFileManager,
                        500,
                        &self.values.file_manager,
                    ),
                ] {
                    rows.push(Row::Label(t(label).into()));
                    for (index, (id, label, disabled)) in
                        self.application_options(kind).into_iter().enumerate()
                    {
                        rows.push(Row::One(Control {
                            id: first + index,
                            kind: UiKind::Radio,
                            text: format!("{} {label}", if &id == preference { "●" } else { "○" }),
                            disabled,
                        }));
                    }
                    rows.push(Row::Label(String::new()));
                }
                rows.extend(Panel::labels(
                    t(Message::AppSettingsHint),
                    Panel::content_width(columns, lines, true),
                ));
            },
            _ => {
                let width = Panel::content_width(columns, lines, true).saturating_sub(2);
                let text = if self.shortcut_filter.text.is_empty() {
                    t(Message::ShortcutSearch).into()
                } else {
                    self.shortcut_filter.display(width, self.panel.focus == 26)
                };
                rows.push(Row::One(Control { id: 26, kind: UiKind::Input, text, disabled: false }));
                rows.push(Row::ShortcutHeader(
                    t(Message::ShortcutAction).into(),
                    t(Message::ShortcutKey).into(),
                ));
                let shortcuts = self.filtered_shortcuts();
                if shortcuts.is_empty() {
                    rows.push(Row::Label(t(Message::ShortcutNoResults).into()));
                }
                for &(index, ref shortcut) in &shortcuts {
                    let (_, label, suffix) = Self::shortcuts()[index];
                    let selected = index == self.selected_shortcut;
                    rows.push(Row::Shortcut {
                        control: Control {
                            id: 100 + index,
                            kind: if selected { UiKind::SelectedItem } else { UiKind::MenuItem },
                            text: format!("{}{suffix}", t(label)),
                            disabled: false,
                        },
                        shortcut: if selected && self.capturing_shortcut {
                            self.pending_swap.as_ref().map_or_else(
                                || t(Message::RecordShortcut).into(),
                                |swap| swap.shortcut.clone(),
                            )
                        } else {
                            shortcut.clone()
                        },
                    });
                }
                let mut clear = Control::button(
                    28,
                    t(if self.capturing_shortcut {
                        Message::ShortcutCancelCapture
                    } else {
                        Message::ShortcutClear
                    }),
                );
                clear.disabled = shortcuts.is_empty();
                let secondary = if self.capturing_shortcut && self.pending_swap.is_some() {
                    Control::button(27, t(Message::SwapShortcuts))
                } else {
                    Control::button(29, t(Message::Defaults))
                };
                rows.push(Row::Toolbar(clear, secondary));
            },
        }
        if self.page == 8 {
            rows.push(Row::One(Control::button(39, t(Message::IconRestore))));
        } else if self.page != 2 {
            rows.push(Row::One(Control::button(12, t(Message::Defaults))));
        }
        let mut save = Control::button(14, t(Message::SaveSettings));
        save.kind = UiKind::PrimaryButton;
        rows.push(Row::Two(save, Control::button(15, t(Message::Cancel))));
        rows
    }

    pub fn save_pulse(&self) -> u8 {
        let Some(start) = self.save_pressed else { return 0 };
        let remaining = (1. - start.elapsed().as_secs_f32() / 0.36).max(0.);
        (255. * remaining * remaining).ceil() as u8
    }

    pub fn settings_saved(&mut self, saved: bool) {
        if saved {
            self.values
                .shortcut_overrides
                .retain(|action, value| self.original.general.shortcuts.get(action) != Some(value));
            self.values
                .shortcut_physical
                .retain(|action, _| self.values.shortcut_overrides.contains_key(action));
        }
        if self.status != Some(Message::SavingSettings) {
            return;
        }
        self.save_pressed = Some(Instant::now());
        self.status =
            Some(if saved { Message::SettingsSaved } else { Message::SettingsSaveFailed });
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        if id == 14 {
            self.save_pressed = Some(Instant::now());
            if self.capturing_shortcut {
                self.panel.focus = 100 + self.selected_shortcut;
                self.status.get_or_insert(Message::ShortcutCapture);
                return None;
            }
        }
        if id == 27 {
            self.confirm_swap();
            return None;
        }
        self.pending_swap = None;
        self.status = None;
        match id {
            0..=2 | 13 | 20 | 21 | 33 | 36 | 38 => {
                self.page = match id {
                    20 => 3,
                    21 => 4,
                    13 => 5,
                    33 => 6,
                    36 => 7,
                    38 => 8,
                    _ => id,
                };
                self.panel.focus = match self.page {
                    2 => 26,
                    8 => FIRST_ICON_CONTROL + self.values.icon as usize,
                    _ => id,
                };
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
                    self.status = Some(Message::SavingSettings);
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
            26 => self.capturing_shortcut = false,
            34 => self.values.agent_integration = !self.values.agent_integration,
            35 => self.values.agent_notifications = !self.values.agent_notifications,
            39 => self.values.icon = IconPreset::Termust,
            #[cfg(termust_updater)]
            40 if !matches!(self.update, UpdateStatus::Unmanaged) => {
                self.values.check_updates = !self.values.check_updates
            },
            #[cfg(termust_updater)]
            41 if self.can_check_updates() => {
                self.update = UpdateStatus::Checking;
                return Some(ProductivityAction::CheckUpdates);
            },
            #[cfg(termust_updater)]
            42 => {
                let UpdateStatus::Available { version } = &self.update else { return None };
                let action = ProductivityAction::InstallUpdate(version.clone());
                self.update = UpdateStatus::Installing;
                return Some(action);
            },
            43 => self.values.agent_status_animation = true,
            44 => self.values.agent_status_animation = false,
            id if (FIRST_ICON_CONTROL..FIRST_ICON_CONTROL + IconPreset::ALL.len()).contains(&id) => {
                self.values.icon = IconPreset::ALL[id - FIRST_ICON_CONTROL];
            },
            400..=599 => {
                use crate::desktop_apps::AppKind;
                let (kind, first) =
                    if id < 500 { (AppKind::Editor, 400) } else { (AppKind::FileManager, 500) };
                if let Some((value, _, false)) = self.application_options(kind).get(id - first) {
                    let value = value.clone();
                    match kind {
                        AppKind::Editor => self.values.editor = value,
                        AppKind::FileManager => self.values.file_manager = value,
                    }
                }
            },
            29 => {
                self.values.shortcut_overrides = default_shortcuts();
                self.values.shortcut_physical.clear();
                self.capturing_shortcut = false;
                self.status = Some(Message::ShortcutChanged);
            },
            28 => {
                if self.capturing_shortcut {
                    self.capturing_shortcut = false;
                } else {
                    let (action, ..) = Self::shortcuts()[self.selected_shortcut];
                    self.values.set_shortcut(action, String::new(), None);
                    self.status = Some(Message::ShortcutChanged);
                }
            },
            id if (100..100 + Self::shortcuts().len()).contains(&id) => {
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

pub struct GroupDialog {
    pub tab_id: crate::tabs::TabId,
    pub locale: Locale,
    pub status: Option<Message>,
    groups: Vec<(String, usize)>,
    selected: Option<String>,
    current: Option<String>,
    panel: Panel,
    name: TextInput,
}

impl GroupDialog {
    pub fn new(
        tab_id: crate::tabs::TabId,
        groups: Vec<(String, usize)>,
        current: Option<String>,
        locale: Locale,
    ) -> Self {
        let selected = current.clone().or_else(|| groups.first().map(|(name, _)| name.clone()));
        Self {
            tab_id,
            locale,
            status: None,
            name: TextInput::new(selected.clone().unwrap_or_default()),
            groups,
            selected,
            current,
            panel: Panel::default(),
        }
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        let t = |message| self.locale.text(message);
        let mut create = Control::button(1, t(Message::CreateGroup));
        create.kind = UiKind::PrimaryButton;
        let mut rename = Control::button(2, t(Message::RenameGroup));
        rename.disabled = self.selected.is_none();
        let mut rows = vec![
            Row::Label(t(Message::GroupName).into()),
            Row::One(Control {
                id: 0,
                kind: UiKind::Input,
                text: self.name.display(
                    Panel::content_width(columns, lines, false).saturating_sub(2),
                    self.panel.focus == 0,
                ),
                disabled: false,
            }),
            Row::Two(create, rename),
        ];
        if self.groups.is_empty() {
            rows.push(Row::Label(t(Message::NoGroups).into()));
        }
        for (index, (name, count)) in self.groups.iter().enumerate() {
            rows.push(Row::One(Control {
                id: 100 + index,
                kind: UiKind::Radio,
                text: format!(
                    "{} {} ({count})",
                    if self.selected.as_ref() == Some(name) { "●" } else { "○" },
                    name,
                ),
                disabled: false,
            }));
        }
        let mut move_tab = Control::button(3, t(Message::MoveToGroup));
        let mut show = Control::button(4, t(Message::ShowGroup));
        move_tab.disabled = self.selected.is_none();
        show.disabled = self.selected.is_none();
        rows.push(Row::Two(move_tab, show));
        let mut ungroup = Control::button(5, t(Message::UngroupTab));
        ungroup.disabled = self.current.is_none();
        rows.push(Row::Two(ungroup, Control::button(6, t(Message::AllTabs))));
        rows.push(Row::One(Control::button(7, t(Message::Cancel))));
        rows
    }

    fn validated_name(&mut self, rename: bool) -> Option<String> {
        let name = self.name.text.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            self.status = Some(Message::GroupInvalid);
            self.panel.focus = 0;
            return None;
        }
        if self.groups.iter().any(|(existing, _)| {
            existing == name && !(rename && self.selected.as_ref() == Some(existing))
        }) {
            self.status = Some(Message::GroupDuplicate);
            self.panel.focus = 0;
            return None;
        }
        Some(name.into())
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        self.status = None;
        match id {
            1 => self.validated_name(false).map(ProductivityAction::GroupCreate),
            2 if self.selected.is_some() => {
                let new = self.validated_name(true)?;
                Some(ProductivityAction::GroupRename { old: self.selected.clone().unwrap(), new })
            },
            3 if self.selected.is_some() => {
                Some(ProductivityAction::GroupMove(self.selected.clone()))
            },
            4 if self.selected.is_some() => {
                Some(ProductivityAction::GroupShow(self.selected.clone()))
            },
            5 if self.current.is_some() => Some(ProductivityAction::GroupMove(None)),
            6 => Some(ProductivityAction::GroupShow(None)),
            7 => Some(ProductivityAction::Cancel),
            100.. => {
                if let Some((name, _)) = self.groups.get(id - 100) {
                    self.selected = Some(name.clone());
                    self.name = TextInput::new(name.clone());
                }
                None
            },
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRow {
    pub tab_id: crate::tabs::TabId,
    pub pane_id: crate::tabs::TabId,
    pub title: String,
    pub provider: String,
    pub status: String,
    pub state: crate::agent::AgentState,
    pub limited: bool,
    pub unread: bool,
}

pub struct AgentDialog {
    pub locale: Locale,
    pub agents: Vec<AgentRow>,
    panel: Panel,
}

impl AgentDialog {
    pub fn new(locale: Locale, mut agents: Vec<AgentRow>) -> Self {
        agents.sort_by_key(|agent| std::cmp::Reverse(agent.state.priority()));
        let focus = if agents.is_empty() { 0 } else { 100 };
        Self { locale, agents, panel: Panel { focus, ..Panel::default() } }
    }

    pub fn update(&mut self, mut agents: Vec<AgentRow>) {
        agents.sort_by_key(|agent| std::cmp::Reverse(agent.state.priority()));
        let selected = self
            .panel
            .focus
            .checked_sub(100)
            .and_then(|index| self.agents.get(index))
            .map(|agent| (agent.tab_id, agent.pane_id));
        self.agents = agents;
        let focus = selected
            .and_then(|selected| {
                self.agents.iter().position(|agent| (agent.tab_id, agent.pane_id) == selected)
            })
            .map_or(0, |index| 100 + index);
        if self.panel.focus != focus {
            self.panel.focus = focus;
            self.panel.last_focus.set(None);
        }
    }

    fn rows(&self, columns: usize, _lines: usize) -> Vec<Row> {
        let t = |message| self.locale.text(message);
        let mut rows =
            vec![Row::AgentHeader(t(Message::AgentChats).into(), t(Message::AgentStatus).into())];
        if self.agents.is_empty() {
            for message in [Message::AgentEmpty, Message::AgentStartHint] {
                rows.extend(Panel::labels(
                    t(message),
                    columns.saturating_sub(2).min(56).saturating_sub(6),
                ));
            }
        }
        for (index, agent) in self.agents.iter().enumerate() {
            rows.push(Row::One(Control {
                id: 100 + index,
                kind: if agent.unread { UiKind::SelectedItem } else { UiKind::MenuItem },
                text: format!(
                    "{}{} · {}",
                    if agent.unread { "● " } else { "" },
                    agent.provider,
                    agent.title
                ),
                disabled: false,
            }));
            if agent.limited {
                rows.extend(Panel::labels(
                    t(Message::AgentLimited),
                    columns.saturating_sub(2).min(56).saturating_sub(6),
                ));
            }
        }
        rows.push(Row::One(Control::button(0, t(Message::AgentClose))));
        rows
    }

    fn activate(&self, id: usize) -> Option<ProductivityAction> {
        if id == 0 {
            return Some(ProductivityAction::Cancel);
        }
        let agent = self.agents.get(id.checked_sub(100)?)?;
        Some(ProductivityAction::FocusAgent { tab_id: agent.tab_id, pane_id: agent.pane_id })
    }
}

pub enum ProductivityDialog {
    Preferences(Box<PreferencesDialog>),
    Workspaces(WorkspaceDialog),
    Groups(GroupDialog),
    Agents(AgentDialog),
}

impl ProductivityDialog {
    pub fn locale_mut(&mut self) -> &mut Locale {
        match self {
            Self::Preferences(dialog) => &mut dialog.locale,
            Self::Workspaces(dialog) => &mut dialog.locale,
            Self::Groups(dialog) => &mut dialog.locale,
            Self::Agents(dialog) => &mut dialog.locale,
        }
    }

    fn rows(&self, columns: usize, lines: usize) -> Vec<Row> {
        match self {
            Self::Preferences(dialog) => dialog.rows(columns, lines),
            Self::Workspaces(dialog) => dialog.rows(columns, lines),
            Self::Groups(dialog) => dialog.rows(columns, lines),
            Self::Agents(dialog) => dialog.rows(columns, lines),
        }
    }

    fn panel(&self) -> &Panel {
        match self {
            Self::Preferences(dialog) => &dialog.panel,
            Self::Workspaces(dialog) => &dialog.panel,
            Self::Groups(dialog) => &dialog.panel,
            Self::Agents(dialog) => &dialog.panel,
        }
    }

    fn panel_mut(&mut self) -> &mut Panel {
        match self {
            Self::Preferences(dialog) => &mut dialog.panel,
            Self::Workspaces(dialog) => &mut dialog.panel,
            Self::Groups(dialog) => &mut dialog.panel,
            Self::Agents(dialog) => &mut dialog.panel,
        }
    }

    fn activate(&mut self, id: usize) -> Option<ProductivityAction> {
        match self {
            Self::Preferences(dialog) => dialog.activate(id),
            Self::Workspaces(dialog) => dialog.activate(id),
            Self::Groups(dialog) => dialog.activate(id),
            Self::Agents(dialog) => dialog.activate(id),
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        Panel::layout(&self.rows(columns, lines), columns, lines)
    }

    fn parts(&self, columns: usize, lines: usize) -> Vec<(UiElement, Option<usize>)> {
        let (locale, title, subtitle, status) = match self {
            Self::Preferences(d) => {
                let subtitle = if d.capturing_shortcut {
                    Message::ShortcutCapture
                } else if d.page == 2 {
                    Message::ShortcutsSubtitle
                } else {
                    Message::PreferencesSubtitle
                };
                (d.locale, Message::SettingsTitle, subtitle, d.status)
            },
            Self::Workspaces(d) => {
                (d.locale, Message::Workspaces, Message::WorkspacesHint, d.status)
            },
            Self::Groups(d) => (d.locale, Message::TabGroups, Message::GroupsHint, d.status),
            Self::Agents(d) => (d.locale, Message::AgentChats, Message::AgentOverviewHint, None),
        };
        let title = if let Self::Agents(dialog) = self {
            format!("{} · {}", locale.text(title), dialog.agents.len())
        } else {
            locale.text(title).into()
        };
        let mut parts = self.panel().parts(
            &self.rows(columns, lines),
            &title,
            locale.text(subtitle),
            locale.text(status.unwrap_or(if columns < 64 || matches!(self, Self::Agents(_)) {
                Message::KeyboardHelpCompact
            } else {
                Message::KeyboardHelp
            })),
            columns,
            lines,
        );
        if let Self::Preferences(dialog) = self {
            for (element, id) in &mut parts {
                if *id == Some(14) {
                    let pulse = dialog.save_pulse();
                    if pulse > 0 {
                        element.kind = UiKind::PressedButton(pulse);
                    }
                    let label = match dialog.status {
                        Some(Message::SavingSettings) => Message::SavingSettings,
                        Some(Message::SettingsSaved | Message::IconIntegrationFailed) => {
                            Message::SavedButton
                        },
                        Some(Message::SettingsSaveFailed) => Message::RetryButton,
                        _ => continue,
                    };
                    element.text = locale.text(label).into();
                } else if element.kind == UiKind::Status
                    && matches!(
                        dialog.status,
                        Some(
                            Message::SettingsSaveFailed
                                | Message::SettingsInvalid
                                | Message::ShortcutInvalid
                                | Message::ShortcutConflict
                                | Message::IconIntegrationFailed
                        )
                    )
                {
                    element.kind = UiKind::Error;
                    if let Some(swap) = &dialog.pending_swap {
                        let (_, label, suffix) = PreferencesDialog::shortcuts()[swap.other];
                        element.text = format!(
                            "{}{suffix} · {}",
                            locale.text(label),
                            locale.text(Message::ShortcutSwapHint)
                        );
                    }
                }
            }
        }
        if let Self::Agents(dialog) = self {
            let mut statuses = Vec::new();
            for (element, id) in &mut parts {
                let Some(agent) = id
                    .and_then(|id| id.checked_sub(100))
                    .and_then(|index| dialog.agents.get(index))
                else {
                    continue;
                };
                let status_width =
                    (agent.status.width() + 2).min(element.rect.width.saturating_sub(4) / 2);
                element.text = crate::string::StrShortener::new(
                    &element.text,
                    element.rect.width.saturating_sub(status_width + 4),
                    crate::string::ShortenDirection::Right,
                    Some('…'),
                )
                .collect();
                if status_width > 0 {
                    statuses.push((
                        UiElement {
                            rect: UiRect {
                                left: element.rect.left + element.rect.width - status_width - 2,
                                width: status_width + 2,
                                ..element.rect
                            },
                            kind: UiKind::AgentStatus(agent.state, agent.unread),
                            text: agent.status.clone(),
                            focused: element.focused,
                            hovered: element.hovered,
                            disabled: false,
                        },
                        *id,
                    ));
                    element.rect.width -= status_width + 2;
                }
            }
            parts.extend(statuses);
        }
        parts
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
                let key = crate::input::keyboard::binding_key(event, modifiers);
                dialog.capture_shortcut(
                    &key,
                    modifiers,
                    Some((event.physical_key, event.location)),
                );
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
        if let Self::Preferences(dialog) = self {
            if *key == Key::Named(NamedKey::Enter) && crate::ui::shortcut_pressed(modifiers) {
                return dialog.activate(14);
            }
            if dialog.page == 2 {
                if crate::ui::shortcut_pressed(modifiers)
                    && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("f"))
                {
                    dialog.panel.focus = 26;
                    dialog.shortcut_filter.selected_all = true;
                    dialog.panel.last_focus.set(None);
                    return None;
                }
                if dialog.panel.focus == 26 && *key == Key::Named(NamedKey::Enter) {
                    if let Some((index, _)) = dialog.filtered_shortcuts().first() {
                        dialog.panel.focus = 100 + *index;
                        return dialog.activate(100 + *index);
                    }
                    return None;
                }
                if dialog.panel.focus == 26 && dialog.shortcut_filter.key(key, modifiers, 128) {
                    dialog.filter_changed();
                    return None;
                }
                if dialog.panel.focus != 26
                    && matches!(key, Key::Named(NamedKey::Home | NamedKey::End))
                {
                    let shortcuts = dialog.filtered_shortcuts();
                    let target = if *key == Key::Named(NamedKey::Home) {
                        shortcuts.first()
                    } else {
                        shortcuts.last()
                    };
                    if let Some((index, _)) = target {
                        dialog.panel.focus = 100 + index;
                        dialog.panel.last_focus.set(None);
                    }
                    return None;
                }
            }
        }
        if *key == Key::Named(NamedKey::Escape) {
            return Some(ProductivityAction::Cancel);
        }
        if let Self::Groups(dialog) = self {
            if dialog.panel.focus == 0 && *key == Key::Named(NamedKey::Enter) {
                return dialog.activate(1);
            }
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
            Self::Groups(dialog) if focus == 0 => Some((&mut dialog.name, 64)),
            _ => None,
        };
        if let Some((input, limit)) = input {
            if input.key(key, modifiers, limit) {
                if let Self::Preferences(dialog) = self {
                    dialog.values.font_family = dialog.family.text.clone();
                    dialog.status = None;
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
            Self::Preferences(dialog)
                if dialog.page == 2 && dialog.panel.focus == 26 && !dialog.capturing_shortcut =>
            {
                dialog.shortcut_filter.key(
                    &Key::Character(text.into()),
                    ModifiersState::empty(),
                    128,
                );
                dialog.filter_changed();
            },
            Self::Preferences(dialog) if dialog.panel.focus == 3 => {
                dialog.family.key(&Key::Character(text.into()), ModifiersState::empty(), 128);
                dialog.values.font_family = dialog.family.text.clone();
                dialog.status = None;
            },
            Self::Workspaces(dialog) if dialog.panel.focus == 0 => {
                dialog.name.key(&Key::Character(text.into()), ModifiersState::empty(), 80);
            },
            Self::Groups(dialog) if dialog.panel.focus == 0 => {
                dialog.name.key(&Key::Character(text.into()), ModifiersState::empty(), 64);
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
    #[test]
    fn appearance_icon_preview_is_visible_and_opens_the_gallery() {
        for (columns, lines) in [(100, 32), (64, 28), (45, 18)] {
            let mut preferences = PreferencesDialog::new(
                Rc::new(UiConfig::default()),
                crossfont::Size::new(11.),
                Vec::new(),
            );
            preferences.locale = Locale::PtBr;
            preferences.values.icon = IconPreset::Dog;
            let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
            let parts = dialog.parts(columns, lines);
            let preview = &parts.iter().find(|(_, id)| *id == Some(38)).unwrap().0;
            let theme = &parts.iter().find(|(_, id)| *id == Some(8)).unwrap().0;
            assert_eq!(preview.text, IconPreset::Dog.label(Locale::PtBr));
            assert_eq!(preview.rect.left, theme.rect.left);
            assert_eq!(preview.rect.width, theme.rect.width);
            assert!(preview.rect.height >= 2);
            assert_eq!(
                preview.kind,
                UiKind::AppearanceChoice {
                    label: Locale::PtBr.text(Message::AppIcon),
                    icon: Some(IconPreset::Dog),
                },
            );
            dialog.click(preview.rect.left + 1, preview.rect.top + 1, columns, lines);
            assert_eq!(dialog.panel().focus, FIRST_ICON_CONTROL + IconPreset::Dog as usize);
            dialog.activate(FIRST_ICON_CONTROL + IconPreset::Yorkie as usize);
            dialog.activate(0);
            assert!(dialog.parts(columns, lines).iter().any(|(element, id)| {
                *id == Some(38)
                    && matches!(
                        element.kind,
                        UiKind::AppearanceChoice { icon: Some(IconPreset::Yorkie), .. }
                    )
            }));
        }
    }

    #[test]
    fn icon_gallery_preserves_other_drafts_and_keeps_cards_and_actions_reachable() {
        for (columns, lines) in [(120, 40), (80, 32), (60, 28), (40, 24)] {
            let mut preferences = super::PreferencesDialog::new(
                std::rc::Rc::new(crate::config::UiConfig::default()),
                crossfont::Size::new(11.),
                Vec::new(),
            );
            preferences.values.font_size = 22.;
            preferences.activate(38);
            for (index, preset) in IconPreset::ALL.into_iter().enumerate() {
                let id = FIRST_ICON_CONTROL + index;
                preferences.panel.focus = id;
                preferences.activate(id);
                let rows = preferences.rows(columns, lines);
                let geometry = super::Panel::geometry(&rows, columns, lines);
                let parts =
                    preferences.panel.parts(&rows, "Settings", "Preview", "Help", columns, lines);
                let (element, _) = parts.iter().find(|(_, control)| *control == Some(id)).unwrap();
                assert!(element.rect.top >= geometry.body.top);
                assert!(element.rect.top + element.rect.height <= geometry.footer.top);
                assert_eq!(preferences.values.icon, preset);
            }
            preferences.activate(39);
            assert_eq!(preferences.values.icon, IconPreset::Termust);
            assert_eq!(preferences.values.font_size, 22.);
        }
    }

    use super::*;

    fn agent_rows() -> Vec<AgentRow> {
        (0..24)
            .map(|index| AgentRow {
                tab_id: 10 + index,
                pane_id: 50 + index,
                title: format!("Task {index}"),
                provider: "Codex".into(),
                status: Locale::En.text(Message::AgentPermission).into(),
                state: crate::agent::AgentState::Permission,
                limited: index == 2,
                unread: index == 1,
            })
            .collect()
    }

    #[test]
    fn unconstrained_agent_overview_shows_all_rows_without_a_hidden_footer_overlap() {
        let rows = agent_rows().into_iter().take(2).collect();
        let dialog = ProductivityDialog::Agents(AgentDialog::new(Locale::En, rows));
        let parts = dialog.parts(120, 36);
        for id in [100, 101, 0] {
            assert!(parts.iter().any(|(_, control)| *control == Some(id)));
        }
        assert!(!parts.iter().any(|(element, _)| element.kind == UiKind::Scrollbar));
    }

    #[test]
    fn agent_overview_focuses_stable_pane_and_keeps_it_after_updates() {
        let mut dialog = ProductivityDialog::Agents(AgentDialog::new(Locale::En, agent_rows()));
        dialog.logical_key(&Key::Named(NamedKey::ArrowDown), ModifiersState::empty());
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()),
            Some(ProductivityAction::FocusAgent { tab_id: 11, pane_id: 51 })
        );
        let ProductivityDialog::Agents(agents) = &mut dialog else { unreachable!() };
        let mut rows = agent_rows();
        rows.reverse();
        agents.update(rows);
        assert_eq!(
            agents.activate(agents.panel.focus),
            Some(ProductivityAction::FocusAgent { tab_id: 11, pane_id: 51 })
        );
        agents.update(Vec::new());
        assert_eq!(agents.activate(agents.panel.focus), Some(ProductivityAction::Cancel));
    }

    #[test]
    fn chat_popup_prioritizes_waiting_panes_without_losing_selection() {
        use crate::agent::AgentState;
        let mut rows: Vec<_> = agent_rows().into_iter().take(3).collect();
        rows[0].state = AgentState::Working;
        rows[1].state = AgentState::Input;
        rows[2].state = AgentState::Permission;
        let mut dialog = AgentDialog::new(Locale::En, rows.clone());
        assert_eq!(dialog.agents.iter().map(|row| row.pane_id).collect::<Vec<_>>(), [52, 51, 50]);
        rows[0].state = AgentState::Permission;
        rows[2].state = AgentState::Ready;
        dialog.update(rows);
        assert_eq!(
            dialog.activate(dialog.panel.focus),
            Some(ProductivityAction::FocusAgent { tab_id: 12, pane_id: 52 })
        );
    }

    #[test]
    fn chat_popup_stays_compact_and_status_cells_focus_the_same_pane() {
        let mut dialog = ProductivityDialog::Agents(AgentDialog::new(Locale::En, agent_rows()));
        let rows = dialog.rows(160, 50);
        let rect = Panel::layout(&rows, 160, 50);
        assert!(rect.width <= 56 && rect.height <= 16);
        assert_eq!(rect.top, 0);
        assert_eq!(rect.left + rect.width, 159);
        let parts = dialog.parts(160, 50);
        let status = parts
            .iter()
            .find(|(element, _)| matches!(element.kind, UiKind::AgentStatus(..)))
            .unwrap();
        assert_eq!(status.1, Some(100));
        assert_eq!(
            dialog.click(status.0.rect.left, status.0.rect.top, 160, 50),
            Some(ProductivityAction::FocusAgent { tab_id: 10, pane_id: 50 })
        );
    }

    #[test]
    fn agent_overview_keeps_rows_and_footer_reachable_at_all_sizes() {
        for locale in [Locale::En, Locale::PtBr] {
            for (columns, lines) in [(120, 40), (73, 25), (45, 22), (24, 12), (18, 7)] {
                let mut dialog = ProductivityDialog::Agents(AgentDialog::new(locale, agent_rows()));
                for focus in (100..124).chain(std::iter::once(0)) {
                    dialog.panel_mut().focus = focus;
                    let parts = dialog.parts(columns, lines);
                    assert!(
                        parts.iter().any(|(_, id)| *id == Some(focus)),
                        "{columns}x{lines} focus={focus}"
                    );
                    let mut occupied = vec![false; columns * lines];
                    for (element, _) in parts {
                        assert!(element.rect.left + element.rect.width <= columns);
                        assert!(element.rect.top + element.rect.height <= lines);
                        for y in element.rect.top..element.rect.top + element.rect.height {
                            for x in element.rect.left..element.rect.left + element.rect.width {
                                assert!(!occupied[y * columns + x], "overlap {columns}x{lines}");
                                occupied[y * columns + x] = true;
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn application_choices_list_installed_apps_and_preserve_missing_preferences() {
        use crate::desktop_apps::{AppKind, InstalledApps};

        let root = tempfile::tempdir().unwrap();
        #[cfg(target_os = "linux")]
        let (applications, managers) = {
            use std::os::unix::fs::PermissionsExt;
            for name in ["code", "zed", "nautilus", "dolphin"] {
                let path = root.path().join(name);
                std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            (
                InstalledApps::detect_in(root.path().as_os_str(), &[]),
                ["auto", "nautilus", "dolphin"],
            )
        };
        #[cfg(target_os = "macos")]
        let (applications, managers) = {
            for name in ["Visual Studio Code.app", "Zed.app", "Finder.app"] {
                let contents = root.path().join(name).join("Contents");
                std::fs::create_dir_all(&contents).unwrap();
                std::fs::write(contents.join("Info.plist"), "").unwrap();
            }
            (InstalledApps::detect_bundles(&[root.path().into()]), ["auto", "finder"])
        };
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        preferences.applications = applications;
        assert_eq!(
            preferences
                .application_options(AppKind::Editor)
                .iter()
                .map(|(id, ..)| id.as_str())
                .collect::<Vec<_>>(),
            ["auto", "code", "zed"]
        );
        assert_eq!(
            preferences
                .application_options(AppKind::FileManager)
                .iter()
                .map(|(id, ..)| id.as_str())
                .collect::<Vec<_>>(),
            managers
        );
        preferences.activate(36);
        preferences.activate(402);
        preferences.activate(500 + managers.len() - 1);
        assert_eq!(preferences.values.editor, "zed");
        assert_eq!(preferences.values.file_manager, managers[managers.len() - 1]);
        assert_eq!(preferences.original.general.editor, "auto");
        assert_eq!(preferences.original.general.file_manager, "auto");

        preferences.values.editor = "codium".into();
        let missing = preferences.application_options(AppKind::Editor);
        assert!(missing.last().unwrap().2);
        preferences.activate(402);
        assert_eq!(preferences.values.editor, "zed");
        preferences.values.editor = "codium".into();
        preferences.activate(403);
        assert_eq!(preferences.values.editor, "codium");
        let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
        for (columns, lines) in [(100, 32), (45, 18), (67, 13), (12, 6)] {
            for _ in 0..24 {
                let parts = dialog.parts(columns, lines);
                assert!(
                    parts
                        .iter()
                        .all(|(element, _)| element.rect.left + element.rect.width <= columns
                            && element.rect.top + element.rect.height <= lines)
                );
                assert!(parts.iter().any(|(element, _)| element.focused));
                dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
            }
        }
    }

    #[test]
    fn agent_preferences_append_after_shortcuts_and_apply_only_on_save() {
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        assert_eq!(
            Panel::controls(&preferences.rows(120, 40))
                .iter()
                .filter(|control| matches!(
                    control.kind,
                    UiKind::Navigation | UiKind::SelectedNavigation
                ))
                .map(|control| control.id)
                .collect::<Vec<_>>(),
            vec![
                0,
                20,
                21,
                1,
                13,
                2,
                33,
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                36
            ]
        );
        preferences.activate(33);
        assert_eq!(preferences.page, 6);
        preferences.activate(34);
        preferences.activate(35);
        let Some(ProductivityAction::SaveSettings(values)) = preferences.activate(14) else {
            panic!("settings not saved")
        };
        assert!(!values.agent_integration && !values.agent_notifications);
        assert!(
            preferences.original.general.agent_integration
                && preferences.original.general.agent_notifications
        );
        let mut empty = ProductivityDialog::Agents(AgentDialog::new(Locale::PtBr, Vec::new()));
        assert!(
            empty.elements(73, 25).iter().any(|element| element.text.contains("Nenhum agente"))
        );
        assert_eq!(
            empty.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
            Some(ProductivityAction::Cancel)
        );
    }

    #[test]
    fn group_actions_preserve_target_and_validate_names() {
        let mut dialog = GroupDialog::new(
            42,
            vec![("Work".into(), 2), ("Pessoal".into(), 1)],
            Some("Work".into()),
            Locale::En,
        );
        assert_eq!(dialog.tab_id, 42);
        dialog.name = TextInput::new("  新しい projeto  ".into());
        assert_eq!(
            dialog.activate(1),
            Some(ProductivityAction::GroupCreate("新しい projeto".into()))
        );
        assert_eq!(
            dialog.activate(2),
            Some(ProductivityAction::GroupRename {
                old: "Work".into(),
                new: "新しい projeto".into()
            })
        );
        dialog.name = TextInput::new("Pessoal".into());
        assert_eq!(dialog.activate(1), None);
        assert_eq!(dialog.status, Some(Message::GroupDuplicate));
        assert_eq!(dialog.activate(2), None);
        assert_eq!(dialog.status, Some(Message::GroupDuplicate));
        for text in [" ".into(), "a".repeat(65), "line\nbreak".into()] {
            dialog.name = TextInput::new(text);
            assert_eq!(dialog.activate(1), None);
            assert_eq!(dialog.status, Some(Message::GroupInvalid));
        }
        dialog.activate(101);
        assert_eq!(dialog.activate(3), Some(ProductivityAction::GroupMove(Some("Pessoal".into()))));
        assert_eq!(dialog.activate(4), Some(ProductivityAction::GroupShow(Some("Pessoal".into()))));
        assert_eq!(dialog.activate(5), Some(ProductivityAction::GroupMove(None)));
        assert_eq!(dialog.activate(6), Some(ProductivityAction::GroupShow(None)));
        assert_eq!(dialog.activate(7), Some(ProductivityAction::Cancel));
    }

    #[test]
    fn group_input_supports_unicode_keyboard_paste_and_cancel() {
        let mut dialog =
            ProductivityDialog::Groups(GroupDialog::new(7, Vec::new(), None, Locale::PtBr));
        dialog.paste("produção");
        dialog.logical_key(&Key::Named(NamedKey::Space), ModifiersState::empty());
        dialog.paste("新しい");
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()),
            Some(ProductivityAction::GroupCreate("produção 新しい".into()))
        );
        dialog.logical_key(&Key::Character("a".into()), ModifiersState::CONTROL);
        dialog.paste(&"界".repeat(64));
        dialog.paste("a");
        let ProductivityDialog::Groups(group) = &dialog else { unreachable!() };
        assert_eq!(group.name.text.chars().count(), 64);
        assert_eq!(group.tab_id, 7);
        dialog.panel_mut().focus = 7;
        dialog.paste("ignored");
        let ProductivityDialog::Groups(group) = &dialog else { unreachable!() };
        assert_eq!(group.name.text.chars().count(), 64);
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
            Some(ProductivityAction::Cancel)
        );
    }

    #[test]
    fn group_panel_keeps_scrolled_selection_and_footer_reachable() {
        for locale in [Locale::En, Locale::PtBr] {
            for (columns, lines) in [(120, 40), (45, 22), (24, 12)] {
                let groups =
                    (0..30).map(|index| (format!("Project {index:02}"), index + 1)).collect();
                let mut dialog = ProductivityDialog::Groups(GroupDialog::new(
                    9,
                    groups,
                    Some("Project 00".into()),
                    locale,
                ));
                let initial = dialog.parts(columns, lines);
                let footer: Vec<_> = initial
                    .iter()
                    .filter(|(_, id)| matches!(id, Some(3..=7)))
                    .map(|(element, id)| (*id, element.rect))
                    .collect();
                assert_eq!(footer.len(), 5);
                dialog.panel_mut().focus = 129;
                let parts = dialog.parts(columns, lines);
                let last = parts.iter().find(|(_, id)| *id == Some(129)).unwrap();
                assert!(last.0.focused);
                for (element, _) in &parts {
                    assert!(element.rect.left + element.rect.width <= columns);
                    assert!(element.rect.top + element.rect.height <= lines);
                }
                assert_eq!(
                    footer,
                    parts
                        .iter()
                        .filter(|(_, id)| matches!(id, Some(3..=7)))
                        .map(|(element, id)| (*id, element.rect))
                        .collect::<Vec<_>>()
                );
                assert_eq!(dialog.click(last.0.rect.left, last.0.rect.top, columns, lines), None);
                assert_eq!(
                    dialog.activate(3),
                    Some(ProductivityAction::GroupMove(Some("Project 29".into())))
                );
            }
        }
    }

    #[test]
    fn split_shortcuts_swap_from_the_visible_button_and_report_save_results() {
        let (right, below) = if cfg!(target_os = "macos") {
            ("Cmd+D", "Shift+Cmd+D")
        } else {
            ("Ctrl+Shift+E", "Ctrl+Shift+J")
        };
        let mut config = UiConfig::default();
        config.general.language = Language::PtBr;
        config.general.shortcuts.insert("SplitVertical".into(), right.into());
        config.general.shortcuts.insert("SplitHorizontal".into(), below.into());
        let prefs = PreferencesDialog::new(Rc::new(config), crossfont::Size::new(11.), vec![]);
        let mut dialog = ProductivityDialog::Preferences(Box::new(prefs));
        dialog.activate(2);
        let actions = PreferencesDialog::shortcuts();
        let vertical = actions.iter().position(|(name, ..)| *name == "SplitVertical").unwrap();
        let horizontal = actions.iter().position(|(name, ..)| *name == "SplitHorizontal").unwrap();
        let ProductivityDialog::Preferences(prefs) = &mut dialog else { unreachable!() };
        let previous = prefs.shortcut_label("SplitVertical");
        let requested = prefs.shortcut_label("SplitHorizontal");
        let binding = shortcut_binding("SplitHorizontal", &requested).unwrap();
        let crate::config::BindingKey::Keycode { key, .. } = binding.trigger else {
            unreachable!()
        };
        dialog.activate(100 + vertical);
        dialog.logical_key(&key, binding.mods);
        let control = |dialog: &ProductivityDialog, target| {
            dialog.parts(98, 32).into_iter().find(|(_, id)| *id == Some(target)).unwrap().0
        };
        let swap = control(&dialog, 27);
        assert_eq!(swap.text, "Trocar atalhos");
        assert!(dialog.elements(98, 32).iter().any(|e| e.kind == UiKind::Error
            && e.text.contains(Locale::PtBr.text(actions[horizontal].1))));
        assert_eq!(dialog.activate(14), None);
        dialog.click(swap.rect.left + 1, swap.rect.top, 98, 32);
        assert!(!dialog.captures_shortcut());
        let Some(ProductivityAction::SaveSettings(draft)) = dialog.activate(14) else { panic!() };
        assert_eq!(draft.shortcut_overrides["SplitVertical"], requested);
        assert_eq!(draft.shortcut_overrides["SplitHorizontal"], previous);
        assert_eq!(control(&dialog, 14).text, "Salvando");
        let ProductivityDialog::Preferences(prefs) = &mut dialog else { unreachable!() };
        prefs.settings_saved(false);
        assert_eq!(control(&dialog, 14).text, "Repetir");
        assert!(matches!(control(&dialog, 14).kind, UiKind::PressedButton(_)));
        dialog.activate(14);
        let ProductivityDialog::Preferences(prefs) = &mut dialog else { unreachable!() };
        let mut config = (*prefs.original).clone();
        draft.apply(&mut config).unwrap();
        prefs.rebase(Rc::new(config), 1.);
        prefs.settings_saved(true);
        assert!(prefs.values.shortcut_overrides.is_empty());
        assert_eq!(control(&dialog, 14).text, "✓ Salvo");
    }

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
    fn rebase_updates_untouched_font_input_and_preserves_in_progress_editing() {
        let original = Rc::new(UiConfig::default());
        let mut dialog = PreferencesDialog::new(original, crossfont::Size::new(11.), Vec::new());
        let current: UiConfig = toml::from_str(
            "[font]\nsize=18\nnormal={family='External'}\n[general]\nlanguage='pt-BR'\n",
        )
        .unwrap();
        dialog.rebase(Rc::new(current.clone()), 2.);
        assert_eq!(dialog.family.text, "External");
        assert_eq!(dialog.family.cursor, 8);
        assert_eq!(dialog.values.font_family, "External");
        assert_eq!(dialog.original_font_size, current.font.size().scale(2.));
        assert_eq!(dialog.locale, Language::PtBr.resolve());

        dialog.family = TextInput::new("Edited family".into());
        dialog.family.cursor = 3;
        dialog.family.selected_all = true;
        dialog.values.font_family = dialog.family.text.clone();
        let next: UiConfig = toml::from_str("[font]\nnormal={family='Next'}\n").unwrap();
        dialog.rebase(Rc::new(next), 1.);
        assert_eq!(dialog.family.text, "Edited family");
        assert_eq!(dialog.family.cursor, 3);
        assert!(dialog.family.selected_all);
        assert_eq!(dialog.values.font_family, "Edited family");
        let Some(ProductivityAction::SaveSettings(values)) = dialog.activate(14) else { panic!() };
        assert_eq!(values.font_family, "Edited family");
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
        let expected = if cfg!(target_os = "macos") { "Ctrl+Option+F9" } else { "Ctrl+Alt+F9" };
        assert_eq!(preferences.shortcut_label("CreateNewTab"), expected);
        assert!(config.general.shortcuts.is_empty());
        let mut preview = (*config).clone();
        preferences.values.preview(&mut preview).unwrap();
        assert!(preview.general.shortcuts.is_empty());
        let Some(ProductivityAction::SaveSettings(values)) = dialog.activate(14) else { panic!() };
        assert_eq!(values.shortcut_overrides["CreateNewTab"], expected);
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
        assert_eq!(
            navigation.len(),
            7 + usize::from(cfg!(any(target_os = "linux", target_os = "macos")))
        );
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
        assert_eq!(navigation[6].0.rect.top, navigation[0].0.rect.top + 2);
        assert_eq!(
            navigation.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
            vec![
                Some(0),
                Some(20),
                Some(21),
                Some(1),
                Some(13),
                Some(2),
                Some(33),
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                Some(36)
            ]
        );
    }

    #[test]
    fn compact_adjustments_and_footer_keep_readable_values_and_precise_hit_targets() {
        for locale in [Locale::En, Locale::PtBr] {
            for (columns, lines) in [(54, 20), (67, 20), (100, 32)] {
                let mut preferences = PreferencesDialog::new(
                    Rc::new(UiConfig::default()),
                    crossfont::Size::new(14.2),
                    Vec::new(),
                );
                preferences.locale = locale;
                preferences.values.font_size = 14.2;
                let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
                let parts = dialog.parts(columns, lines);
                let decrease = parts.iter().find(|(_, id)| *id == Some(4)).unwrap().0.rect;
                let increase = parts.iter().find(|(_, id)| *id == Some(5)).unwrap().0.rect;
                assert_eq!(decrease.top, increase.top);
                assert!(decrease.width <= 5 && increase.width <= 5);
                assert!(parts.iter().any(|(element, _)| {
                    element.rect.top == decrease.top
                        && element.text.trim() == "14.2 pt"
                        && element.rect.left >= decrease.left + decrease.width
                        && element.rect.left + element.rect.width <= increase.left
                }));
                let label = parts
                    .iter()
                    .find(|(element, _)| element.text == locale.text(Message::FontSize))
                    .unwrap();
                assert_eq!(label.0.rect.top, decrease.top);
                assert_eq!(label.0.rect.height, decrease.height);
                assert!(parts.iter().any(|(element, _)| element.text.trim() == "14.2 pt"
                    && element.rect.top == decrease.top
                    && element.rect.height == decrease.height));
                assert!(label.0.rect.left + label.0.rect.width < decrease.left);
                for id in [14, 15] {
                    let action =
                        parts.iter().find(|(_, candidate)| *candidate == Some(id)).unwrap();
                    assert!(action.0.rect.width <= 20);
                }
                let status =
                    parts.iter().find(|(element, _)| element.kind == UiKind::Status).unwrap();
                let layout = dialog.layout(columns, lines);
                assert!(status.0.text.width() <= status.0.rect.width);
                assert!(status.0.rect.top + status.0.rect.height < layout.top + layout.height);
                assert_eq!(dialog.click(decrease.left, decrease.top, columns, lines), None);
                let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
                assert!((preferences.values.font_size - 13.7).abs() < 0.001);
                assert_eq!(dialog.click(increase.left, increase.top, columns, lines), None);
                let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
                assert!((preferences.values.font_size - 14.2).abs() < 0.001);
            }
        }
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
        for _ in 0..14 + usize::from(cfg!(any(target_os = "linux", target_os = "macos"))) {
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
            for page in [
                0,
                20,
                21,
                1,
                13,
                2,
                33,
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                36,
            ] {
                let mut preferences = PreferencesDialog::new(
                    Rc::new(UiConfig::default()),
                    crossfont::Size::new(11.),
                    Vec::new(),
                );
                preferences.locale = locale;
                preferences.activate(page);
                let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
                for (columns, lines) in [(100, 32), (45, 18), (67, 13), (90, 12), (12, 6), (2, 1)] {
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

    #[cfg(termust_updater)]
    #[test]
    fn updates_require_an_explicit_install_and_preserve_terminal_drafts() {
        let config = Rc::new(UiConfig::default());
        let new_dialog = || {
            let mut preferences =
                PreferencesDialog::new(config.clone(), crossfont::Size::new(11.), Vec::new());
            preferences.locale = Locale::PtBr;
            ProductivityDialog::Preferences(Box::new(preferences))
        };
        let click = |dialog: &mut ProductivityDialog, id| {
            dialog.panel_mut().focus = id;
            let parts = dialog.parts(120, 40);
            let (element, _) = parts.iter().find(|(_, control)| *control == Some(id)).unwrap();
            dialog.click(element.rect.left, element.rect.top, 120, 40)
        };
        let mut dialog = new_dialog();
        click(&mut dialog, 20);
        let rows = dialog.rows(120, 40);
        let controls: Vec<_> = Panel::controls(&rows).iter().map(|control| control.id).collect();
        assert!(controls.windows(6).any(|ids| ids == [22, 23, 24, 25, 40, 41]));
        assert!(!controls.contains(&42));
        click(&mut dialog, 24);
        click(&mut dialog, 40);
        assert_eq!(click(&mut dialog, 41), Some(ProductivityAction::CheckUpdates));
        assert_eq!(dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()), None);
        assert_eq!(dialog.activate(42), None);

        let ProductivityDialog::Preferences(preferences) = &mut dialog else { unreachable!() };
        let version = "0.18.0-termust.75332083.b535ae3aaaf0";
        preferences.update = UpdateStatus::Available { version: version.into() };
        assert!(!preferences.values.check_updates);
        assert_ne!(preferences.values.hide_mouse_when_typing, config.mouse.hide_when_typing);
        let compact_width = Panel::content_width(42, 20, true);
        let compact_labels: String = preferences
            .rows(42, 20)
            .into_iter()
            .filter_map(|row| match row {
                Row::Label(label) if label.width() <= compact_width => Some(label),
                _ => None,
            })
            .collect();
        assert!(compact_labels.contains(version));
        assert!(dialog.elements(120, 40).iter().any(|element| element.text.contains(version)));
        assert_eq!(click(&mut dialog, 42), Some(ProductivityAction::InstallUpdate(version.into())));
        assert_eq!(dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()), None);
        assert_eq!(dialog.activate(41), None);
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
            Some(ProductivityAction::Cancel)
        );
        let mut reopened = new_dialog();
        let ProductivityDialog::Preferences(preferences) = &reopened else { unreachable!() };
        assert!(preferences.values.check_updates);
        assert_eq!(preferences.values.hide_mouse_when_typing, config.mouse.hide_when_typing);
        click(&mut reopened, 20);
        click(&mut reopened, 40);
        let Some(ProductivityAction::SaveSettings(values)) = click(&mut reopened, 14) else {
            panic!()
        };
        assert!(!values.check_updates);
    }

    #[cfg(termust_updater)]
    #[test]
    fn updates_report_each_outcome_and_offer_retry_after_failure() {
        let mut preferences = PreferencesDialog::new(
            Rc::new(UiConfig::default()),
            crossfont::Size::new(11.),
            Vec::new(),
        );
        preferences.activate(20);
        for (status, message, can_check) in [
            (UpdateStatus::Checking, Message::UpdateChecking, false),
            (UpdateStatus::Current, Message::UpdateCurrent, true),
            (UpdateStatus::Installing, Message::UpdateInstalling, false),
            (UpdateStatus::Installed, Message::UpdateInstalled, false),
            (UpdateStatus::Unmanaged, Message::UpdateUnmanaged, false),
            (UpdateStatus::Failed, Message::UpdateFailed, true),
        ] {
            preferences.update = status;
            let rows = preferences.rows(120, 40);
            let controls = Panel::controls(&rows);
            assert_eq!(controls.iter().any(|c| c.id == 41 && !c.disabled), can_check);
            assert!(!controls.iter().any(|c| c.id == 42));
            let labels: Vec<_> = rows
                .iter()
                .filter_map(|row| match row {
                    Row::Label(label) => Some(label.as_str()),
                    _ => None,
                })
                .collect();
            assert!(labels.join(" ").contains(preferences.locale.text(message)));
        }
        assert_eq!(preferences.activate(41), Some(ProductivityAction::CheckUpdates));
    }

    #[test]
    fn shortcut_list_keeps_search_and_actions_visible_while_every_binding_is_editable() {
        for locale in [Locale::En, Locale::PtBr] {
            for (columns, lines) in [(45, 18), (67, 20), (107, 29)] {
                let mut preferences = PreferencesDialog::new(
                    Rc::new(UiConfig::default()),
                    crossfont::Size::new(11.),
                    Vec::new(),
                );
                preferences.locale = locale;
                preferences.activate(2);
                let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
                let initial = dialog.parts(columns, lines);
                let search = initial.iter().find(|(_, id)| *id == Some(26)).unwrap().0.rect;
                let footer: Vec<_> = [28, 29, 14, 15]
                    .into_iter()
                    .map(|id| {
                        (
                            id,
                            initial
                                .iter()
                                .find(|(_, control)| *control == Some(id))
                                .unwrap()
                                .0
                                .rect,
                        )
                    })
                    .collect();
                for (index, (action, ..)) in PreferencesDialog::shortcuts().into_iter().enumerate()
                {
                    dialog.panel_mut().focus = 100 + index;
                    dialog.panel().last_focus.set(None);
                    let parts = dialog.parts(columns, lines);
                    let (row, _) = parts.iter().find(|(_, id)| *id == Some(100 + index)).unwrap();
                    assert_eq!(row.rect.height, 1);
                    let ProductivityDialog::Preferences(preferences) = &dialog else {
                        unreachable!()
                    };
                    assert!(
                        row.text.contains(
                            preferences.shortcut_label(action).split(" / ").next().unwrap()
                        )
                    );
                    assert_eq!(
                        parts.iter().find(|(_, id)| *id == Some(26)).unwrap().0.rect,
                        search
                    );
                    for (id, rect) in &footer {
                        assert_eq!(
                            parts.iter().find(|(_, control)| *control == Some(*id)).unwrap().0.rect,
                            *rect
                        );
                    }
                    dialog.click(row.rect.left + row.rect.width - 1, row.rect.top, columns, lines);
                    assert!(dialog.captures_shortcut(), "{action}");
                    dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty());
                }
            }
        }
    }

    #[test]
    fn shortcut_search_matches_localized_names_and_keys_and_defaults_only_reset_shortcuts() {
        for (locale, query) in
            [(Locale::En, "Rename"), (Locale::PtBr, "Renomear"), (Locale::En, "F2")]
        {
            let config: UiConfig =
                toml::from_str("[keyboard]\nbindings=[{key='F2',action='RenameTab',mode='~Alt'}]")
                    .unwrap();
            let mut preferences =
                PreferencesDialog::new(Rc::new(config), crossfont::Size::new(11.), Vec::new());
            preferences.locale = locale;
            preferences.activate(2);
            preferences.values.font_size = 19.;
            preferences.shortcut_filter = TextInput::new(query.into());
            preferences.filter_changed();
            assert_eq!(preferences.filtered_shortcuts().len(), 1);
            assert_eq!(
                PreferencesDialog::shortcuts()[preferences.filtered_shortcuts()[0].0].0,
                "RenameTab"
            );
            preferences.activate(26);
            assert!(!preferences.capturing_shortcut);
            preferences.activate(100 + preferences.filtered_shortcuts()[0].0);
            assert!(preferences.capturing_shortcut);
            preferences.capture_shortcut(&Key::Named(NamedKey::F8), ModifiersState::empty(), None);
            assert_eq!(preferences.values.shortcut_overrides["RenameTab"], "F8");
            preferences.activate(29);
            assert_eq!(preferences.values.font_size, 19.);
            assert_eq!(preferences.values.shortcut_overrides, default_shortcuts());
            preferences.shortcut_filter = TextInput::new("not-an-action".into());
            let rows = preferences.rows(67, 20);
            assert!(
                Panel::controls(&rows).iter().find(|control| control.id == 28).unwrap().disabled
            );
            assert!(rows.iter().any(|row| matches!(row, Row::Label(text) if text == locale.text(Message::ShortcutNoResults))));
        }
    }

    #[test]
    fn shortcut_search_accepts_paste_and_keeps_the_edited_key_visible() {
        let config: UiConfig =
            toml::from_str("[keyboard]\nbindings=[{key='F2',action='RenameTab',mode='~Alt'}]")
                .unwrap();
        let preferences =
            PreferencesDialog::new(Rc::new(config), crossfont::Size::new(11.), Vec::new());
        let mut dialog = ProductivityDialog::Preferences(Box::new(preferences));
        dialog.activate(2);
        let search =
            dialog.parts(67, 20).into_iter().find(|(_, id)| *id == Some(26)).unwrap().0.rect;
        dialog.click(search.left, search.top, 67, 20);
        assert!(!dialog.captures_shortcut());
        dialog.paste("F2");
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert_eq!(preferences.filtered_shortcuts().len(), 1);
        dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty());
        assert!(dialog.captures_shortcut());
        dialog.logical_key(&Key::Named(NamedKey::F8), ModifiersState::empty());
        assert!(!dialog.captures_shortcut());
        let ProductivityDialog::Preferences(preferences) = &dialog else { unreachable!() };
        assert!(preferences.shortcut_filter.text.is_empty());
        assert_eq!(preferences.values.shortcut_overrides["RenameTab"], "F8");
        assert!(
            dialog
                .parts(67, 20)
                .iter()
                .any(|(element, id)| *id == Some(101) && element.text.contains("F8"))
        );
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
            ProductivityDialog::Groups(GroupDialog::new(1, Vec::new(), None, Locale::En)),
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
                ProductivityDialog::Groups(group) => assert_eq!(group.name.text, "Daily panes"),
                ProductivityDialog::Agents(_) => unreachable!(),
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
        input.key(&Key::Character("a".into()), ModifiersState::SUPER, 80);
        input.key(&Key::Character("api".into()), ModifiersState::empty(), 80);
        assert_eq!(input.text, if cfg!(target_os = "macos") { "api" } else { "backendapi" });
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
