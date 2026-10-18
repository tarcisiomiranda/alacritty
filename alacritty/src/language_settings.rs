use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table};
use unicode_width::UnicodeWidthStr;
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::localization::{Language, Locale, Message};
use crate::tab_ui::{UiElement, UiKind, UiRect};

const MAX_CONFIG_SIZE: usize = 4 * 1024 * 1024;
const LANGUAGES: [Language; 3] = [Language::Auto, Language::En, Language::PtBr];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageDialogAction {
    Save(Language),
    Cancel,
    Preferences,
}

pub struct LanguageDialog {
    pub selected: Language,
    pub locale: Locale,
    pub status: Option<Message>,
    focus: usize,
    hovered: Option<usize>,
}

impl LanguageDialog {
    #[cfg(test)]
    pub fn new(selected: Language) -> Self {
        Self {
            selected,
            locale: selected.resolve(),
            status: None,
            focus: LANGUAGES.iter().position(|language| *language == selected).unwrap_or(0),
            hovered: None,
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> UiRect {
        let width = if columns >= 12 { columns.saturating_sub(4) } else { columns }.min(64);
        let height = lines.min(15);
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
        control: Option<usize>,
    ) -> (UiElement, Option<usize>) {
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
                focused: control.is_some_and(|control| control < 5 && self.focus == control),
                hovered: control.is_some() && self.hovered == control,
                disabled: false,
            },
            control,
        )
    }

    fn control(&self, index: usize) -> (UiKind, String) {
        match index {
            0..=2 => {
                let language = LANGUAGES[index];
                let mark = if language == self.selected { "[✓]" } else { "[ ]" };
                (UiKind::Checkbox, format!("{mark} {}", language.label(self.locale)))
            },
            3 => (UiKind::PrimaryButton, self.locale.text(Message::SaveLanguage).into()),
            _ => (UiKind::SecondaryButton, self.locale.text(Message::Cancel).into()),
        }
    }

