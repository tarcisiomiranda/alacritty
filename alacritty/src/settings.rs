use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use alacritty_config::SerdeReplace;
use toml_edit::Value;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};

use crate::config::scrolling::MAX_SCROLLBACK_LINES;
use crate::config::{Action, BindingKey, BindingMode, KeyBinding, UiConfig};
use crate::language_settings::save_values;
use crate::localization::Language;

pub type SettingsChange = (Vec<String>, Option<Value>);

pub const SHORTCUT_ACTIONS: [&str; 16] = [
    "CreateNewTab",
    "DuplicateTab",
    "ReopenClosedTab",
    "SplitVertical",
    "SplitHorizontal",
    "FocusPaneLeft",
    "FocusPaneRight",
    "FocusPaneUp",
    "FocusPaneDown",
    "TogglePaneZoom",
    "ClosePane",
    "OpenWorkspaces",
    "MoveTabLeft",
    "MoveTabRight",
    "OpenSettings",
    "SaveSession",
];

pub fn shortcut_binding(action: &str, shortcut: &str) -> Option<KeyBinding> {
    if !SHORTCUT_ACTIONS.contains(&action) || shortcut.len() > 128 {
        return None;
    }
    let parts: Vec<_> = shortcut.split('+').collect();
    let (key, modifiers) = parts.split_last()?;
    if key.is_empty() {
        return None;
    }
    let mut mods = ModifiersState::empty();
    for modifier in modifiers {
        mods |= match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ModifiersState::CONTROL,
            "alt" | "option" => ModifiersState::ALT,
            "shift" => ModifiersState::SHIFT,
            "super" | "command" | "cmd" => ModifiersState::SUPER,
            _ => return None,
        };
    }
    let trigger: BindingKey = toml::Value::String((*key).into()).try_into().ok()?;
    let binding = KeyBinding {
        action: toml::Value::String(action.into()).try_into().ok()?,
        trigger,
        mods,
        mode: BindingMode::empty(),
        notmode: BindingMode::empty(),
    };
    if matches!(&binding.trigger, BindingKey::Keycode {
        key: Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
        ),
        ..
    }) {
        return None;
    }
    let function_key = match &binding.trigger {
        BindingKey::Keycode { key: Key::Named(key), .. } => format!("{key:?}")
            .strip_prefix('F')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=35).contains(&number)),
        _ => false,
    };
    if !function_key
        && !mods.intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER)
    {
        return None;
    }
    Some(binding)
}

pub fn shortcut_text(key: &Key, mods: ModifiersState) -> Option<String> {
    let key =
        match key {
            Key::Character(text)
                if text.chars().count() == 1 && !text.chars().any(char::is_control) =>
            {
                if text == "+" { "Plus".into() } else { text.to_ascii_uppercase() }
            },
            Key::Named(
                NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper,
            ) => return None,
            Key::Named(key) => format!("{key:?}"),
            _ => return None,
        };
    let mut parts = Vec::new();
    for (modifier, label) in [
        (ModifiersState::CONTROL, "Ctrl"),
        (ModifiersState::ALT, "Alt"),
        (ModifiersState::SHIFT, "Shift"),
        (ModifiersState::SUPER, "Super"),
    ] {
        if mods.contains(modifier) {
            parts.push(label.to_owned());
        }
    }
    parts.push(key);
    Some(parts.join("+"))
}

pub fn resolved_shortcuts(
    config: &UiConfig,
    overrides: &BTreeMap<String, String>,
) -> Vec<KeyBinding> {
    let mut bindings = config.key_bindings().to_vec();
    for action in SHORTCUT_ACTIONS {
        let Some(value) = overrides.get(action) else { continue };
        let replacement = if value.is_empty() { None } else { shortcut_binding(action, value) };
        if !value.is_empty() && replacement.is_none() {
            continue;
        }
        let Ok(action) = toml::Value::String(action.into()).try_into::<Action>() else { continue };
        bindings.retain(|binding| binding.action != action);
        bindings.extend(replacement);
    }
    bindings
}

