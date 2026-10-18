//! Miscellaneous configuration options.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use alacritty_config_derive::ConfigDeserialize;

use crate::localization::Language;

/// General config section.
///
/// This section is for fields which can not be easily categorized,
/// to avoid common TOML issues with root-level fields.
#[derive(ConfigDeserialize, Serialize, Clone, PartialEq, Debug)]
pub struct General {
    pub language: Language,
    pub shortcuts: BTreeMap<String, String>,
    pub restore_session: bool,
    pub session_autosave: bool,
    pub confirm_tab_close: bool,
    pub startup_session: Option<String>,

    /// Configuration file imports.
    ///
    /// This is never read since the field is directly accessed through the config's
    /// [`toml::Value`], but still present to prevent unused field warnings.
    pub import: Vec<String>,

    /// Shell startup directory.
    pub working_directory: Option<PathBuf>,

    /// Live config reload.
    pub live_config_reload: bool,

    /// Offer IPC through a unix socket.
    #[allow(unused)]
    pub ipc_socket: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: Language::default(),
            shortcuts: BTreeMap::new(),
            restore_session: true,
            session_autosave: false,
            confirm_tab_close: true,
            startup_session: None,
            live_config_reload: true,
            ipc_socket: true,
            working_directory: Default::default(),
            import: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_close_confirmation_defaults_enabled() {
        assert!(General::default().confirm_tab_close);
        let empty: General = toml::from_str("").unwrap();
        assert!(empty.confirm_tab_close);
        let existing: General = toml::from_str("language='en'\nrestore_session=false\n").unwrap();
        assert!(existing.confirm_tab_close);
    }

    #[test]
    fn tab_close_confirmation_parses_and_roundtrips_both_values() {
        for enabled in [false, true] {
            let config: General =
                toml::from_str(&format!("confirm_tab_close={enabled}\n")).unwrap();
            assert_eq!(config.confirm_tab_close, enabled);
            let serialized = toml::to_string(&config).unwrap();
            let reloaded: General = toml::from_str(&serialized).unwrap();
            assert_eq!(reloaded, config);
        }
    }
}