    fn parts(&self, columns: usize, lines: usize) -> Vec<(UiElement, Option<usize>)> {
        let layout = self.layout(columns, lines);
        if layout.width == 0 || layout.height == 0 {
            return Vec::new();
        }
        let inset = if layout.width >= 12 { 2 } else { 0 };
        let width = layout.width.saturating_sub(inset * 2);
        let mut parts = Vec::new();

        if layout.height < 6 {
            if layout.height >= 2 {
                parts.push(self.part(
                    layout,
                    (inset, 0, width),
                    UiKind::Title,
                    self.locale.text(Message::SettingsTitle).into(),
                    None,
                ));
            }
            let (kind, text) = self.control(self.focus);
            parts.push(self.part(
                layout,
                (inset, layout.height - 1, width),
                kind,
                text,
                Some(self.focus),
            ));
        } else if layout.height < 10 {
            parts.push(self.part(
                layout,
                (inset, 0, width),
                UiKind::Title,
                self.locale.text(Message::InterfaceLanguage).into(),
                None,
            ));
            for index in 0..5 {
                let (kind, text) = self.control(index);
                parts.push(self.part(layout, (inset, index + 1, width), kind, text, Some(index)));
            }
            if layout.height >= 8 {
                parts.push(self.part(
                    layout,
                    (inset, layout.height - 2, width),
                    UiKind::Subtitle,
                    self.locale.text(Message::LanguageHint).into(),
                    None,
                ));
            }
        } else {
            parts.push(self.part(
                layout,
                (inset, 1, width.saturating_sub(3)),
                UiKind::Title,
                self.locale.text(Message::SettingsTitle).into(),
                None,
            ));
            let choice_row = if layout.height >= 15 {
                5
            } else if layout.height >= 13 {
                4
            } else {
                3
            };
            if layout.height >= 13 {
                parts.push(self.part(
                    layout,
                    (inset, 2, width),
                    UiKind::Subtitle,
                    self.locale.text(Message::SettingsSubtitle).into(),
                    None,
                ));
                parts.push(self.part(
                    layout,
                    (inset, if layout.height >= 15 { 4 } else { 3 }, width),
                    UiKind::Label,
                    self.locale.text(Message::InterfaceLanguage).into(),
                    None,
                ));
            }
            for index in 0..3 {
                let (kind, text) = self.control(index);
                parts.push(self.part(
                    layout,
                    (inset, choice_row + index, width),
                    kind,
                    text,
                    Some(index),
                ));
            }
            if layout.height >= 13 {
                parts.push(self.part(
                    layout,
                    (inset, layout.height - 6, width),
                    UiKind::SecondaryButton,
                    self.locale.text(Message::GeneralPreferences).into(),
                    Some(6),
                ));
            }
            let button_row = layout.height - if width >= 36 { 3 } else { 4 };
            if layout.height < 13 && button_row > choice_row + 4 {
                let mut rows: Vec<String> = Vec::new();
                for word in self.locale.text(Message::LanguageHint).split_whitespace() {
                    if rows.last().is_none_or(|row| row.width() + word.width() + 1 > width) {
                        rows.push(word.into());
                    } else {
                        let row = rows.last_mut().unwrap();
                        row.push(' ');
                        row.push_str(word);
                    }
                }
                for (offset, text) in rows.into_iter().take(button_row - choice_row - 5).enumerate()
                {
                    parts.push(self.part(
                        layout,
                        (inset, choice_row + 4 + offset, width),
                        UiKind::Subtitle,
                        text,
                        None,
                    ));
                }
            }
            if button_row > choice_row + 3 {
                parts.push(self.part(
                    layout,
                    (inset, button_row - 1, width),
                    UiKind::Divider,
                    String::new(),
                    None,
                ));
            }
            let save_width = if width >= 36 { (width - 2) / 2 } else { width };
            let (kind, text) = self.control(3);
            parts.push(self.part(layout, (inset, button_row, save_width), kind, text, Some(3)));
            let (left, row, cancel_width) = if width >= 36 {
                (inset + save_width + 2, button_row, width - save_width - 2)
            } else {
                (inset, button_row + 1, width)
            };
            let (kind, text) = self.control(4);
            parts.push(self.part(layout, (left, row, cancel_width), kind, text, Some(4)));
            parts.push(self.part(
                layout,
                (layout.width.saturating_sub(4), 1, 2.min(layout.width)),
                UiKind::SecondaryButton,
                "×".into(),
                Some(5),
            ));
        }
        if layout.height >= 7 {
            parts.push(self.part(
                layout,
                (inset, layout.height - 1, width),
                UiKind::Status,
                self.locale.text(self.status.unwrap_or(Message::KeyboardHelp)).into(),
                None,
            ));
        }
        parts.retain(|(element, _)| element.rect.width > 0 && element.rect.height > 0);
        parts
    }

    fn select(&mut self, index: usize) {
        self.selected = LANGUAGES[index];
        self.locale = self.selected.resolve();
        self.focus = index;
        self.status = None;
    }

    fn activate(&mut self) -> Option<LanguageDialogAction> {
        match self.focus {
            0..=2 => {
                self.select(self.focus);
                None
            },
            3 => Some(LanguageDialogAction::Save(self.selected)),
            _ => Some(LanguageDialogAction::Cancel),
        }
    }