pub fn shortcut_conflict(
    config: &UiConfig,
    overrides: &BTreeMap<String, String>,
    action: &str,
    value: &str,
    physical: Option<PhysicalKey>,
) -> bool {
    let Some(candidate) = shortcut_binding(action, value) else { return false };
    let mut bindings = resolved_shortcuts(config, overrides);
    for hint in &config.hints.enabled {
        if let Some(binding) = &hint.binding {
            bindings.push(binding.key_binding(hint).clone());
        }
    }
    let physical_candidate = physical.map(|physical| KeyBinding {
        trigger: BindingKey::Scancode(physical),
        ..candidate.clone()
    });
    bindings.iter().any(|binding| {
        binding.action != candidate.action
            && (candidate.triggers_match(binding)
                || physical_candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.triggers_match(binding)))
    })
}

pub fn default_shortcuts() -> BTreeMap<String, String> {
    let config = UiConfig::default();
    SHORTCUT_ACTIONS
        .into_iter()
        .map(|name| {
            let action = toml::Value::String(name.into()).try_into::<Action>().unwrap();
            let value = config
                .key_bindings()
                .iter()
                .find_map(|binding| {
                    if binding.action != action {
                        return None;
                    }
                    let BindingKey::Keycode { key, .. } = &binding.trigger else { return None };
                    shortcut_text(key, binding.mods)
                })
                .unwrap_or_default();
            (name.into(), value)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    Keep,
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsValues {
    pub language: Language,
    pub font_family: String,
    pub font_size: f32,
    pub opacity: f32,
    pub theme: ThemeChoice,
    pub restore_session: bool,
    pub session_autosave: bool,
    pub confirm_tab_close: bool,
    pub scrollback_lines: u32,
    pub hide_mouse_when_typing: bool,
    pub copy_on_select: bool,
    pub startup_session: Option<String>,
    pub shortcut_overrides: BTreeMap<String, String>,
    pub shortcut_physical: BTreeMap<String, PhysicalKey>,
}

impl SettingsValues {
    pub fn from_config(config: &UiConfig) -> Self {
        Self {
            shortcut_overrides: BTreeMap::new(),
            shortcut_physical: BTreeMap::new(),
            language: config.general.language,
            font_family: config.font.normal().family.clone(),
            font_size: config.font.size().as_pt(),
            opacity: config.window.opacity.as_f32(),
            theme: ThemeChoice::Keep,
            restore_session: config.general.restore_session,
            session_autosave: config.general.session_autosave,
            confirm_tab_close: config.general.confirm_tab_close,
            scrollback_lines: config.scrolling.history(),
            hide_mouse_when_typing: config.mouse.hide_when_typing,
            copy_on_select: config.selection.save_to_clipboard,
            startup_session: config
                .general
                .startup_session
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        if !self.font_size.is_finite() || !(1.0..=128.0).contains(&self.font_size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "font size must be between 1 and 128",
            ));
        }
        if self.scrollback_lines > MAX_SCROLLBACK_LINES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scrollback exceeds 100000 lines",
            ));
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opacity must be between 0% and 100%",
            ));
        }
        if self.font_family.chars().any(char::is_control)
            || self.font_family.trim().is_empty()
            || self.font_family.trim().chars().count() > 128
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid font family"));
        }
        if self.startup_session.as_ref().is_some_and(|name| {
            name.chars().any(char::is_control)
                || name.trim().is_empty()
                || name.trim().chars().count() > 80
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid startup session name",
            ));
        }
        if self.shortcut_overrides.iter().any(|(action, value)| {
            !SHORTCUT_ACTIONS.contains(&action.as_str())
                || (!value.is_empty() && shortcut_binding(action, value).is_none())
        }) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid keyboard shortcut"));
        }
        Ok(())
    }

    pub fn shortcut_changes_valid(&self, config: &UiConfig) -> bool {
        let mut overrides = config.general.shortcuts.clone();
        overrides.extend(self.shortcut_overrides.clone());
        self.shortcut_overrides.iter().all(|(action, value)| {
            value.is_empty()
                || !shortcut_conflict(
                    config,
                    &overrides,
                    action,
                    value,
                    self.shortcut_physical.get(action).copied(),
                )
        })
    }

    pub fn changes(&self) -> Vec<SettingsChange> {
        let language = match self.language {
            Language::Auto => "auto",
            Language::En => "en",
            Language::PtBr => "pt-BR",
        };
        let mut changes = vec![
            change(&["general", "language"], Value::from(language)),
            change(&["font", "normal", "family"], Value::from(self.font_family.trim())),
            change(&["font", "size"], Value::from(f64::from(self.font_size))),
            change(&["window", "opacity"], Value::from(f64::from(self.opacity))),
            change(&["general", "restore_session"], Value::from(self.restore_session)),
            change(&["general", "session_autosave"], Value::from(self.session_autosave)),
            change(&["general", "confirm_tab_close"], Value::from(self.confirm_tab_close)),
            change(&["scrolling", "history"], Value::from(i64::from(self.scrollback_lines))),
            change(&["mouse", "hide_when_typing"], Value::from(self.hide_mouse_when_typing)),
            change(&["selection", "save_to_clipboard"], Value::from(self.copy_on_select)),
            (
                vec!["general".into(), "startup_session".into()],
                Some(Value::from(self.startup_session.as_deref().map(str::trim).unwrap_or(""))),
            ),
        ];
        for (action, shortcut) in &self.shortcut_overrides {
            changes.push(change(&["general", "shortcuts", action], Value::from(shortcut.as_str())));
        }
        let (primary, normal, bright) = match self.theme {
            ThemeChoice::Keep => return changes,
            ThemeChoice::Dark => (
                ["#d8e2ef", "#10151f", "#f3f6fb", "#8994a6"],
                [
                    "#263041", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2",
                    "#c8d3e1",
                ],
                [
                    "#667085", "#ff7a85", "#b4dc8a", "#f2d08a", "#82c3ff", "#dd9af3", "#7bd3df",
                    "#f3f6fb",
                ],
            ),
            ThemeChoice::Light => (
                ["#1e293b", "#f4f7fb", "#0f172a", "#64748b"],
                [
                    "#1e293b", "#b42332", "#2c7041", "#825b00", "#225fb5", "#7a3fa0", "#006d77",
                    "#cbd5e1",
                ],
                [
                    "#475569", "#cf3041", "#38834f", "#997000", "#3476cf", "#9655b9", "#16868f",
                    "#ffffff",
                ],
            ),
        };
        for (key, color) in ["foreground", "background", "bright_foreground", "dim_foreground"]
            .into_iter()
            .zip(primary)
        {
            changes.push(change(&["colors", "primary", key], Value::from(color)));
        }
        let color_keys = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
        for (group, palette) in [("normal", normal), ("bright", bright)] {
            for (key, color) in color_keys.into_iter().zip(palette) {
                changes.push(change(&["colors", group, key], Value::from(color)));
            }
        }
        changes
    }

    pub fn apply(&self, config: &mut UiConfig) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())?;
        if !self.shortcut_changes_valid(config) {
            return Err("keyboard shortcut conflict".into());
        }
        let mut next = config.clone();
        for (path, value) in self.changes() {
            let Some(value) = value else { continue };
            let mut patch = match value {
                Value::String(value) => toml::Value::String(value.value().clone()),
                Value::Float(value) => toml::Value::Float(*value.value()),
                Value::Boolean(value) => toml::Value::Boolean(*value.value()),
                Value::Integer(value) => toml::Value::Integer(*value.value()),
                _ => return Err("invalid settings value".into()),
            };
            for key in path.into_iter().rev() {
                patch = toml::Value::Table(toml::map::Map::from_iter([(key, patch)]));
            }
            next.replace(patch).map_err(|_| String::from("could not apply terminal settings"))?;
        }
        next.general.startup_session =
            self.startup_session.as_ref().map(|name| name.trim().to_owned());
        *config = next;
        Ok(())
    }

    pub fn preview(&self, config: &mut UiConfig) -> Result<(), String> {
        let mut values = self.clone();
        values.scrollback_lines = config.scrolling.history();
        values.shortcut_overrides.clear();
        values.shortcut_physical.clear();
        values.apply(config)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        save_values(path, &self.changes())
    }
}