    pub fn key(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Option<LanguageDialogAction> {
        self.logical_key(&event.logical_key, modifiers)
    }

    fn logical_key(
        &mut self,
        key: &Key,
        modifiers: ModifiersState,
    ) -> Option<LanguageDialogAction> {
        match key {
            Key::Named(NamedKey::Escape) => Some(LanguageDialogAction::Cancel),
            Key::Named(NamedKey::F2) => Some(LanguageDialogAction::Preferences),
            Key::Named(NamedKey::Tab) => {
                self.focus = (self.focus + if modifiers.shift_key() { 4 } else { 1 }) % 5;
                None
            },
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if self.focus < 3 => {
                self.select(
                    (self.focus + if *key == Key::Named(NamedKey::ArrowUp) { 2 } else { 1 }) % 3,
                );
                None
            },
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) => {
                self.focus =
                    (self.focus + if *key == Key::Named(NamedKey::ArrowUp) { 4 } else { 1 }) % 5;
                None
            },
            Key::Named(NamedKey::Home) => {
                self.select(0);
                None
            },
            Key::Named(NamedKey::End) => {
                self.select(2);
                None
            },
            Key::Named(NamedKey::Enter | NamedKey::Space) => self.activate(),
            Key::Character(text) if text.as_str() == " " => self.activate(),
            _ => None,
        }
    }

    pub fn click(
        &mut self,
        column: usize,
        line: usize,
        columns: usize,
        lines: usize,
    ) -> Option<LanguageDialogAction> {
        if !self.layout(columns, lines).contains(column, line) {
            return Some(LanguageDialogAction::Cancel);
        }
        let control =
            self.parts(columns, lines).into_iter().rev().find_map(|(element, control)| {
                element.rect.contains(column, line).then_some(control).flatten()
            })?;
        if control == 5 {
            Some(LanguageDialogAction::Cancel)
        } else if control == 6 {
            Some(LanguageDialogAction::Preferences)
        } else {
            self.focus = control;
            self.activate()
        }
    }

    pub fn hover(&mut self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        let hovered =
            self.parts(columns, lines).into_iter().rev().find_map(|(element, control)| {
                element.rect.contains(column, line).then_some(control).flatten()
            });
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        changed
    }

    pub fn pointer_cursor(&self, column: usize, line: usize, columns: usize, lines: usize) -> bool {
        self.parts(columns, lines)
            .into_iter()
            .any(|(element, control)| control.is_some() && element.rect.contains(column, line))
    }
}

pub fn save_language(path: &Path, language: Language) -> io::Result<()> {
    let language = match language {
        Language::Auto => "auto",
        Language::En => "en",
        Language::PtBr => "pt-BR",
    };
    save_values(path, &[(
        vec!["general".into(), "language".into()],
        Some(toml_edit::Value::from(language)),
    )])
}

pub fn save_values(
    path: &Path,
    changes: &[(Vec<String>, Option<toml_edit::Value>)],
) -> io::Result<()> {
    let target = config_target(path)?;
    let original = read_config(&target)?;
    let original_text = original
        .as_ref()
        .map_or(Ok(""), |(bytes, _)| std::str::from_utf8(bytes).map_err(|_| invalid_config()))?;
    let mut document = original_text.parse::<DocumentMut>().map_err(|_| invalid_config())?;
    for (path, replacement) in changes {
        if path.is_empty()
            || path.iter().any(|key| key.is_empty() || key.chars().any(char::is_control))
        {
            return Err(invalid_config());
        }
        set_config_item(document.as_item_mut(), path, replacement.as_ref())?;
    }
    let updated = document.to_string();
    if updated.len() > MAX_CONFIG_SIZE {
        return Err(invalid_config());
    }
    let parent = target.parent().filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let mut directories = fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directories.mode(0o700);
    }
    directories.create(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Some((_, metadata)) = &original {
        if metadata.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "configuration is read-only",
            ));
        }
        temporary.as_file().set_permissions(metadata.permissions())?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
        }
    }
    temporary.write_all(updated.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    if config_target(path)? != target || !same_config(&original, &read_config(&target)?) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "configuration changed while saving",
        ));
    }
    temporary.persist(&target).map_err(|error| error.error)?;
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn set_config_item(
    item: &mut Item,
    path: &[String],
    replacement: Option<&toml_edit::Value>,
) -> io::Result<()> {
    let key = &path[0];
    if let Some(table) = item.as_table_mut() {
        if path.len() == 1 {
            if let Some(replacement) = replacement {
                let mut replacement = replacement.clone();
                if let Some(previous) = table.get_mut(key) {
                    if let Some(value) = previous.as_value() {
                        *replacement.decor_mut() = value.decor().clone();
                    }
                    *previous = Item::Value(replacement);
                } else {
                    table.insert(key, Item::Value(replacement));
                }
            } else {
                table.remove(key);
            }
            return Ok(());
        }
        if table.get(key).is_none() {
            if replacement.is_none() {
                return Ok(());
            }
            table.insert(key, Item::Table(Table::new()));
        }
        let nested = table.get_mut(key).ok_or_else(invalid_config)?;
        set_config_item(nested, &path[1..], replacement)
    } else if let Some(table) = item.as_inline_table_mut() {
        set_inline_config(table, path, replacement)
    } else {
        Err(invalid_config())
    }
}