fn change(path: &[&str], value: Value) -> SettingsChange {
    (path.iter().map(|key| (*key).to_owned()).collect(), Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_overrides_replace_only_supported_actions_and_fail_soft() {
        let config: UiConfig = toml::from_str(
            "[keyboard]\nbindings=[{key='x',mods='Control',chars='custom'},{key='F8',mode='Vi',\
             action='CreateNewTab'}]\n",
        )
        .unwrap();
        let overrides = BTreeMap::from([
            ("CreateNewTab".into(), "Ctrl+Alt+F9".into()),
            ("DuplicateTab".into(), String::new()),
            ("ClosePane".into(), "not a key".into()),
            ("FutureAction".into(), "F2".into()),
        ]);
        let resolved = resolved_shortcuts(&config, &overrides);
        let new_tabs: Vec<_> =
            resolved.iter().filter(|binding| binding.action == Action::CreateNewTab).collect();
        assert_eq!(new_tabs.len(), 1);
        assert_eq!(*new_tabs[0], shortcut_binding("CreateNewTab", "Ctrl+Alt+F9").unwrap());
        assert!(!resolved.iter().any(|binding| binding.action == Action::DuplicateTab));
        for original in config.key_bindings().iter().filter(|binding| {
            binding.action != Action::CreateNewTab && binding.action != Action::DuplicateTab
        }) {
            assert!(resolved.contains(original));
        }
        assert!(shortcut_binding("CreateNewTab", "t").is_none());
        assert!(shortcut_binding("CreateNewTab", "Shift+T").is_none());
        assert!(shortcut_binding("CreateNewTab", "F7").is_some());
        assert!(shortcut_binding("CreateNewTab", "Ctrl+Plus").is_some());
        assert!(shortcut_binding("CreateNewTab", "Ctrl+Shift").is_none());
    }

    #[test]
    fn shortcut_conflicts_include_modes_hints_and_physical_keys() {
        use winit::platform::scancode::PhysicalKeyExtScancode;
        let config: UiConfig = toml::from_str(
            "[keyboard]\nbindings=[{key='F8',mods='Control',mode='Vi',chars='custom'},{key=9,\
             mods='Control|Alt',chars='physical'}]\n",
        )
        .unwrap();
        let overrides = BTreeMap::new();
        assert!(shortcut_conflict(&config, &overrides, "CreateNewTab", "Ctrl+F8", None));
        assert!(shortcut_conflict(&config, &overrides, "CreateNewTab", "Ctrl+Shift+O", None));
        assert!(shortcut_conflict(
            &config,
            &overrides,
            "CreateNewTab",
            "Ctrl+Alt+F7",
            Some(PhysicalKey::from_scancode(9))
        ));
        assert!(!shortcut_conflict(&config, &overrides, "CreateNewTab", "Ctrl+Alt+F9", None));
        let overrides = BTreeMap::from([("DuplicateTab".into(), "Ctrl+Alt+F9".into())]);
        assert!(shortcut_conflict(&config, &overrides, "CreateNewTab", "Ctrl+Alt+F9", None));
    }

    #[test]
    fn changed_shortcut_paths_preserve_imports_custom_bindings_and_unknown_overrides() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        let text = r"# preserve
[general]
import=['shortcuts.toml']
[general.shortcuts]
FutureAction='F2' # preserve future
[ keyboard ]
bindings=[{key='x',mods='Control',chars='custom'}] # preserve custom
";
        std::fs::write(&path, text).unwrap();
        let mut config: UiConfig = toml::from_str(text).unwrap();
        config.general.shortcuts.insert("OpenSettings".into(), "Ctrl+Alt+F10".into());
        let baseline = config.clone();
        let mut values = SettingsValues::from_config(&config);
        values.shortcut_overrides.insert("CreateNewTab".into(), "Ctrl+Alt+F9".into());
        values.shortcut_overrides.insert("DuplicateTab".into(), String::new());
        values.preview(&mut config).unwrap();
        assert_eq!(config.general.shortcuts, baseline.general.shortcuts);
        values.apply(&mut config).unwrap();
        assert_eq!(config.general.shortcuts["CreateNewTab"], "Ctrl+Alt+F9");
        assert_eq!(config.general.shortcuts["OpenSettings"], "Ctrl+Alt+F10");
        assert_eq!(config.general.shortcuts["FutureAction"], "F2");
        values.save(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# preserve future"));
        assert!(saved.contains("# preserve custom"));
        let loaded: UiConfig = toml::from_str(&saved).unwrap();
        assert_eq!(loaded.key_bindings(), baseline.key_bindings());
        assert_eq!(loaded.general.import, baseline.general.import);
        assert_eq!(loaded.general.shortcuts["FutureAction"], "F2");
        assert_eq!(loaded.general.shortcuts["DuplicateTab"], "");
        assert!(!loaded.general.shortcuts.contains_key("OpenSettings"));
    }

    #[test]
    fn terminal_preferences_roundtrip_with_bounds_and_preserve_other_settings() {
        let mut config: UiConfig = toml::from_str(
            r"[scrolling]
history=4321
multiplier=7
[mouse]
hide_when_typing=true
[selection]
save_to_clipboard=false
semantic_escape_chars='abc'
[cursor]
style={shape='Beam',blinking='Always'}
",
        )
        .unwrap();
        let before = config.clone();
        let mut values = SettingsValues::from_config(&config);
        assert_eq!(values.scrollback_lines, 4321);
        assert!(values.hide_mouse_when_typing);
        assert!(!values.copy_on_select);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        for lines in [0, MAX_SCROLLBACK_LINES] {
            values.scrollback_lines = lines;
            values.hide_mouse_when_typing = false;
            values.copy_on_select = true;
            values.apply(&mut config).unwrap();
            assert_eq!(config.scrolling.history(), lines);
            assert_eq!(config.scrolling.multiplier, before.scrolling.multiplier);
            assert_eq!(
                config.selection.semantic_escape_chars,
                before.selection.semantic_escape_chars
            );
            assert_eq!(config.cursor, before.cursor);
            values.save(&path).unwrap();
            let loaded: UiConfig =
                toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(SettingsValues::from_config(&loaded), values);
        }
        values.scrollback_lines = MAX_SCROLLBACK_LINES + 1;
        let original = config.clone();
        let original_file = std::fs::read(&path).unwrap();
        assert!(values.apply(&mut config).is_err());
        assert_eq!(config, original);
        assert!(values.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original_file);
    }

    #[test]
    fn preview_keeps_scrollback_until_save_without_discarding_other_drafts() {
        let mut config = UiConfig::default();
        let original_history = config.scrolling.history();
        let mut values = SettingsValues::from_config(&config);
        values.scrollback_lines = 0;
        values.language = Language::PtBr;
        values.opacity = 0.8;
        values.hide_mouse_when_typing = true;
        values.copy_on_select = true;
        values.preview(&mut config).unwrap();
        assert_eq!(config.scrolling.history(), original_history);
        assert_eq!(config.general.language, Language::PtBr);
        assert_eq!(config.window.opacity.as_f32(), 0.8);
        assert!(config.mouse.hide_when_typing);
        assert!(config.selection.save_to_clipboard);
        values.apply(&mut config).unwrap();
        assert_eq!(config.scrolling.history(), 0);
    }

    #[test]
    fn values_match_loaded_config_and_apply_preserves_unrelated_settings() {
        let mut config = UiConfig::default();
        config
            .replace(
                toml::from_str(
                    r"[font]
size=15.5
offset={x=2,y=3}
normal={family='Original',style='Regular'}
bold={family='Bold original',style='Heavy'}
[window]
opacity=0.7
blur=true
[general]
startup_session='Old workspace'
[colors.primary]
background='#223344'
",
                )
                .unwrap(),
            )
            .unwrap();
        let before = config.clone();
        let mut values = SettingsValues::from_config(&config);
        assert_eq!(values.font_family, "Original");
        assert_eq!(values.font_size, 15.5);
        assert_eq!(values.opacity, 0.7);
        assert_eq!(values.startup_session.as_deref(), Some("Old workspace"));
        assert_eq!(values.theme, ThemeChoice::Keep);
        assert!(values.confirm_tab_close);
        values.font_family = "  New family  ".into();
        values.font_size = 19.0;
        values.opacity = 0.9;
        values.language = Language::PtBr;
        values.restore_session = false;
        values.session_autosave = true;
        values.confirm_tab_close = false;
        values.startup_session = None;
        values.apply(&mut config).unwrap();
        assert_eq!(config.font.normal().family, "New family");
        assert_eq!(config.font.normal().style, before.font.normal().style);
        assert_eq!(config.font.bold(), before.font.bold());
        assert_eq!(config.font.offset, before.font.offset);
        assert_eq!(config.font.size().as_pt(), 19.0);
        assert_eq!(config.window.opacity.as_f32(), 0.9);
        assert_eq!(config.window.blur, before.window.blur);
        assert_eq!(config.colors, before.colors);
        assert_eq!(config.general.language, Language::PtBr);
        assert!(!config.general.restore_session);
        assert!(config.general.session_autosave);
        assert!(!config.general.confirm_tab_close);
        assert_eq!(config.general.startup_session, None);
    }

    #[test]
    fn invalid_values_are_rejected_before_apply_or_save() {
        let base = SettingsValues::from_config(&UiConfig::default());
        for font_size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.9, 128.1] {
            let mut values = base.clone();
            values.font_size = font_size;
            assert_eq!(values.validate().unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        for opacity in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let mut values = base.clone();
            values.opacity = opacity;
            assert!(values.validate().is_err());
        }
        for family in [String::new(), "   ".into(), "name\n".into(), "x".repeat(129)] {
            let mut values = base.clone();
            values.font_family = family;
            assert!(values.validate().is_err());
        }
        for name in [String::new(), "   ".into(), "name\n".into(), "🦀".repeat(81)] {
            let mut values = base.clone();
            values.startup_session = Some(name);
            assert!(values.validate().is_err());
        }
        let mut values = base;
        values.font_size = f32::NAN;
        let mut config = UiConfig::default();
        let before = config.clone();
        assert!(values.apply(&mut config).is_err());
        assert_eq!(config, before);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        assert!(values.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn existing_large_fonts_and_low_opacity_are_supported() {
        let mut values = SettingsValues::from_config(&UiConfig::default());
        values.font_size = 64.;
        values.opacity = 0.1;
        let mut config = UiConfig::default();
        values.apply(&mut config).unwrap();
        assert_eq!(config.font.size().as_pt(), 64.);
        assert_eq!(config.window_opacity(), 0.1);
    }

    #[test]
    fn themes_replace_complete_palettes_and_preserve_special_colors() {
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            let mut config = UiConfig::default();
            let before = config.clone();
            let mut values = SettingsValues::from_config(&config);
            values.theme = theme;
            values.apply(&mut config).unwrap();
            assert_ne!(config.colors.primary, before.colors.primary);
            assert_ne!(config.colors.normal, before.colors.normal);
            assert_ne!(config.colors.bright, before.colors.bright);
            assert_eq!(config.colors.cursor, before.colors.cursor);
            assert_eq!(config.colors.selection, before.colors.selection);
            assert_eq!(config.colors.hints, before.colors.hints);
            assert_eq!(config.colors.search, before.colors.search);
            assert_eq!(config.colors.dim, before.colors.dim);
            assert_eq!(values.changes().iter().filter(|(path, _)| path[0] == "colors").count(), 20);
            let background = config.colors.primary.background;
            if theme == ThemeChoice::Dark {
                assert!(background.r < 40 && background.g < 40 && background.b < 40);
            } else {
                assert!(background.r > 230 && background.g > 230 && background.b > 230);
            }
        }
    }

    #[test]
    fn clearing_startup_session_overrides_imported_preferences_after_reload() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        std::fs::write(&path, "[general]\nimport=['preferences.toml']\n").unwrap();
        let mut imported: UiConfig =
            toml::from_str("[general]\nstartup_session='Imported workspace'\n").unwrap();
        let mut values = SettingsValues::from_config(&imported);
        assert_eq!(values.startup_session.as_deref(), Some("Imported workspace"));
        values.startup_session = None;
        values.save(&path).unwrap();
        let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["general"]["startup_session"].as_str(), Some(""));
        assert_eq!(saved["general"]["import"][0].as_str(), Some("preferences.toml"));
        imported.replace(saved).unwrap();
        assert_eq!(imported.general.startup_session.as_deref(), Some(""));
        assert_eq!(SettingsValues::from_config(&imported).startup_session, None);
        values.apply(&mut imported).unwrap();
        assert_eq!(imported.general.startup_session, None);
        imported.general.startup_session = Some("   ".into());
        assert_eq!(SettingsValues::from_config(&imported).startup_session, None);
    }

    #[test]
    fn tab_close_confirmation_saves_and_reloads_without_changing_other_preferences() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        let original = r"# Keep header
[general]
import=['preferences.toml']
language='pt-BR'
restore_session=false
session_autosave=true
startup_session='Workspace'
[font]
normal={family='Before',style='Regular'}
[window]
blur=true
[mouse]
hide_when_typing=true
";
        std::fs::write(&path, original).unwrap();
        let mut expected: UiConfig = toml::from_str(original).unwrap();
        let mut values = SettingsValues::from_config(&expected);
        assert!(values.confirm_tab_close);

        for enabled in [false, true] {
            values.confirm_tab_close = enabled;
            values.save(&path).unwrap();
            let saved = std::fs::read_to_string(&path).unwrap();
            assert!(saved.contains("# Keep header"));
            let document: toml::Value = toml::from_str(&saved).unwrap();
            assert_eq!(document["general"]["confirm_tab_close"].as_bool(), Some(enabled));
            assert_eq!(document["general"]["import"][0].as_str(), Some("preferences.toml"));
            let loaded: UiConfig = toml::from_str(&saved).unwrap();
            expected.general.confirm_tab_close = enabled;
            assert_eq!(loaded, expected);
            assert_eq!(SettingsValues::from_config(&loaded).confirm_tab_close, enabled);
        }
    }

    #[test]
    fn saved_values_roundtrip_and_preserve_imports_and_nested_font_style() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alacritty.toml");
        std::fs::write(
            &path,
            r"# Keep header
[general]
import=['theme.toml']
startup_session='Old'
[font]
normal={family='Before',style='Regular'} # Keep style
bold={family='Bold',style='Heavy'}
",
        )
        .unwrap();
        let mut values = SettingsValues::from_config(&UiConfig::default());
        values.font_family = "After".into();
        values.font_size = 17.0;
        values.theme = ThemeChoice::Light;
        values.session_autosave = true;
        values.startup_session = None;
        values.save(&path).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("# Keep header"));
        assert!(text.contains("# Keep style"));
        let document: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(document["general"]["import"][0].as_str(), Some("theme.toml"));
        assert_eq!(document["general"]["startup_session"].as_str(), Some(""));
        assert_eq!(document["font"]["normal"]["family"].as_str(), Some("After"));
        assert_eq!(document["font"]["normal"]["style"].as_str(), Some("Regular"));
        assert_eq!(document["font"]["bold"]["family"].as_str(), Some("Bold"));
        let loaded: UiConfig = toml::from_str(&text).unwrap();
        assert_eq!(loaded.font.size().as_pt(), 17.0);
        assert!(loaded.general.session_autosave);
        assert_eq!(
            loaded.colors.primary.background,
            crate::display::color::Rgb::new(0xf4, 0xf7, 0xfb)
        );
    }
}