fn set_inline_config(
    table: &mut toml_edit::InlineTable,
    path: &[String],
    replacement: Option<&toml_edit::Value>,
) -> io::Result<()> {
    let key = &path[0];
    if path.len() == 1 {
        if let Some(replacement) = replacement {
            let mut replacement = replacement.clone();
            if let Some(previous) = table.get_mut(key) {
                *replacement.decor_mut() = previous.decor().clone();
                *previous = replacement;
            } else {
                table.insert(key, replacement);
            }
        } else {
            table.remove(key);
        }
        return Ok(());
    }
    if table.get(key).is_none() {
        if replacement.is_none() {
            return Ok(());
        }
        table.insert(key, toml_edit::Value::InlineTable(toml_edit::InlineTable::new()));
    }
    let nested = table
        .get_mut(key)
        .and_then(toml_edit::Value::as_inline_table_mut)
        .ok_or_else(invalid_config)?;
    set_inline_config(nested, &path[1..], replacement)
}

fn config_target(path: &Path) -> io::Result<PathBuf> {
    let legacy = |path: &Path| {
        path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        })
    };
    if legacy(path) {
        return Err(invalid_config());
    }
    let path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)?,
        Ok(_) => path.to_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_owned(),
        Err(error) => return Err(error),
    };
    if legacy(&path) {
        return Err(invalid_config());
    }
    Ok(path)
}

fn read_config(path: &Path) -> io::Result<Option<(Vec<u8>, Metadata)>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    validate_metadata(&metadata)?;
    let file = File::open(path)?;
    let opened = file.metadata()?;
    validate_metadata(&opened)?;
    if !same_file(&metadata, &opened) {
        return Err(invalid_config());
    }
    let mut bytes = Vec::new();
    file.take((MAX_CONFIG_SIZE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONFIG_SIZE {
        return Err(invalid_config());
    }
    Ok(Some((bytes, opened)))
}

fn validate_metadata(metadata: &Metadata) -> io::Result<()> {
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_SIZE as u64 {
        return Err(invalid_config());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err(invalid_config());
        }
    }
    Ok(())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino() && left.mode() == right.mode()
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
            && left.permissions().readonly() == right.permissions().readonly()
    }
}

fn same_config(left: &Option<(Vec<u8>, Metadata)>, right: &Option<(Vec<u8>, Metadata)>) -> bool {
    match (left, right) {
        (Some((left_bytes, left_metadata)), Some((right_bytes, right_metadata))) => {
            left_bytes == right_bytes && same_file(left_metadata, right_metadata)
        },
        (None, None) => true,
        _ => false,
    }
}

fn invalid_config() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid language configuration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_language_preserves_comments_imports_and_unrelated_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        let original = "# Custom theme\n[general]\nimport = [\"theme.toml\"] # Keep \
                        this\nlanguage = 'en' # Language preference\n\n[env]\nSYNTHETIC_TOKEN = \
                        'fixture-$VALUE'\n\n[window]\ntitle = 'A # quoted title'\n";
        fs::write(&path, original).unwrap();
        save_language(&path, Language::PtBr).unwrap();
        let changed = fs::read_to_string(&path).unwrap();
        assert!(changed.contains("# Custom theme"));
        assert!(changed.contains("# Language preference"));
        assert!(changed.contains("import = [\"theme.toml\"] # Keep this"));
        assert!(changed.contains("SYNTHETIC_TOKEN = 'fixture-$VALUE'"));
        assert!(changed.contains("title = 'A # quoted title'"));
        let parsed = changed.parse::<DocumentMut>().unwrap();
        assert_eq!(parsed["general"]["language"].as_str(), Some("pt-BR"));
        assert_eq!(parsed["general"]["import"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn inline_tables_and_dotted_general_settings_are_supported() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        for original in [
            "general = { import = ['theme.toml'], language = 'en' } # Keep inline\n",
            "general.import = ['theme.toml']\ngeneral.language = 'en'\n",
        ] {
            fs::write(&path, original).unwrap();
            save_language(&path, Language::Auto).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            let parsed = text.parse::<DocumentMut>().unwrap();
            assert_eq!(parsed["general"]["language"].as_str(), Some("auto"));
            assert_eq!(parsed["general"]["import"][0].as_str(), Some("theme.toml"));
            if original.contains("# Keep inline") {
                assert!(text.contains("# Keep inline"));
            }
        }
    }

    #[test]
    fn multiple_nested_changes_preserve_regular_inline_and_dotted_tables() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        for original in [
            "# Keep header\n[font.normal]\nfamily = 'Before' # Keep family\nstyle = \
             'Regular'\n[font.bold]\nfamily = \
             'Bold'\n[general]\nstartup_session='Remove'\n[window]\nopacity=0.8\n",
            "# Keep header\nfont = { normal = { family = 'Before', style = 'Regular' }, bold = { \
             family = 'Bold' } } # Keep font\ngeneral = { startup_session = 'Remove' }\nwindow = \
             { opacity = 0.8 }\n",
            "# Keep header\nfont.normal.family = 'Before' # Keep family\nfont.normal.style = \
             'Regular'\nfont.bold.family = 'Bold'\ngeneral.startup_session = \
             'Remove'\nwindow.opacity = 0.8\n",
        ] {
            fs::write(&path, original).unwrap();
            let changes = [
                (
                    vec!["font".into(), "normal".into(), "family".into()],
                    Some(toml_edit::Value::from("After")),
                ),
                (vec!["font".into(), "size".into()], Some(toml_edit::Value::from(17.0))),
                (vec!["window".into(), "opacity".into()], Some(toml_edit::Value::from(0.9))),
                (vec!["general".into(), "startup_session".into()], None),
            ];
            save_values(&path, &changes).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            let document: toml::Value = toml::from_str(&text).unwrap();
            assert_eq!(document["font"]["normal"]["family"].as_str(), Some("After"));
            assert_eq!(document["font"]["normal"]["style"].as_str(), Some("Regular"));
            assert_eq!(document["font"]["bold"]["family"].as_str(), Some("Bold"));
            assert_eq!(document["font"]["size"].as_float(), Some(17.0));
            assert_eq!(document["window"]["opacity"].as_float(), Some(0.9));
            assert!(
                document
                    .get("general")
                    .and_then(|general| general.get("startup_session"))
                    .is_none()
            );
            assert!(text.contains("# Keep header"));
            if original.contains("# Keep family") {
                assert!(text.contains("# Keep family"));
            }
            if original.contains("# Keep font") {
                assert!(text.contains("# Keep font"));
            }
        }
    }

    #[test]
    fn creating_nested_inline_tables_and_removing_missing_values_are_supported() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        fs::write(&path, "font = { size = 12.0 } # Keep inline\n").unwrap();
        save_values(&path, &[
            (
                vec!["font".into(), "normal".into(), "family".into()],
                Some(toml_edit::Value::from("New family")),
            ),
            (vec!["general".into(), "startup_session".into()], None),
        ])
        .unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("# Keep inline"));
        let document: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(document["font"]["normal"]["family"].as_str(), Some("New family"));
        assert!(document.get("general").is_none());
    }

    #[test]
    fn malformed_paths_and_conflicting_nested_values_leave_config_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        let original = "font = { size = 12.0, normal = 'private-fixture' }\n";
        fs::write(&path, original).unwrap();
        for invalid_path in [vec![], vec![String::new()], vec!["font".into(), "\n".into()], vec![
            "font".into(),
            "normal".into(),
            "family".into(),
        ]] {
            let changes = [
                (vec!["font".into(), "size".into()], Some(toml_edit::Value::from(18.0))),
                (invalid_path, Some(toml_edit::Value::from("After"))),
            ];
            let error = save_values(&path, &changes).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!error.to_string().contains("private-fixture"));
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn absent_config_creates_general_language_without_other_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new/config/alacritty.toml");
        save_language(&path, Language::En).unwrap();
        let parsed = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        assert_eq!(parsed["general"]["language"].as_str(), Some("en"));
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn invalid_configs_are_never_overwritten_and_errors_do_not_contain_values() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        for original in [
            "general = 'private-fixture'\n",
            "general = []\n",
            "[general]\nlanguage = 'en'\n[general]\nlanguage = 'pt-BR'\n",
            "private-fixture = [\n",
        ] {
            fs::write(&path, original).unwrap();
            let error = save_language(&path, Language::Auto).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!error.to_string().contains("private-fixture"));
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
        let yaml = root.path().join("alacritty.yml");
        fs::write(&yaml, "general:\n  language: en\n").unwrap();
        assert_eq!(
            save_language(&yaml, Language::En).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_to_string(yaml).unwrap(), "general:\n  language: en\n");
    }

    #[test]
    fn oversized_and_non_regular_configs_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        fs::write(&path, vec![b' '; MAX_CONFIG_SIZE + 1]).unwrap();
        assert!(save_language(&path, Language::En).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), (MAX_CONFIG_SIZE + 1) as u64);
        let directory = root.path().join("directory.toml");
        fs::create_dir(&directory).unwrap();
        assert!(save_language(&directory, Language::En).is_err());
        let blocked = path.join("nested.toml");
        assert!(save_language(&blocked, Language::En).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), (MAX_CONFIG_SIZE + 1) as u64);
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_preserved_new_files_are_private_and_readonly_files_stay_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new/config/alacritty.toml");
        save_language(&path, Language::En).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        save_language(&path, Language::PtBr).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        let before = fs::read(&path).unwrap();
        let error = save_language(&path, Language::Auto).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_keep_the_link_and_hardlinks_are_rejected_without_overwrite() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("target.toml");
        let link = root.path().join("alacritty.toml");
        fs::write(&path, "[general]\nlanguage = 'en'\n").unwrap();
        symlink(&path, &link).unwrap();
        save_language(&link, Language::PtBr).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap()["general"]
                ["language"]
                .as_str(),
            Some("pt-BR")
        );
        let hardlink = root.path().join("hardlink.toml");
        fs::hard_link(&path, &hardlink).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(save_language(&hardlink, Language::En).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&hardlink).unwrap(), before);
    }

    #[test]
    fn concurrency_guard_compares_bytes_and_detects_new_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        assert!(same_config(&None, &read_config(&path).unwrap()));
        fs::write(&path, "[general]\nlanguage = 'en'\n").unwrap();
        let before = read_config(&path).unwrap();
        assert!(!same_config(&None, &before));
        assert!(same_config(&before, &read_config(&path).unwrap()));
        fs::write(&path, "[general]\nlanguage = 'pt-BR'\n").unwrap();
        assert!(!same_config(&before, &read_config(&path).unwrap()));
    }

    #[test]
    fn general_preferences_link_and_shortcut_are_reachable_without_overlapping_controls() {
        let mut dialog = LanguageDialog::new(Language::En);
        for columns in [40, 90] {
            for lines in 13..=15 {
                let parts = dialog.parts(columns, lines);
                let link = &parts.iter().find(|(_, control)| *control == Some(6)).unwrap().0;
                assert_eq!(link.rect.top, dialog.layout(columns, lines).top + lines - 6);
                assert!(dialog.pointer_cursor(link.rect.left, link.rect.top, columns, lines));
                assert!(
                    parts
                        .iter()
                        .filter(|(_, control)| matches!(control, Some(0..=4)))
                        .all(|(element, _)| element.rect.top != link.rect.top)
                );
                assert_eq!(
                    dialog.click(link.rect.left, link.rect.top, columns, lines),
                    Some(LanguageDialogAction::Preferences)
                );
                assert_eq!(dialog.selected, Language::En);
            }
        }
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::F2), ModifiersState::empty()),
            Some(LanguageDialogAction::Preferences)
        );
        assert!(dialog.parts(20, 7).iter().all(|(_, control)| *control != Some(6)));
        assert_eq!(
            dialog.logical_key(&Key::Named(NamedKey::F2), ModifiersState::empty()),
            Some(LanguageDialogAction::Preferences)
        );
    }

    #[test]
    fn mouse_choices_preview_and_save_the_selected_language() {
        let mut dialog = LanguageDialog::new(Language::En);
        let portuguese =
            dialog.parts(90, 26).into_iter().find(|(_, control)| *control == Some(2)).unwrap().0;
        assert!(dialog.pointer_cursor(portuguese.rect.left, portuguese.rect.top, 90, 26));
        assert!(dialog.hover(portuguese.rect.left, portuguese.rect.top, 90, 26));
        assert!(!dialog.hover(portuguese.rect.left, portuguese.rect.top, 90, 26));
        assert_eq!(dialog.click(portuguese.rect.left, portuguese.rect.top, 90, 26), None);
        assert_eq!(dialog.selected, Language::PtBr);
        assert_eq!(dialog.locale, Language::PtBr.resolve());
        let save =
            dialog.parts(90, 26).into_iter().find(|(_, control)| *control == Some(3)).unwrap().0;
        assert_eq!(
            dialog.click(save.rect.left, save.rect.top, 90, 26),
            Some(LanguageDialogAction::Save(Language::PtBr))
        );
        assert_eq!(dialog.click(0, 0, 90, 26), Some(LanguageDialogAction::Cancel));
    }

    #[test]
    fn compact_dialog_keeps_keyboard_actions_reachable_and_elements_inside_viewport() {
        for (columns, lines) in [(90, 26), (45, 18), (20, 10), (8, 6), (5, 3), (1, 1)] {
            let mut dialog = LanguageDialog::new(Language::En);
            let layout = dialog.layout(columns, lines);
            for _ in 0..5 {
                let elements = dialog.parts(columns, lines);
                assert!(elements.iter().any(|(_, control)| *control == Some(dialog.focus)));
                for (element, _) in &elements {
                    assert!(element.rect.left + element.rect.width <= columns);
                    assert!(element.rect.top + element.rect.height <= lines);
                    assert!(layout.contains(element.rect.left, element.rect.top));
                }
                dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
            }
            dialog.logical_key(&Key::Named(NamedKey::End), ModifiersState::empty());
            assert_eq!(dialog.selected, Language::PtBr);
            dialog.logical_key(&Key::Named(NamedKey::Tab), ModifiersState::empty());
            assert_eq!(
                dialog.logical_key(&Key::Named(NamedKey::Enter), ModifiersState::empty()),
                Some(LanguageDialogAction::Save(Language::PtBr))
            );
            assert_eq!(
                dialog.logical_key(&Key::Named(NamedKey::Escape), ModifiersState::empty()),
                Some(LanguageDialogAction::Cancel)
            );
        }
    }
}
